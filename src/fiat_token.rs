// Copyright 2023 Circle Internet Financial, LTD.  All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

/*!
Fungible Token implementation with JSON serialization.
NOTES:
  - The maximum balance value is limited by U128 (2**128 - 1).
  - JSON calls should pass U128 as a base-10 string. E.g. "100".
  - The contract optimizes the inner trie structure by hashing account IDs. It will prevent some
    abuse of deep tries. Shouldn't be an issue, once NEAR clients implement full hashing of keys.
  - To prevent the deployed contract from being modified or deleted, it should not have any access
    keys on its account.
 */
#![allow(clippy::too_many_arguments)]

use near_contract_standards::fungible_token::{
    core::FungibleTokenCore,
    metadata::{FungibleTokenMetadata, FungibleTokenMetadataProvider},
    resolver::FungibleTokenResolver,
    FungibleToken,
};
use near_contract_standards::storage_management::{
    StorageBalance, StorageBalanceBounds, StorageManagement,
};
use near_sdk::{
    borsh::{self, BorshDeserialize, BorshSerialize},
    collections::LazyOption,
    env,
    json_types::U128,
    log, near_bindgen, require,
    store::UnorderedMap,
    AccountId, Balance, PanicOnDefault, Promise, PromiseOrValue,
};
use near_sdk_contract_tools::{
    approval::ApprovalManagerInternal, upgrade::serialized::UpgradeHook,
};
use near_sdk_contract_tools::{
    approval::{
        simple_multisig::{ApprovalState, Configuration},
        ApprovalManager,
    },
    rbac::Rbac,
    standard::nep297::Event,
    Rbac, SimpleMultisig, Upgrade,
};

use crate::events::fiat_token_event;
use crate::fiat_token_action::FiatTokenAction;
use crate::fiat_token_storage_key::FiatTokenStorageKey;
use crate::requires::{require_not_blocklisted, require_only};
use crate::role::Role;

/// Defines the multi-sig requests/actual behavior of what each [`FiatTokenAction`] will do.
impl near_sdk_contract_tools::approval::Action<Contract> for FiatTokenAction {
    type Output = ();

    fn execute(self, contract: &mut Contract) -> Self::Output {
        match self {
            FiatTokenAction::ApproveForUpgrade => contract.approve_for_upgrade(),
            FiatTokenAction::ConfigureController {
                controller_id,
                minter_id,
            } => {
                contract.configure_controller(controller_id, minter_id);
            }
            FiatTokenAction::ConfigureMinterAllowance {
                minter_allowance, ..
            } => contract.configure_minter_allowance(minter_allowance),
            FiatTokenAction::ConfigureMultisigRole { role, account_id } => {
                contract.configure_multisig_role(role, account_id)
            }
            FiatTokenAction::DecreaseMinterAllowance { decrement, .. } => {
                contract.decrease_minter_allowance(decrement)
            }
            FiatTokenAction::IncreaseMinterAllowance { increment, .. } => {
                contract.increase_minter_allowance(increment)
            }
            FiatTokenAction::Pause => contract.pause(),
            FiatTokenAction::RemoveController { controller_id } => {
                contract.remove_controller(controller_id)
            }
            FiatTokenAction::RemoveMinter { .. } => contract.remove_minter(),
            FiatTokenAction::RevokeMultisigRole { role, account_id } => {
                contract.revoke_multisig_role(role, account_id)
            }
            FiatTokenAction::UpdateBlocklister { new_blocklister_id } => {
                contract.update_blocklister(new_blocklister_id)
            }
            FiatTokenAction::Unpause => contract.unpause(),
        }
    }
}

#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault, Rbac, SimpleMultisig, Upgrade)]
#[simple_multisig(action = "FiatTokenAction", role = "Role::Multisig")]
#[rbac(roles = "Role")]
#[upgrade]
#[near_bindgen]
pub struct Contract {
    token: FungibleToken,
    metadata: LazyOption<FungibleTokenMetadata>,
    allowed: UnorderedMap<AccountId, UnorderedMap<AccountId, U128>>,
    controllers: UnorderedMap<AccountId, AccountId>,
    minter_allowed: UnorderedMap<AccountId, U128>,
    blocklister: AccountId,
    paused: bool,
    approved_for_upgrade: bool,
}

#[near_bindgen]
impl Contract {
    /// Initialize the Contract struct. The `#[init]` decorator also checks if the contract already
    /// exists in the environment.
    #[init]
    pub fn init(
        admin_ids: Vec<AccountId>,
        master_minter_ids: Vec<AccountId>,
        owner_ids: Vec<AccountId>,
        pauser_ids: Vec<AccountId>,
        blocklister_id: AccountId,
        metadata: FungibleTokenMetadata,
    ) -> Self {
        // Configure the multi-sig settings: An approval threshold requiring at least that amount
        // to execute a request, and a validity period requiring that a request cannot be executed,
        // and can be deleted by any approval-eligible member after the period (in nanoseconds)
        // has elapsed. 0 = perpetual validity, no deletion.
        <Self as ApprovalManager<_, _, _>>::init(Configuration::new(
            2,               // Approval threshold.
            432000000000000, // 5 days in nanoseconds.
        ));

        metadata.assert_valid();
        let mut this = Self {
            token: FungibleToken::new(FiatTokenStorageKey::FungibleToken),
            metadata: LazyOption::new(FiatTokenStorageKey::Metadata, Some(&metadata)),
            allowed: UnorderedMap::new(FiatTokenStorageKey::Allowed),
            controllers: UnorderedMap::new(FiatTokenStorageKey::Controllers),
            minter_allowed: UnorderedMap::new(FiatTokenStorageKey::MinterAllowed),
            blocklister: blocklister_id.clone(),
            paused: false,
            approved_for_upgrade: false,
        };

        this.init_multisig_roles(admin_ids, &Role::Admin);
        Rbac::add_role(&mut this, blocklister_id, &Role::Blocklister);
        this.init_multisig_roles(master_minter_ids, &Role::MasterMinter);
        this.init_multisig_roles(owner_ids, &Role::Owner);
        this.init_multisig_roles(pauser_ids, &Role::Pauser);

        this
    }

    /// Setup and grant the main contract roles (Admin, Master Minter, etc.).
    /// Note that the Blocklister is not required to be multi-sig, and so does not have the
    /// Multisig [`Role`].
    fn init_multisig_roles(&mut self, account_ids: Vec<AccountId>, role: &Role) {
        for account_id in account_ids {
            self._grant_multisig_role(account_id.clone(), role);
        }
    }

    /// Returns amount of tokens spender is allowed to transfer on behalf of the token holder.
    /// * `holder_id`   - Token holder's address.
    /// * `spender_id`  - Spender's address.
    pub fn allowance(&self, holder_id: &AccountId, spender_id: &AccountId) -> U128 {
        *self
            .allowed
            .get(holder_id)
            .and_then(|holder_allowance| holder_allowance.get(spender_id))
            .unwrap_or(&U128::from(0))
    }

    /// Gets minter allowance for an account.
    /// * `minter_id`  - The address of the minter.
    pub fn minter_allowance(&self, minter_id: &AccountId) -> U128 {
        *self.minter_allowed.get(minter_id).unwrap_or(&U128::from(0))
    }

    /// Checks if account is a minter.
    /// * `account_id`  - The address to check.
    pub fn is_minter(&self, account_id: &AccountId) -> bool {
        <Contract as Rbac>::has_role(account_id, &Role::Minter)
    }

    /// Configures a controller with the given minter and grants it the Controller [`Role`].
    /// Does not initialize the minter nor does it set the minters allowance.
    /// Only callable by a MasterMinter.
    /// * `controller_id`   - The controller to be configured with a minter.
    /// * `minter_id`       - The minter to be set for the newly configured controller.
    fn configure_controller(&mut self, controller_id: AccountId, minter_id: AccountId) {
        require_only(Role::MasterMinter);
        require_not_blocklisted(&controller_id);
        require_not_blocklisted(&minter_id);
        self._grant_multisig_role(controller_id.clone(), &Role::Controller);

        self.controllers
            .insert(controller_id.clone(), minter_id.clone());
        fiat_token_event::ControllerConfigured {
            controller_id,
            minter_id,
        }
        .emit();
    }

    /// Disables the controller by revoking its Controller [`Role`] and removing its minter.
    /// Only callable by a MasterMinter.
    /// * `controller_id`   - The controller to be removed from the minter.
    fn remove_controller(&mut self, controller_id: AccountId) {
        require_only(Role::MasterMinter);
        self._revoke_multisig_role(&controller_id, &Role::Controller);

        if self.controllers.contains_key(&controller_id) {
            self.controllers.remove(&controller_id);
            fiat_token_event::ControllerRemoved { controller_id }.emit();
        } else {
            env::panic_str("FiatToken: controller does not exist");
        }
    }

    /// Enables/initializes a minter and sets its allowance.
    /// This function can only be called by a controller controlling a minter.
    /// * `minter_allowance`   - Minter's allowance limit.
    fn configure_minter_allowance(&mut self, minter_allowance: U128) {
        let minter_id: AccountId = self.get_minter().clone();
        Rbac::add_role(self, minter_id.clone(), &Role::Minter);
        self.minter_allowed
            .insert(minter_id.clone(), minter_allowance);
        require_not_blocklisted(&minter_id);
        fiat_token_event::MinterConfigured {
            minter_id,
            minter_allowance,
        }
        .emit();
    }

    /// Disables a controller's minter. This function can only be called by a controller.
    fn remove_minter(&mut self) {
        let minter_id: AccountId = self.get_minter().clone();
        Rbac::remove_role(self, &minter_id, &Role::Minter);
        self.minter_allowed.remove(&minter_id);
        fiat_token_event::MinterRemoved { minter_id }.emit();
    }

    /// Increases a controller's minter's allowance if and only if the minter is an active minter.
    /// * `increment`   - Amount of increase in minter allowance.
    fn increase_minter_allowance(&mut self, increment: U128) {
        let minter_id: &AccountId = self.get_minter();
        require_not_blocklisted(minter_id);
        require!(
            increment.0 > 0,
            "FiatToken: minter allowance increment must be greater than 0"
        );
        let new_allowance: u128 = self
            .minter_allowance(minter_id)
            .0
            .checked_add(increment.0)
            .unwrap_or_else(|| env::panic_str("FiatToken: attempted to overflow minter allowance"));
        self.configure_minter_allowance(U128::from(new_allowance));
    }

    /// Decreases a controller's minter's allowance if and only if the minter is an active minter.
    /// * `decrement`   - Amount of decrease in minter allowance.
    fn decrease_minter_allowance(&mut self, decrement: U128) {
        let minter_id: &AccountId = self.get_minter();
        require_not_blocklisted(minter_id);
        require!(
            decrement.0 > 0,
            "FiatToken: minter allowance decrement must be greater than 0"
        );
        let new_allowance: u128 = self
            .minter_allowance(minter_id)
            .0
            .checked_sub(decrement.0)
            .unwrap_or_else(|| {
                env::panic_str("FiatToken: attempted to underflow minter allowance")
            });
        self.configure_minter_allowance(U128::from(new_allowance));
    }

    /// Sets the spender_id's allowance over the caller (the holder of the tokens being
    /// approved to be spent) to be a given value.
    /// * `spender_id`  - Spender's address.
    /// * `value`       - Allowance amount.
    pub fn approve(&mut self, spender_id: AccountId, value: U128) {
        let holder_id: AccountId = env::predecessor_account_id();
        self._approve(holder_id, spender_id, value)
    }

    /// Increases the spender_id's allowance by a given increment.
    /// Panics if the allowance exceeds the u128 max value.
    /// * `spender_id`  - Spender's address.
    /// * `increment`   - Amount of increase in allowance.
    pub fn increase_allowance(&mut self, spender_id: AccountId, increment: U128) {
        require!(
            increment.0 > 0,
            "FiatToken: allowance increment must be greater than 0"
        );
        let holder_id: AccountId = env::predecessor_account_id();
        let old_allowance: u128 = self.allowance(&holder_id, &spender_id).0;
        require!(
            old_allowance > 0,
            "FiatToken: must approve initial allowance before incrementing"
        );
        let new_allowance: u128 = old_allowance
            .checked_add(increment.0)
            .unwrap_or_else(|| env::panic_str("FiatToken: attempted to overflow allowance"));
        self._approve(holder_id, spender_id, U128::from(new_allowance));
    }

    /// Decreases the spender_id's allowance by a given decrement.
    /// Panics if the allowance goes below 0.
    /// * `spender_id`  - Spender's address.
    /// * `decrement`   - Amount of decrease in allowance.
    pub fn decrease_allowance(&mut self, spender_id: AccountId, decrement: U128) {
        require!(
            decrement.0 > 0,
            "FiatToken: allowance decrement must be greater than 0"
        );
        let holder_id: AccountId = env::predecessor_account_id();
        let old_allowance: u128 = self.allowance(&holder_id, &spender_id).0;
        require!(
            old_allowance > 0,
            "FiatToken: must approve initial allowance before decrementing"
        );
        let new_allowance: u128 = old_allowance
            .checked_sub(decrement.0)
            .unwrap_or_else(|| env::panic_str("FiatToken: attempted to underflow allowance"));
        self._approve(holder_id, spender_id, U128::from(new_allowance));
    }

    /// Internal function to set allowance.
    /// * `holder_id`   - Holder's address for which the spender can spend tokens.
    /// * `spender_id`  - Spender's address.
    /// * `allowance`   - Allowance amount.
    fn _approve(&mut self, holder_id: AccountId, spender_id: AccountId, allowance: U128) {
        require!(!self.paused, "FiatToken: paused");
        require_not_blocklisted(&holder_id);
        require_not_blocklisted(&spender_id);

        // Set the allowance.
        let inner_allowance_map: UnorderedMap<AccountId, U128> =
            UnorderedMap::new(FiatTokenStorageKey::Allowance {
                holder_id: holder_id.clone(),
                spender_id: spender_id.clone(),
            });
        *self
            .allowed
            .entry(holder_id.clone())
            .or_insert(inner_allowance_map)
            .entry(spender_id.clone())
            .or_insert(allowance) = allowance;

        fiat_token_event::Approve {
            holder_id,
            spender_id,
            allowance,
        }
        .emit();
    }

    /// Transfers tokens by spending allowance.
    /// * `from`    - Payer's address.
    /// * `to`      - Payee's address.
    /// * `value`   - Transfer amount.
    /// * return true if successful.
    pub fn transfer_from(&mut self, from: AccountId, to: AccountId, value: U128) {
        require!(!self.paused, "FiatToken: paused");
        let caller_id: AccountId = env::predecessor_account_id();
        require_not_blocklisted(&caller_id);
        require_not_blocklisted(&from);
        require_not_blocklisted(&to);

        // Calculate what the allowance will be.
        let new_allowance: u128 = self
            .allowance(&from, &caller_id)
            .0
            .checked_sub(value.0)
            .unwrap_or_else(|| env::panic_str("FiatToken: transfer amount exceeds allowance"));

        // Perform the transfer of tokens. This will emit the transfer event.
        self.token.internal_transfer(&from, &to, value.into(), None);

        // Decrease the allowance.
        self._approve(from.clone(), caller_id, U128::from(new_allowance));
    }

    /// Mints tokens via internal_deposit and emits an FtMint event.
    /// Validates that caller is a minter and that neither the caller nor the to account
    /// are blacklisted.
    /// * `to`      - The address that will receive the minted tokens.
    /// * `amount`  -  The amount of tokens to mint. Must be less than or equal
    /// to the minter_allowance of the caller.
    pub fn mint(&mut self, to: AccountId, amount: U128) {
        require!(!self.paused, "FiatToken: paused");
        require_only(Role::Minter);
        let caller_id: AccountId = env::predecessor_account_id();
        require_not_blocklisted(&caller_id);
        require_not_blocklisted(&to);
        require!(amount.0 > 0, "FiatToken: mint amount not greater than 0");

        // Calculate new minter allowance after minting.
        let new_minter_allowance: u128 = self
            .minter_allowed
            .get(&env::predecessor_account_id())
            .unwrap_or(&U128::from(0))
            .0
            .checked_sub(amount.0)
            .unwrap_or_else(|| env::panic_str("FiatToken: mint amount exceeds minter allowance"));

        self.token.internal_deposit(&to, Balance::from(amount));

        // Decrease the minter allowance.
        self.minter_allowed
            .insert(caller_id, U128::from(new_minter_allowance));

        near_contract_standards::fungible_token::events::FtMint {
            owner_id: &to,
            amount: &amount,
            memo: None,
        }
        .emit();
    }

    /// Burns tokens via internal_withdraw and emits an FtBurn event.
    /// Validates that caller is a minter.
    /// * `amount`  - The amount of tokens to burn. Must be less than or equal
    /// to the minter's account balance.
    pub fn burn(&mut self, amount: U128) {
        require!(!self.paused, "FiatToken: paused");
        require_only(Role::Minter);
        let caller_id: AccountId = env::predecessor_account_id();
        require_not_blocklisted(&caller_id);
        require!(amount.0 > 0, "FiatToken: burn amount not greater than 0");

        let minter_balance: U128 = self.ft_balance_of(caller_id.clone());
        require!(
            minter_balance >= amount,
            "FiatToken: burn amount exceeds balance"
        );

        self.token
            .internal_withdraw(&caller_id, Balance::from(amount));

        near_contract_standards::fungible_token::events::FtBurn {
            owner_id: &caller_id,
            amount: &amount,
            memo: None,
        }
        .emit();
    }

    /// Called by the owner to pause; triggers stopped state.
    fn pause(&mut self) {
        require_only(Role::Pauser);
        require!(!self.paused, "FiatToken: paused");
        require_not_blocklisted(&env::predecessor_account_id());
        self.paused = true;
        fiat_token_event::Paused.emit();
    }

    /// Called by the owner to unpause; returns to normal state.
    fn unpause(&mut self) {
        require_only(Role::Pauser);
        require!(self.paused, "FiatToken: not paused");
        require_not_blocklisted(&env::predecessor_account_id());
        self.paused = false;
        fiat_token_event::Unpaused.emit();
    }

    /// Adds an account to the blocklist.
    /// * `account_id`  - The account to block.
    pub fn blocklist(&mut self, account_id: AccountId) {
        require_only(Role::Blocklister);
        Rbac::add_role(self, account_id.clone(), &Role::Blocklisted);
        fiat_token_event::Blocklist { account_id }.emit();
    }

    /// Removes an account from the blocklist.
    /// * `account_id`  - The account to unblock.
    pub fn unblocklist(&mut self, account_id: AccountId) {
        require_only(Role::Blocklister);
        Rbac::remove_role(self, &account_id, &Role::Blocklisted);
        fiat_token_event::Unblocklist { account_id }.emit();
    }

    /// Retrieves the current admins of this contract.
    pub fn admins(&self) -> Vec<AccountId> {
        <Contract as Rbac>::iter_members_of(&Role::Admin).collect()
    }

    /// Retrieves the current blocklister.
    pub fn blocklister(&self) -> AccountId {
        self.blocklister.clone()
    }

    /// Retrieves the current master minters.
    pub fn master_minters(&self) -> Vec<AccountId> {
        <Contract as Rbac>::iter_members_of(&Role::MasterMinter).collect()
    }

    /// Retrieves the current owners of this contract.
    pub fn owners(&self) -> Vec<AccountId> {
        <Contract as Rbac>::iter_members_of(&Role::Owner).collect()
    }

    /// Retrieves the current pausers.
    pub fn pausers(&self) -> Vec<AccountId> {
        <Contract as Rbac>::iter_members_of(&Role::Pauser).collect()
    }

    /// Returns whether or not a specific account_id is blocklisted.
    pub fn is_blocklisted(&self, account_id: AccountId) -> bool {
        <Contract as Rbac>::has_role(&account_id, &Role::Blocklisted)
    }

    /// Retrieves the minter mapped to the controller.
    /// Panics if the controller does not control a minter.
    pub fn get_minter(&self) -> &AccountId {
        let controller_id: &AccountId = &env::predecessor_account_id();
        require_only(Role::Controller);
        require_not_blocklisted(controller_id);

        self.controllers
            .get(controller_id)
            .unwrap_or_else(|| env::panic_str("FiatToken: caller does not control a minter"))
    }

    /// Configures a new multi-sig contract role, e.g. Admin, Master Minter, etc.
    /// * `role`        - The contract role to configure the account for.
    /// * `account_id`  - The account for which to grant the roles to.
    fn configure_multisig_role(&mut self, role: Role, account_id: AccountId) {
        match role {
            Role::Admin => require_only(Role::Admin),
            Role::MasterMinter | Role::Owner | Role::Pauser => require_only(Role::Owner),
            _ => env::panic_str("FiatToken: cannot grant the specified role"),
        };
        self._grant_multisig_role(account_id.clone(), &role);

        fiat_token_event::RoleConfigured { role, account_id }.emit()
    }

    /// Revokes a multi-sig contract role from an account.
    /// * `role`        - The contract role to revoke from the account.
    /// * `account_id`  - The account for which to revoke the roles from.
    fn revoke_multisig_role(&mut self, role: Role, account_id: AccountId) {
        match role {
            Role::Admin => require_only(Role::Admin),
            Role::MasterMinter | Role::Owner | Role::Pauser => require_only(Role::Owner),
            _ => env::panic_str("FiatToken: cannot revoke the specified role"),
        };
        self._revoke_multisig_role(&account_id, &role);

        fiat_token_event::RoleRevoked { role, account_id }.emit()
    }

    /// Changes the blocklister to a different account.
    /// * `new_blocklister_id`  - The account to make the new blocklister.
    fn update_blocklister(&mut self, new_blocklister_id: AccountId) {
        require_only(Role::Owner);
        let old_blocklister: AccountId = self.blocklister.clone();
        Rbac::remove_role(self, &old_blocklister, &Role::Blocklister);
        Rbac::add_role(self, new_blocklister_id.clone(), &Role::Blocklister);
        self.blocklister = new_blocklister_id.clone();
        fiat_token_event::BlocklisterChanged { new_blocklister_id }.emit();
    }

    /// Approves the contract to be upgraded. While upgrading the contract,
    /// [`approved_for_upgrade`] should be and will be set to false in the [`migrate`] function.
    fn approve_for_upgrade(&mut self) {
        require_only(Role::Admin);
        require_not_blocklisted(&env::predecessor_account_id());
        self.approved_for_upgrade = true;
        fiat_token_event::ApprovedForUpgrade.emit();
    }

    /// Helper function for the [`FungibleTokenResolver`] implementation required by
    /// NEP-141.
    fn on_tokens_burned(&mut self, account_id: AccountId, amount: Balance) {
        near_contract_standards::fungible_token::events::FtBurn {
            owner_id: &account_id,
            amount: &U128::from(amount),
            memo: Some("Burned token supply!"),
        }
        .emit();
        log!("Account @{} burned {}", account_id, amount);
    }

    /// Retrives the next request ID for the next multisig request created.
    pub fn get_next_multisig_request_id(&self) -> u32 {
        <Self as ApprovalManagerInternal<_, _, _>>::slot_next_request_id()
            .read()
            .unwrap_or(0)
    }

    /// Creates a multi-signature request for a [`FiatTokenAction`] that must be approved by at
    /// least the ([`ApprovalManager`]'s) configured [`threshold`] and executed within the
    /// configured [`validity_period_nanoseconds`] amount of time.
    /// Only an account that has been granted the Multisig [`Role`] and the Role specified
    /// by the action can successfully create a request.
    /// * `action`  - The action that this request will execute when it has been fully approved.
    /// The type of the action must conform to the defined [`FiatTokenAction`],
    /// otherwise a deserialization error will be thrown.
    /// Returns the request ID.
    pub fn create_multisig_request(&mut self, action: FiatTokenAction) -> u32 {
        require_only(action.role_required());
        let request_id =
            ApprovalManager::create_request(self, action, ApprovalState::new()).unwrap();
        fiat_token_event::MultisigRequestCreated { request_id }.emit();
        request_id
    }

    /// Approves a multi-signature request. Must be called by an account with the Multisig [`Role`]
    /// and the [`Role`] specified by the action/request.
    /// * `request_id`  - ID of the request to approve.
    pub fn approve_multisig_request(&mut self, request_id: u32) {
        let request = <Contract as ApprovalManager<_, _, _>>::get_request(request_id).unwrap();
        require_only(request.action.role_required());
        if request.action.requires_controller_check() {
            require!(
                self.controllers.get(request.action.controller()).unwrap()
                    == self
                        .controllers
                        .get(&env::predecessor_account_id())
                        .unwrap(),
                "FiatToken: can only approve requests to configure the allowance of your own minter"
            );
        }
        ApprovalManager::approve_request(self, request_id).unwrap();
    }

    /// Executes a multi-signature request, performing pre-defined behavior.
    /// Must be called by an account with the Multisig [`Role`] *and* the [`Role`] specified by the
    /// action/request, and will only work if the request has had sufficient approvals.
    /// * `request_id`  - ID of the request to execute.
    pub fn execute_multisig_request(&mut self, request_id: u32) {
        let request = <Contract as ApprovalManager<_, _, _>>::get_request(request_id).unwrap();
        require_only(request.action.role_required());
        if request.action.requires_controller_check() {
            require!(
                self.controllers.get(request.action.controller()).unwrap()
                    == self
                        .controllers
                        .get(&env::predecessor_account_id())
                        .unwrap(),
                "FiatToken: can only execute requests to configure the allowance of your own minter"
            );
        }
        ApprovalManager::execute_request(self, request_id).unwrap()
    }

    /// Removes a multi-signature request. Must be called by an account with the Multisig [`Role`]
    /// and the [`Role`] specified by the action/request.
    /// * `request_id`  - ID of the request to remove.
    pub fn remove_multisig_request(&mut self, request_id: u32) {
        let request = <Contract as ApprovalManager<_, _, _>>::get_request(request_id).unwrap();
        require_only(request.action.role_required());
        if request.action.requires_controller_check() {
            require!(
                self.controllers.get(request.action.controller()).unwrap()
                    == self
                        .controllers
                        .get(&env::predecessor_account_id())
                        .unwrap(),
                "FiatToken: can only remove requests to configure the allowance of your own minter"
            );
        }
        ApprovalManager::remove_request(self, request_id).unwrap()
    }

    /// Private function to grant the Multisig [`Role`] and a specified [`Role`] to an account.
    /// Function will not panic if an account is granted a Role it already has.
    /// Must only be called by a contract admin account.
    /// * `account_id`  - ID of the account to grant the roles.
    /// * `role`        - Pre-defined [`Role`] to grant to the account.
    fn _grant_multisig_role(&mut self, account_id: AccountId, role: &Role) {
        Rbac::add_role(self, account_id.clone(), &Role::Multisig);
        Rbac::add_role(self, account_id, role);
    }

    /// Private function to revoke the Multisig [`Role`] and the specified [`Role`] from an account.
    /// Must only be called by a contract admin account.
    /// * `account_id`  - ID of the account to revoke the roles from.
    /// * `role`        - Pre-defined [`Role`] to revoke from the account.
    fn _revoke_multisig_role(&mut self, account_id: &AccountId, role: &Role) {
        Rbac::remove_role(self, account_id, &Role::Multisig);
        Rbac::remove_role(self, account_id, role);
    }

    /// Should only be called by this contract on migration.
    /// This method is called from the upgrade() method via the UpgradeHook
    /// https://github.com/NEARFoundation/near-sdk-contract-tools/blob/2bc7afbcc3d66962bfda5a957b5444d855edd228/src/upgrade/serialized.rs#L13
    /// https://github.com/NEARFoundation/near-sdk-contract-tools/blob/2bc7afbcc3d66962bfda5a957b5444d855edd228/src/upgrade/mod.rs#L33
    /// For next version upgrades, change this function, and remember to set
    /// [`approved_for_upgrade`] to false.
    /// *Note*: when adding a new Role, make sure to add it to the bottom of the enum list,
    /// otherwise the migration of Roles and the associated accounts will mess up. At the time of
    /// writing (4/10/2023) this is still an issue.
    #[init(ignore_state)]
    #[private]
    pub fn migrate() -> Self {
        env::log_str("Deserializing current contract...");
        #[derive(BorshDeserialize, BorshSerialize)]
        struct PrevContract {
            token: FungibleToken,
            metadata: LazyOption<FungibleTokenMetadata>,
            allowed: UnorderedMap<AccountId, UnorderedMap<AccountId, U128>>,
            controllers: UnorderedMap<AccountId, AccountId>,
            minter_allowed: UnorderedMap<AccountId, U128>,
            blocklister: AccountId,
            paused: bool,
            approved_for_upgrade: bool,
        }

        let prev: PrevContract = env::state_read().expect("Contract should be initialized");
        env::log_str("Upgrading contract...");

        let mut upgraded_contract: Contract = Self {
            token: prev.token,
            metadata: prev.metadata,
            allowed: prev.allowed,
            controllers: prev.controllers,
            minter_allowed: prev.minter_allowed,
            blocklister: prev.blocklister.clone(),
            paused: prev.paused,
            approved_for_upgrade: false, // Need to reset to false.
        };

        // Re-name token from USD Coin to USDC.
        let mut new_metadata = upgraded_contract.metadata.get().unwrap();
        new_metadata.name = "USDC".to_string();
        upgraded_contract.metadata.set(&new_metadata);

        upgraded_contract
    }
}

/// The #[upgrade] macro exposes an `upgrade(code: Vec<u8>)` function that, when called, will
/// first automatically call this UpgradeHook, allowing permission controls.
impl UpgradeHook for Contract {
    fn on_upgrade(&self) {
        require_only(Role::Admin);
        require_not_blocklisted(&env::predecessor_account_id());
        require!(
            self.approved_for_upgrade,
            "FiatToken: not approved for upgrade"
        );
    }
}

#[near_bindgen]
impl FungibleTokenMetadataProvider for Contract {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        self.metadata.get().unwrap()
    }
}

#[near_bindgen]
impl FungibleTokenCore for Contract {
    #[payable]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        require!(!self.paused, "FiatToken: paused");
        require_not_blocklisted(&env::predecessor_account_id());
        require_not_blocklisted(&receiver_id);
        self.token.ft_transfer(receiver_id, amount, memo);
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        require!(!self.paused, "FiatToken: paused");
        require_not_blocklisted(&env::predecessor_account_id());
        require_not_blocklisted(&receiver_id);
        self.token.ft_transfer_call(receiver_id, amount, memo, msg)
    }

    fn ft_total_supply(&self) -> U128 {
        require!(!self.paused, "FiatToken: paused");
        self.token.ft_total_supply()
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        require!(!self.paused, "FiatToken: paused");
        self.token.ft_balance_of(account_id)
    }
}

#[near_bindgen]
impl FungibleTokenResolver for Contract {
    #[private]
    fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128 {
        require!(!self.paused, "FiatToken: paused");
        require_not_blocklisted(&sender_id);
        require_not_blocklisted(&receiver_id);
        let (used_amount, burned_amount) =
            self.token
                .internal_ft_resolve_transfer(&sender_id, receiver_id, amount);
        if burned_amount > 0 {
            self.on_tokens_burned(
                AccountId::try_from(sender_id.to_string())
                    .expect("Couldn't validate sender address"),
                burned_amount,
            );
        }
        used_amount.into()
    }
}

#[near_bindgen]
impl StorageManagement for Contract {
    #[payable]
    fn storage_deposit(
        &mut self,
        account_id: Option<AccountId>,
        registration_only: Option<bool>,
    ) -> StorageBalance {
        require!(!self.paused, "FiatToken: paused");
        require_not_blocklisted(&env::predecessor_account_id());
        if let Some(account) = account_id.clone() {
            require_not_blocklisted(&account);
        }
        self.token.storage_deposit(account_id, registration_only)
    }

    #[payable]
    fn storage_withdraw(&mut self, amount: Option<U128>) -> StorageBalance {
        self.token.storage_withdraw(amount)
    }

    #[payable]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        near_sdk::assert_one_yocto();
        require!(!self.paused, "FiatToken: paused");
        require_not_blocklisted(&env::predecessor_account_id());
        let account_id = env::predecessor_account_id();
        let force = force.unwrap_or(false);
        require!(!force, "FiatToken: cannot force storage unregister");
        if let Some(balance) = self.token.accounts.get(&account_id) {
            require!(
                balance == 0,
                "FiatToken: cannot unregister an account with a positive balance"
            );
            self.token.accounts.remove(&account_id);
            Promise::new(account_id.clone()).transfer(self.storage_balance_bounds().min.0 + 1);
            true
        } else {
            log!("The account {} is not registered", &account_id);
            false
        }
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.token.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.token.storage_balance_of(account_id)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use near_contract_standards::fungible_token::metadata::FT_METADATA_SPEC;
    use near_sdk::json_types::U128;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::{test_utils, testing_env, AccountId, Balance, ONE_YOCTO};

    use super::*;

    const USDC_CONTRACT_DECIMALS: u8 = 6;
    const USDC_CONTRACT_SYMBOL: &str = "USDC";

    fn get_context(predecessor_account_id: AccountId) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder
            .current_account_id(accounts(0))
            .signer_account_id(predecessor_account_id.clone())
            .predecessor_account_id(predecessor_account_id);
        builder
    }

    fn get_metadata() -> FungibleTokenMetadata {
        FungibleTokenMetadata {
            spec: FT_METADATA_SPEC.to_string(),
            name: USDC_CONTRACT_SYMBOL.to_string(),
            symbol: USDC_CONTRACT_SYMBOL.to_string(),
            decimals: USDC_CONTRACT_DECIMALS,
            icon: None,
            reference: None,
            reference_hash: None,
        }
    }

    fn admin() -> AccountId {
        "admin".parse().unwrap()
    }

    fn owner() -> AccountId {
        "owner".parse().unwrap()
    }

    fn master_minter() -> AccountId {
        "masterminter".parse().unwrap()
    }

    fn pauser() -> AccountId {
        "pauser".parse().unwrap()
    }

    fn blocklister() -> AccountId {
        "blocklister".parse().unwrap()
    }

    fn controller() -> AccountId {
        "controller".parse().unwrap()
    }

    fn minter() -> AccountId {
        "minter".parse().unwrap()
    }

    fn init_contract() -> Contract {
        let admins = vec![admin()];
        let master_minters = vec![master_minter()];
        let owners = vec![owner()];
        let pausers = vec![pauser()];

        let mut usdc: Contract = Contract::init(
            admins,
            master_minters,
            owners,
            pausers,
            blocklister(),
            FungibleTokenMetadata::from(get_metadata()),
        );

        // By default, configure a controller to control a minter, and configure the minter's allowance to be the max U128 value.
        set_caller(master_minter());
        usdc.configure_controller(controller(), minter());

        set_caller(controller());
        usdc.configure_minter_allowance(U128::from(u128::MAX));

        usdc
    }

    /// Registers an account with the contract via storage_deposit and mints init_amount of initial supply to it.
    fn init_account(contract: &mut Contract, account_id: AccountId, init_amount: Option<U128>) {
        let account_storage_deposit: u128 = contract.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        contract.storage_deposit(Some(account_id.clone()), Some(false));
        set_caller(minter());
        if init_amount.is_some() {
            contract.mint(account_id, init_amount.unwrap().into());
        }
    }

    // Helper function to blocklist within the testing env context.
    fn _blocklist(contract: &mut Contract, account_id: AccountId) {
        set_caller(blocklister());
        contract.blocklist(account_id);
    }

    // Helper function to set the caller of the current testing env context.
    fn set_caller(caller_id: AccountId) -> VMContextBuilder {
        let context: VMContextBuilder = get_context(caller_id);
        testing_env!(context.build());
        context
    }

    #[test]
    fn test_init() {
        let usdc: Contract = init_contract();
        assert_eq!(usdc.ft_total_supply().0, 0);
        assert_eq!(usdc.ft_balance_of(owner()).0, 0);
        assert_eq!(usdc.ft_balance_of(accounts(0).into()).0, 0);

        assert_eq!(usdc.admins().get(0).unwrap(), &admin());
        assert_eq!(usdc.master_minters().get(0).unwrap(), &master_minter());
        assert_eq!(usdc.owners().get(0).unwrap(), &owner());
        assert_eq!(usdc.pausers().get(0).unwrap(), &pauser());

        let metadata: FungibleTokenMetadata = usdc.ft_metadata();
        assert_eq!(metadata.name, "USDC".to_string());
        assert_eq!(metadata.symbol, "USDC".to_string());
        assert_eq!(metadata.decimals, 6);
    }

    #[test]
    fn test_allowance() {
        let usdc: Contract = init_contract();
        assert_eq!(usdc.allowance(&accounts(1), &accounts(0)), U128::from(0));
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller does not control a minter")]
    fn test_get_minter_caller_does_not_control_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let minterless_controller: AccountId = "ml_controller".parse().unwrap();
        set_caller(master_minter());
        usdc._grant_multisig_role(minterless_controller.clone(), &Role::Controller);
        set_caller(minterless_controller);

        // Act.
        usdc.get_minter();
    }

    #[test]
    fn test_get_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(master_minter());
        usdc.configure_controller(controller(), minter());
        set_caller(controller());

        // Act.
        assert_eq!(usdc.get_minter(), &minter());
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a MasterMinter")]
    fn test_configure_controller_not_master_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        usdc.configure_controller(controller(), minter());
    }

    #[test]
    fn test_configure_controller() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let controller: AccountId = { "controller1".parse().unwrap() };
        let minter: AccountId = { "minter1".parse().unwrap() };
        set_caller(master_minter());

        // Act.
        usdc.configure_controller(controller.clone(), minter.clone());

        // Assert.
        // Configuring a controller just assigns a minter to the controller. It does not initialize
        // the minter nor does it set its minter allowance.
        assert_eq!(usdc.is_minter(&minter), false);
        assert_eq!(usdc.minter_allowance(&minter), U128::from(0));
        assert_eq!(
            test_utils::get_logs()[0],
            format!(
                "EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"controller_configured\",\"data\":{{\"controller_id\":\"{}\",\"minter_id\":\"{}\"}}}}",
                controller.to_string(),
                minter.to_string(),
            )
        );
    }

    #[test]
    fn test_configure_controller_update_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let controller: AccountId = { "controller1".parse().unwrap() };
        let minter: AccountId = { "minter1".parse().unwrap() };
        let minter2: AccountId = { "minter2".parse().unwrap() };
        set_caller(master_minter());

        // Act.
        usdc.configure_controller(controller.clone(), minter.clone());
        usdc.configure_controller(controller.clone(), minter2.clone());

        // Assert.
        assert_eq!(
            test_utils::get_logs()[0],
            format!(
                "EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"controller_configured\",\"data\":{{\"controller_id\":\"{}\",\"minter_id\":\"{}\"}}}}",
                controller.to_string(),
                minter.to_string(),
            )
        );
        assert_eq!(
            test_utils::get_logs()[1],
            format!(
                "EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"controller_configured\",\"data\":{{\"controller_id\":\"{}\",\"minter_id\":\"{}\"}}}}",
                controller.to_string(),
                minter2.to_string(),
            )
        );
    }

    #[test]
    fn test_remove_controller() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(master_minter());
        let controller2: AccountId = "controller2".parse().unwrap();
        usdc.configure_controller(controller2.clone(), minter());

        // Act.
        usdc.remove_controller(controller2.clone());

        // Assert.
        assert!(!usdc.controllers.contains_key(&controller2));
        assert_eq!(
            test_utils::get_logs()[1],
            format!(
                "EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"controller_removed\",\"data\":{{\"controller_id\":\"{}\"}}}}",
                controller2.to_string(),
            )
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a MasterMinter")]
    fn test_remove_controller_not_master_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        usdc.remove_controller(controller());
    }

    #[test]
    #[should_panic(expected = "FiatToken: controller does not exist")]
    fn test_remove_controller_not_found() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(master_minter());
        let controller2: AccountId = "controller2".parse().unwrap();
        usdc.configure_controller(controller2, minter());
        let controller3: AccountId = "controller3".parse().unwrap();
        usdc.configure_controller(controller3, minter());

        // Act.
        usdc.remove_controller(minter());
    }

    #[test]
    fn test_configure_minter_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(controller());

        // Act.
        usdc.configure_minter_allowance(U128::from(12345));

        // Assert.
        assert_eq!(usdc.is_minter(&minter()), true,);
        assert_eq!(usdc.minter_allowance(&minter()), U128::from(12345),);
        assert_eq!(
            test_utils::get_logs()[0],
            format!(
                "EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"minter_configured\",\"data\":{{\"minter_id\":\"{}\",\"minter_allowance\":{}}}}}",
                minter().to_string(),
                &near_sdk::serde_json::to_string(&U128::from(12345)).unwrap(),
            )
        );
    }

    #[test]
    fn test_remove_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(controller());

        // Act.
        usdc.configure_minter_allowance(U128::from(12345));
        usdc.remove_minter();

        // Assert.
        assert_eq!(usdc.is_minter(&minter()), false,);
        assert_eq!(usdc.minter_allowance(&minter()), U128::from(0),);
        assert_eq!(
            test_utils::get_logs()[1],
            format!(
                "EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"minter_removed\",\"data\":{{\"minter_id\":\"{}\"}}}}",
                minter().to_string(),
            )
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: minter allowance increment must be greater than 0")]
    fn test_increase_minter_allowance_bad_increment() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(controller());

        // Act.
        usdc.configure_minter_allowance(U128::from(u128::MAX));
        usdc.increase_minter_allowance(U128::from(0));
    }

    #[test]
    #[should_panic(expected = "FiatToken: attempted to overflow minter allowance")]
    fn test_increase_minter_allowance_overflow() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(controller());

        // Act.
        usdc.configure_minter_allowance(U128::from(u128::MAX));
        usdc.increase_minter_allowance(U128::from(1));
    }

    #[test]
    fn test_increase_minter_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(controller());

        // Act.
        usdc.configure_minter_allowance(U128::from(123));
        usdc.increase_minter_allowance(U128::from(10));

        // Assert.
        assert_eq!(usdc.minter_allowance(&minter()), U128::from(133));
        assert_eq!(
            test_utils::get_logs()[1],
            format!(
                "EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"minter_configured\",\"data\":{{\"minter_id\":\"{}\",\"minter_allowance\":{}}}}}",
                minter().to_string(),
                &near_sdk::serde_json::to_string(&U128::from(133)).unwrap(),
            )
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: minter allowance decrement must be greater than 0")]
    fn test_decrease_minter_allowance_bad_decrement() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(controller());

        // Act.
        usdc.configure_minter_allowance(U128::from(u128::MAX));
        usdc.decrease_minter_allowance(U128::from(0));
    }

    #[test]
    #[should_panic(expected = "FiatToken: attempted to underflow minter allowance")]
    fn test_decrease_minter_allowance_underflow() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(controller());

        // Act.
        usdc.configure_minter_allowance(U128::from(1));
        usdc.decrease_minter_allowance(U128::from(2));
    }

    #[test]
    fn test_decrease_minter_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(controller());

        // Act.
        usdc.configure_minter_allowance(U128::from(123));
        usdc.decrease_minter_allowance(U128::from(10));

        // Assert.
        assert_eq!(usdc.minter_allowance(&minter()), U128::from(113));
        assert_eq!(
            test_utils::get_logs()[1],
            format!(
                "EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"minter_configured\",\"data\":{{\"minter_id\":\"{}\",\"minter_allowance\":{}}}}}",
                minter().to_string(),
                &near_sdk::serde_json::to_string(&U128::from(113)).unwrap(),
            )
        );
    }

    #[test]
    fn test_minter_allowance() {
        let usdc: Contract = init_contract();
        assert_eq!(usdc.is_minter(&accounts(1)), false);
        assert_eq!(usdc.minter_allowance(&accounts(1)), U128::from(0));
    }

    #[test]
    fn test_update_master_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(owner());

        // Act.
        usdc.configure_multisig_role(Role::MasterMinter, accounts(2).into());

        // Assert.
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"role_configured\",\"data\":{{\"role\":\"{}\",\"account_id\":\"{}\"}}}}", Role::MasterMinter, accounts(2).to_string())
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Owner")]
    fn test_update_master_minter_not_owner() {
        // Arrange.
        set_caller(accounts(1));

        let mut usdc: Contract = init_contract();

        // Act.
        usdc.configure_multisig_role(Role::MasterMinter, accounts(2));
    }

    #[test]
    fn test_approve() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let allowed_amount: U128 = U128::from(123);
        let holder_id = env::predecessor_account_id();
        let spender_id: AccountId = "spender".parse().unwrap();

        // Act.
        usdc.approve(spender_id.clone(), allowed_amount);

        // Assert.
        assert_eq!(usdc.allowance(&holder_id, &spender_id), allowed_amount);
        assert_eq!(
            test_utils::get_logs()[1],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"approve\",\"data\":{{\"holder_id\":\"{}\",\"spender_id\":\"{}\",\"allowance\":{}}}}}",
                    holder_id.to_string(),
                    spender_id.to_string(),
                    near_sdk::serde_json::to_string(&allowed_amount).unwrap()
            )
        );
    }

    #[test]
    fn test_approve_twice() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let initial_allowed_amount: U128 = U128::from(123);
        let updated_allowed_amount: U128 = U128::from(200);
        let holder_id = &env::predecessor_account_id();
        let spender_id: AccountId = "spender".parse().unwrap();

        // Act.
        usdc.approve(spender_id.clone(), initial_allowed_amount);
        usdc.approve(spender_id.clone(), updated_allowed_amount);

        // Assert.
        assert_eq!(
            usdc.allowance(&holder_id, &spender_id),
            updated_allowed_amount
        );
        assert_eq!(
            test_utils::get_logs()[1],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"approve\",\"data\":{{\"holder_id\":\"{}\",\"spender_id\":\"{}\",\"allowance\":{}}}}}",
                    holder_id.to_string(),
                    spender_id.to_string(),
                    near_sdk::serde_json::to_string(&initial_allowed_amount).unwrap()
            )
        );
        assert_eq!(
            test_utils::get_logs()[2],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"approve\",\"data\":{{\"holder_id\":\"{}\",\"spender_id\":\"{}\",\"allowance\":{}}}}}",
                    holder_id.to_string(),
                    spender_id.to_string(),
                    near_sdk::serde_json::to_string(&updated_allowed_amount).unwrap())
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: allowance increment must be greater than 0")]
    fn test_increase_allowance_increment_less_than_zero() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let allowed_amount: U128 = U128::from(123);
        let increment: U128 = U128::from(0);
        let spender_id: AccountId = "spender".parse().unwrap();
        usdc.approve(spender_id.clone(), allowed_amount);

        // Act.
        usdc.increase_allowance(spender_id.clone(), increment);
    }

    #[test]
    #[should_panic(expected = "FiatToken: must approve initial allowance before incrementing")]
    fn test_increase_allowance_must_approve_first() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let increment: U128 = U128::from(5);
        let spender_id: AccountId = "spender".parse().unwrap();

        // Act.
        usdc.increase_allowance(spender_id.clone(), increment);
    }

    #[test]
    #[should_panic(expected = "FiatToken: attempted to overflow allowance")]
    fn test_increase_allowance_overflow() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let allowed_amount: U128 = U128::from(u128::MAX);
        let increment: U128 = U128::from(1);
        let spender_id: AccountId = "spender".parse().unwrap();
        usdc.approve(spender_id.clone(), allowed_amount);

        // Act.
        usdc.increase_allowance(spender_id.clone(), increment);
    }

    #[test]
    fn test_increase_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let allowed_amount: U128 = U128::from(123);
        let increment: U128 = U128::from(7);
        let final_amount: U128 = U128::from(130);
        let holder_id = env::predecessor_account_id();
        let spender_id: AccountId = "spender".parse().unwrap();
        usdc.approve(spender_id.clone(), allowed_amount);

        // Act.
        usdc.increase_allowance(spender_id.clone(), increment);

        // Assert.
        assert_eq!(usdc.allowance(&holder_id, &spender_id), final_amount);
        assert_eq!(
            test_utils::get_logs()[2],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"approve\",\"data\":{{\"holder_id\":\"{}\",\"spender_id\":\"{}\",\"allowance\":{}}}}}",
                    holder_id.to_string(),
                    spender_id.to_string(),
                    near_sdk::serde_json::to_string(&final_amount).unwrap()
            )
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: allowance decrement must be greater than 0")]
    fn test_decrease_allowance_increment_less_than_zero() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let allowed_amount: U128 = U128::from(123);
        let decrement: U128 = U128::from(0);
        let spender_id: AccountId = "spender".parse().unwrap();
        usdc.approve(spender_id.clone(), allowed_amount);

        // Act.
        usdc.decrease_allowance(spender_id.clone(), decrement);
    }

    #[test]
    #[should_panic(expected = "FiatToken: must approve initial allowance before decrementing")]
    fn test_decrease_allowance_must_approve_first() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let decrement: U128 = U128::from(5);
        let spender_id: AccountId = "spender".parse().unwrap();

        // Act.
        usdc.decrease_allowance(spender_id.clone(), decrement);
    }

    #[test]
    #[should_panic(expected = "FiatToken: attempted to underflow allowance")]
    fn test_decrease_allowance_underflow() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let allowed_amount: U128 = U128::from(1);
        let decrement: U128 = U128::from(2);
        let spender_id: AccountId = "spender".parse().unwrap();
        usdc.approve(spender_id.clone(), allowed_amount);

        // Act.
        usdc.decrease_allowance(spender_id.clone(), decrement);
    }

    #[test]
    fn test_decrease_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let allowed_amount: U128 = U128::from(123);
        let decrement: U128 = U128::from(3);
        let final_amount: U128 = U128::from(120);
        let holder_id = env::predecessor_account_id();
        let spender_id: AccountId = "spender".parse().unwrap();
        usdc.approve(spender_id.clone(), allowed_amount);

        // Act.
        usdc.decrease_allowance(spender_id.clone(), decrement);

        // Assert.
        assert_eq!(usdc.allowance(&holder_id, &spender_id), final_amount);
        assert_eq!(
            test_utils::get_logs()[2],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"approve\",\"data\":{{\"holder_id\":\"{}\",\"spender_id\":\"{}\",\"allowance\":{}}}}}",
                    holder_id.to_string(),
                    spender_id.to_string(),
                    near_sdk::serde_json::to_string(&final_amount).unwrap()
            )
        );
    }

    #[test]
    /// See https://doc.rust-lang.org/book/ch11-01-writing-tests.html.
    #[should_panic(expected = "FiatToken: transfer amount exceeds allowance")]
    fn test_transfer_from_not_in_allowed() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let to_account: AccountId = "to".parse().unwrap();
        let amount: U128 = U128::from(51);
        init_account(
            &mut usdc,
            env::predecessor_account_id(),
            Some(U128::from(100)),
        );
        init_account(&mut usdc, to_account.clone(), Some(U128::from(100)));

        // Act.
        usdc.transfer_from(env::predecessor_account_id(), to_account.clone(), amount);

        // Assert.
        // Cannot use result.get_err() .unwrap()as that is still under development: https://github.com/rust-lang/rust/issues/62358
        // assert_eq!(result.get_err(&"t.unwrap()ransfer amount exceeds allowance"), true);
    }

    #[test]
    #[should_panic(expected = "FiatToken: transfer amount exceeds allowance")]
    fn test_transfer_from_insufficent_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let to_account: AccountId = "to".parse().unwrap();
        let allowed_amount: U128 = U128::from(10);
        let amount: U128 = U128::from(51);
        usdc.approve(env::predecessor_account_id(), allowed_amount);
        init_account(
            &mut usdc,
            env::predecessor_account_id(),
            Some(U128::from(100)),
        );
        init_account(&mut usdc, to_account.clone(), Some(U128::from(100)));

        // Act.
        usdc.transfer_from(env::predecessor_account_id(), to_account.clone(), amount);
    }

    #[test]
    fn test_transfer_from() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let holder_account: AccountId = "from".parse().unwrap();
        let spender_account: AccountId = "spender".parse().unwrap();
        let to_account: AccountId = "to".parse().unwrap();
        let allowed_amount: U128 = U128::from(123);
        let amount: U128 = U128::from(51);
        init_account(&mut usdc, holder_account.clone(), Some(U128::from(100)));
        init_account(&mut usdc, spender_account.clone(), Some(U128::from(100)));
        init_account(&mut usdc, to_account.clone(), Some(U128::from(100)));

        // Token holders approve spenders over their tokens.
        set_caller(holder_account.clone());
        usdc.approve(spender_account.clone(), allowed_amount);

        // Act. A spender requests the FiatToken contract to spend the token holder's tokens.
        set_caller(spender_account.clone());
        usdc.transfer_from(holder_account.clone(), to_account.clone(), amount);

        // Assert.
        assert_eq!(
            usdc.allowance(&holder_account, &spender_account),
            U128::from(72) // 123 - 51.
        );
        println!("{:?}", test_utils::get_logs());
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_transfer\",\"data\":[{{\"old_owner_id\":\"{}\",\"new_owner_id\":\"{}\",\"amount\":{}}}]}}", &holder_account, &to_account, near_sdk::serde_json::to_string(&amount).unwrap())
        );
    }

    #[test]
    #[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
    fn test_ft_transfer_requires_deposit() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let sender: AccountId = "from".parse().unwrap();
        let receiver: AccountId = "to".parse().unwrap();
        let amount: U128 = U128::from(50);
        init_account(&mut usdc, sender.clone(), Some(U128::from(100)));
        init_account(&mut usdc, receiver.clone(), Some(U128::from(100)));

        // Act.
        set_caller(sender.clone());
        usdc.ft_transfer(receiver.clone(), amount, None);
    }

    #[test]
    fn test_ft_transfer() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let sender: AccountId = "from".parse().unwrap();
        let receiver: AccountId = "to".parse().unwrap();
        let amount: U128 = U128::from(50);
        init_account(&mut usdc, sender.clone(), Some(U128::from(100)));
        init_account(&mut usdc, receiver.clone(), Some(U128::from(100)));

        // Act.
        let mut context: VMContextBuilder = get_context(sender.clone());
        context.attached_deposit(1);
        testing_env!(context.build());
        usdc.ft_transfer(receiver.clone(), amount, None);

        // Assert.
        assert_eq!(usdc.ft_balance_of(receiver.clone()).0, 150);
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_transfer\",\"data\":[{{\"old_owner_id\":\"{}\",\"new_owner_id\":\"{}\",\"amount\":{}}}]}}",
                    &sender,
                    &receiver,
                    near_sdk::serde_json::to_string(&amount).unwrap()
            )
        );
    }

    #[test]
    fn test_ft_transfer_call() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let sender: AccountId = "from".parse().unwrap();
        let receiver: AccountId = "to".parse().unwrap();
        let amount: U128 = U128::from(50);
        init_account(&mut usdc, sender.clone(), Some(U128::from(100)));
        init_account(&mut usdc, receiver.clone(), Some(U128::from(100)));

        // Act.
        let mut context: VMContextBuilder = get_context(sender.clone());
        context.attached_deposit(1);
        testing_env!(context.build());
        usdc.ft_transfer_call(receiver.clone(), amount, None, "Msg".to_string());

        // Assert.
        assert_eq!(usdc.ft_balance_of(receiver.clone()).0, 150);
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_transfer\",\"data\":[{{\"old_owner_id\":\"{}\",\"new_owner_id\":\"{}\",\"amount\":{}}}]}}",
                    &sender,
                    &receiver,
                    near_sdk::serde_json::to_string(&amount).unwrap()
            )
        );
    }

    #[test]
    fn test_mint() {
        // Arrange.
        set_caller(master_minter());

        let mut usdc: Contract = init_contract();
        let minter: AccountId = "minter".parse().unwrap();
        let to: AccountId = "to_id".parse().unwrap();
        let mint_amount: U128 = U128::from(100);
        let previous_amount: Balance = usdc.ft_total_supply().0;
        usdc.configure_minter_allowance(U128::from(123));
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Some(to.clone()), Some(false));

        set_caller(minter);

        // Act.
        usdc.mint(to.clone(), mint_amount.clone());

        // Assert.
        assert_eq!(usdc.ft_total_supply().0, previous_amount + mint_amount.0);
        assert_eq!(usdc.ft_balance_of(to.clone()).0, mint_amount.0);
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_mint\",\"data\":[{{\"owner_id\":\"{}\",\"amount\":{}}}]}}",
                    &to,
                    near_sdk::serde_json::to_string(&mint_amount).unwrap()
            )
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: mint amount exceeds minter allowance")]
    fn test_mint_exceeds_allowance() {
        // Arrange.
        set_caller(master_minter());

        let mut usdc: Contract = init_contract();
        let minter: AccountId = "minter".parse().unwrap();
        let to: AccountId = "to_id".parse().unwrap();
        let exceeding_mint_amount: U128 = U128::from(200);
        usdc.configure_minter_allowance(U128::from(123));
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Some(to.clone()), Some(false));

        set_caller(minter);

        // Act.
        usdc.mint(to.clone(), exceeding_mint_amount.clone());
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Minter")]
    fn test_mint_caller_not_a_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let to: AccountId = "to_id".parse().unwrap();
        let exceeding_mint_amount: U128 = U128::from(200);
        usdc.configure_minter_allowance(U128::from(123));
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Some(to.clone()), Some(false));
        set_caller(owner());

        // Act.
        usdc.mint(to.clone(), exceeding_mint_amount.clone());
    }

    #[test]
    #[should_panic(expected = "FiatToken: mint amount not greater than 0")]
    fn test_mint_amount_must_be_greater_than_zero() {
        // Arrange.
        set_caller(master_minter());

        let mut usdc: Contract = init_contract();
        let minter: AccountId = "minter".parse().unwrap();
        let to: AccountId = "to_id".parse().unwrap();
        let mint_amount: U128 = U128::from(0);
        usdc.configure_minter_allowance(U128::from(123));
        init_account(&mut usdc, to.clone(), None);

        set_caller(minter);

        // Act.
        usdc.mint(to.clone(), mint_amount.clone());
    }

    #[test]
    #[should_panic(expected = "FiatToken: mint amount exceeds minter allowance")]
    fn test_mint_twice_exceeds_allowance() {
        // Arrange.
        set_caller(master_minter());

        let mut usdc: Contract = init_contract();
        let minter: AccountId = "minter".parse().unwrap();
        let to: AccountId = "to_id".parse().unwrap();
        let mint_amount: U128 = U128::from(100);
        usdc.configure_minter_allowance(U128::from(123));
        init_account(&mut usdc, to.clone(), None);

        set_caller(minter);

        // Act.
        usdc.mint(to.clone(), mint_amount.clone());
        usdc.mint(to.clone(), mint_amount.clone());
    }

    #[test]
    fn test_burn() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let burner_controller: AccountId = "burnercontroller".parse().unwrap();
        let burner: AccountId = "burner".parse().unwrap();
        let initial_burner_balance: U128 = U128::from(50);
        // Setup the burner to have some amount of tokens. In the real world, to burn tokens from a
        // generic account, we have to first send those tokens to a burner before proceeding.
        init_account(&mut usdc, burner.clone(), Some(initial_burner_balance));
        let balance_prior_to_burn: Balance = usdc.ft_total_supply().0;
        set_caller(master_minter());
        usdc.configure_controller(burner_controller.clone(), burner.clone());

        // Configure the burner to be a minter.
        set_caller(burner_controller);
        usdc.configure_minter_allowance(initial_burner_balance);

        // Act.
        let burn_amount: U128 = U128::from(49);
        set_caller(burner.clone());
        usdc.burn(burn_amount.clone());

        // Assert.
        assert_eq!(
            usdc.ft_total_supply().0,
            balance_prior_to_burn - burn_amount.clone().0
        );
        assert_eq!(
            usdc.ft_balance_of(burner.clone()).0,
            initial_burner_balance.0 - burn_amount.0
        );
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_burn\",\"data\":[{{\"owner_id\":\"{}\",\"amount\":{}}}]}}",
                    &burner,
                    near_sdk::serde_json::to_string(&burn_amount).unwrap()
            )
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: burn amount not greater than 0")]
    fn test_burn_amount_must_be_greater_than_zero() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let burner_controller: AccountId = "burnercontroller".parse().unwrap();
        let burner: AccountId = "burner".parse().unwrap();
        let initial_burner_balance: U128 = U128::from(50);
        // Setup the burner to have some amount of tokens. In the real world, to burn tokens from a
        // generic account, we have to first send those tokens to a burner before proceeding.
        init_account(&mut usdc, burner.clone(), Some(initial_burner_balance));
        set_caller(master_minter());
        usdc.configure_controller(burner_controller.clone(), burner.clone());

        // Configure the burner to be a minter.
        set_caller(burner_controller);
        usdc.configure_minter_allowance(initial_burner_balance);

        // Act.
        set_caller(burner);
        usdc.burn(U128::from(0));
    }

    #[test]
    #[should_panic(expected = "FiatToken: burn amount exceeds balance")]
    fn test_burn_amount_exceeds_burner_balance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let burner_controller: AccountId = "burnercontroller".parse().unwrap();
        let burner: AccountId = "burner".parse().unwrap();
        let initial_burner_balance: U128 = U128::from(50);
        // Setup the burner to have some amount of tokens. In the real world, to burn tokens from a
        // generic account, we have to first send those tokens to a burner before proceeding.
        init_account(&mut usdc, burner.clone(), Some(initial_burner_balance));
        set_caller(master_minter());
        usdc.configure_controller(burner_controller.clone(), burner.clone());

        // Configure the burner to be a minter.
        set_caller(burner_controller);
        usdc.configure_minter_allowance(initial_burner_balance);

        // Act.
        set_caller(burner);
        usdc.burn(U128::from(51));
    }

    #[test]
    #[should_panic(expected = "FiatToken: burn amount exceeds balance")]
    fn test_burn_twice_exceeds_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let burner_controller: AccountId = "burnercontroller".parse().unwrap();
        let burner: AccountId = "burner".parse().unwrap();
        let initial_burner_balance: U128 = U128::from(50);
        // Setup the burner to have some amount of tokens. In the real world, to burn tokens from a
        // generic account, we have to first send those tokens to a burner before proceeding.
        init_account(&mut usdc, burner.clone(), Some(initial_burner_balance));
        set_caller(master_minter());
        usdc.configure_controller(burner_controller.clone(), burner.clone());

        // Configure the burner to be a minter.
        set_caller(burner_controller);
        usdc.configure_minter_allowance(initial_burner_balance);

        // Act.
        set_caller(burner);
        usdc.burn(U128::from(50));
        usdc.burn(U128::from(1));
    }

    #[test]
    #[should_panic(expected = "FiatToken: cannot revoke the specified role")]
    fn test_revoke_multisig_role_caller_invalid_role() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        usdc.revoke_multisig_role(Role::Minter, admin());
    }

    #[test]
    fn test_revoke_multisig_role_can_revoke_accounts_without_role() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(admin());

        // Act.
        usdc.revoke_multisig_role(Role::Admin, admin());
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"role_revoked\",\"data\":{{\"role\":\"Admin\",\"account_id\":\"{}\"}}}}", admin())
        );

        set_caller(owner());
        // Revoking a Pauser who does not have the role of Owner is ok. It will just be a no-op.
        usdc.revoke_multisig_role(Role::Owner, pauser());
        usdc.revoke_multisig_role(Role::Owner, owner());
        assert_eq!(
            test_utils::get_logs()[1],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"role_revoked\",\"data\":{{\"role\":\"Owner\",\"account_id\":\"{}\"}}}}", owner())

        );
    }

    #[test]
    fn test_pause() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(pauser());

        // Act.
        usdc.pause();

        // Assert.
        assert_eq!(
            test_utils::get_logs()[0],
            "EVENT_JSON:{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"paused\",\"data\":null}"
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Pauser")]
    fn test_pause_not_pauser() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(accounts(1));

        // Act.
        usdc.pause();
    }

    #[test]
    #[should_panic(expected = "FiatToken: paused")]
    fn test_pause_already_paused() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(pauser());

        // Act.
        usdc.pause();
        usdc.pause();
    }

    #[test]
    fn test_unpause() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(pauser());

        // Act.
        usdc.pause();
        usdc.unpause();

        // Assert.
        assert_eq!(
            test_utils::get_logs()[1],
            "EVENT_JSON:{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"unpaused\",\"data\":null}"
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Pauser")]
    fn test_unpause_not_pauser() {
        // Arrange.
        set_caller(accounts(1));

        let mut usdc: Contract = init_contract();

        // Act.
        usdc.unpause();
    }

    #[test]
    #[should_panic(expected = "FiatToken: not paused")]
    fn test_unpause_already_unpaused() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(pauser());

        // Act.
        usdc.unpause();
    }

    #[test]
    fn test_update_pauser() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let pauser_id = accounts(2).clone();
        set_caller(owner());

        // Act.
        usdc.configure_multisig_role(Role::Pauser, pauser_id.clone());

        // Assert.
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"role_configured\",\"data\":{{\"role\":\"{}\",\"account_id\":\"{}\"}}}}", Role::Pauser, accounts(2).to_string())
        );

        // Check if pauser has changed.
        // Arrange.
        set_caller(pauser_id.clone());

        // Act.
        usdc.pause();

        // Assert.
        assert_eq!(
            test_utils::get_logs()[0],
            "EVENT_JSON:{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"paused\",\"data\":null}"
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Owner")]
    fn test_update_pauser_not_owner() {
        // Arrange.
        set_caller(accounts(1));

        let mut usdc: Contract = init_contract();

        // Act.
        usdc.configure_multisig_role(Role::Pauser, accounts(2).into());
    }

    #[test]
    fn test_blocklist_and_unblocklist() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let blocklister_account: AccountId = "blocklister".parse().unwrap();
        let to_block_account: AccountId = "block_me".parse().unwrap();
        init_account(&mut usdc, to_block_account.clone(), Some(U128::from(100)));
        set_caller(blocklister());

        // Act & Assert
        assert_eq!(usdc.blocklister(), blocklister_account);
        assert_eq!(usdc.is_blocklisted(to_block_account.clone()), false);
        usdc.blocklist(to_block_account.clone());
        assert_eq!(usdc.is_blocklisted(to_block_account.clone()), true);
        usdc.unblocklist(to_block_account.clone());
        assert_eq!(usdc.is_blocklisted(to_block_account.clone()), false);
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"blocklist\",\"data\":{{\"account_id\":\"{}\"}}}}", to_block_account.to_string())
        );
        assert_eq!(
            test_utils::get_logs()[1],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"unblocklist\",\"data\":{{\"account_id\":\"{}\"}}}}", to_block_account.to_string())
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Blocklister")]
    fn test_blocklist_not_blocklister() {
        // Arrange.
        let not_blocklister_account: AccountId = "not_blocklister".parse().unwrap();
        set_caller(not_blocklister_account);

        let mut usdc: Contract = init_contract();

        let to_block_account: AccountId = "block_me".parse().unwrap();
        init_account(&mut usdc, to_block_account.clone(), Some(U128::from(100)));

        // Act & Assert
        assert_eq!(usdc.is_blocklisted(to_block_account.clone()), false);
        usdc.blocklist(to_block_account.clone());
    }

    #[test]
    fn test_update_blocklister() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let new_blocklister_account: AccountId = "new_blocklister".parse().unwrap();
        init_account(
            &mut usdc,
            new_blocklister_account.clone(),
            Some(U128::from(100)),
        );
        set_caller(owner());

        // Act & Assert
        assert_eq!(usdc.blocklister(), blocklister());
        usdc.update_blocklister(new_blocklister_account.clone());
        assert_eq!(usdc.blocklister(), new_blocklister_account);
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"blocklister_changed\",\"data\":{{\"new_blocklister_id\":\"{}\"}}}}", new_blocklister_account.to_string())
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Owner")]
    fn test_update_blocklister_not_owner() {
        // Arrange.
        let not_owner_account: AccountId = "not_owner".parse().unwrap();
        set_caller(not_owner_account.clone());

        let mut usdc: Contract = init_contract();

        // Act & Assert
        assert_eq!(usdc.blocklister(), blocklister());
        usdc.update_blocklister(not_owner_account);
    }

    #[test]
    fn test_configure_owner() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let new_owner_id: AccountId = "new_owner".parse().unwrap();
        set_caller(owner());

        // Act.
        usdc.configure_multisig_role(Role::Owner, new_owner_id.clone());

        // Assert.
        assert!(usdc.owners().contains(&&new_owner_id));
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"role_configured\",\"data\":{{\"role\":\"{}\",\"account_id\":\"{}\"}}}}", Role::Owner, new_owner_id.to_string())
        );
    }

    #[test]
    fn test_configure_admin() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let new_admin_id: AccountId = "new_admin".parse().unwrap();
        set_caller(admin());

        // Act.
        let mut new_admin_ids: Vec<AccountId> = Vec::new();
        new_admin_ids.insert(0, new_admin_id.clone());
        usdc.configure_multisig_role(Role::Admin, new_admin_id.clone());

        // Assert.
        assert!(usdc.admins().contains(&&new_admin_id));
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"role_configured\",\"data\":{{\"role\":\"{}\",\"account_id\":\"{}\"}}}}", Role::Admin, new_admin_id.to_string())
        );
    }

    #[test]
    #[should_panic(expected = "RemovalNotAllowed(RequestStillValid)")]
    fn test_remove_multisig_request() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let new_master_minter_id: AccountId = "new_master_minter".parse().unwrap();
        let request_id: u32 = usdc.get_next_multisig_request_id();
        let update_master_minter_action: FiatTokenAction = FiatTokenAction::ConfigureMultisigRole {
            role: Role::MasterMinter,
            account_id: new_master_minter_id.clone(),
        };
        set_caller(owner());
        usdc.create_multisig_request(update_master_minter_action);

        // Act.
        usdc.remove_multisig_request(request_id);
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Owner")]
    fn test_create_multisig_request_missing_role() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let new_master_minter_id: AccountId = "new_master_minter".parse().unwrap();
        let update_master_minter_action: FiatTokenAction = FiatTokenAction::ConfigureMultisigRole {
            role: Role::MasterMinter,
            account_id: new_master_minter_id.clone(),
        };
        usdc._grant_multisig_role(new_master_minter_id, &Role::Controller);

        // Act.
        usdc.create_multisig_request(update_master_minter_action);
    }

    #[test]
    #[should_panic(
        expected = "called `Result::unwrap()` on an `Err` value: ExecutionEligibility(InsufficientApprovals { current: 1, required: 2 })"
    )]
    fn test_multisig_update_blocklister_insufficient_approvals() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let new_blocklister_id: AccountId = "new_blocklister".parse().unwrap();
        let update_blocklister_action: FiatTokenAction = FiatTokenAction::UpdateBlocklister {
            new_blocklister_id: new_blocklister_id.clone(),
        };
        set_caller(owner());
        let owner2: AccountId = "owner2".parse().unwrap();
        usdc.configure_multisig_role(Role::Owner, owner2);

        // Act.
        let update_blocklister_request_id: u32 =
            usdc.create_multisig_request(update_blocklister_action);
        usdc.approve_multisig_request(update_blocklister_request_id.clone());
        assert_eq!(
            test_utils::get_logs()[1],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"multisig_request_created\",\"data\":{{\"request_id\":{}}}}}", update_blocklister_request_id.to_string())
        );
        usdc.execute_multisig_request(update_blocklister_request_id.clone());
    }

    #[test]
    #[should_panic(
        expected = "called `Result::unwrap()` on an `Err` value: ApprovalError(AlreadyApprovedByAccount)"
    )]
    fn test_multisig_update_blocklister_same_approver() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let new_blocklister_id: AccountId = "new_blocklister".parse().unwrap();
        let update_blocklister_action: FiatTokenAction = FiatTokenAction::UpdateBlocklister {
            new_blocklister_id: new_blocklister_id.clone(),
        };
        set_caller(owner());

        // Act.
        let update_blocklister_request_id: u32 =
            usdc.create_multisig_request(update_blocklister_action);
        usdc.approve_multisig_request(update_blocklister_request_id);
        usdc.approve_multisig_request(update_blocklister_request_id.clone());
    }

    #[test]
    #[should_panic(expected = "FiatToken: caller is not a Owner")]
    fn test_multisig_configure_pauser_grant_then_revoke_role() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let new_pauser_id: AccountId = "new_pauser".parse().unwrap();
        let pauser2: AccountId = "pauser2".parse().unwrap();
        set_caller(pauser());
        let configure_pauser_action: FiatTokenAction = FiatTokenAction::ConfigureMultisigRole {
            role: Role::Pauser,
            account_id: new_pauser_id.clone(),
        };

        // Act.
        set_caller(pauser());
        let configure_pauser_request_id: u32 =
            usdc.create_multisig_request(configure_pauser_action);
        usdc.approve_multisig_request(configure_pauser_request_id);
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"multisig_request_created\",\"data\":{{\"request_id\":{}}}}}", configure_pauser_request_id.to_string())
        );
        // Grant the account the Multisig and Owner Role but then revoke it.
        usdc._revoke_multisig_role(&pauser2, &Role::Pauser);
        set_caller(pauser2);
        usdc.approve_multisig_request(configure_pauser_request_id.clone());
    }

    #[test]
    fn test_multisig_configure_minter_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let minter2: AccountId = "minter2".parse().unwrap();
        let controller2a: AccountId = "controller2a".parse().unwrap();
        let controller2b: AccountId = "controller2b".parse().unwrap();
        set_caller(master_minter());
        usdc.configure_controller(controller2a.clone(), minter2.clone());
        usdc.configure_controller(controller2b.clone(), minter2.clone());
        let configure_minter_allowance_action: FiatTokenAction =
            FiatTokenAction::ConfigureMinterAllowance {
                controller_id: controller2a.clone(),
                minter_allowance: U128::from(12345),
            };
        set_caller(controller2a);

        // Act.
        let configure_minter_allowance_request_id: u32 =
            usdc.create_multisig_request(configure_minter_allowance_action);
        usdc.approve_multisig_request(configure_minter_allowance_request_id);
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"multisig_request_created\",\"data\":{{\"request_id\":{}}}}}", configure_minter_allowance_request_id.to_string())
        );
        set_caller(controller2b);
        usdc.approve_multisig_request(configure_minter_allowance_request_id.clone());
        usdc.execute_multisig_request(configure_minter_allowance_request_id.clone());

        // Assert.
        assert!(usdc.is_minter(&minter2));
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"minter_configured\",\"data\":{{\"minter_id\":\"{}\",\"minter_allowance\":\"12345\"}}}}", minter2.to_string())
        );
    }

    #[test]
    #[should_panic(
        expected = "FiatToken: can only approve requests to configure the allowance of your own minter"
    )]
    fn test_multisig_only_approve_requests_to_configure_own_minter() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let minter2: AccountId = "minter2".parse().unwrap();
        let controller2: AccountId = "controller2".parse().unwrap();
        set_caller(master_minter());
        usdc.configure_controller(controller2.clone(), minter2.clone());
        let configure_minter_allowance_action: FiatTokenAction =
            FiatTokenAction::ConfigureMinterAllowance {
                controller_id: controller2.clone(),
                minter_allowance: U128::from(12345),
            };
        set_caller(controller());

        // Act.
        // Attempt to approve (as controller2) a request for configuring controller's minter.
        let configure_minter_allowance_request_id: u32 =
            usdc.create_multisig_request(configure_minter_allowance_action);
        usdc.approve_multisig_request(configure_minter_allowance_request_id);
    }

    #[test]
    fn test_multisig_decrease_minter_allowance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let minter2: AccountId = "minter2".parse().unwrap();
        let controller2a: AccountId = "controller2a".parse().unwrap();
        let controller2b: AccountId = "controller2b".parse().unwrap();
        set_caller(master_minter());
        usdc.configure_controller(controller2a.clone(), minter2.clone());
        usdc.configure_controller(controller2b.clone(), minter2.clone());
        let dec_minter_allowance_action: FiatTokenAction =
            FiatTokenAction::DecreaseMinterAllowance {
                controller_id: controller2b.clone(),
                decrement: U128(50),
            };

        // Act.
        set_caller(controller2b);
        usdc.configure_minter_allowance(U128::from(51));
        let dec_minter_allowance_request_id: u32 =
            usdc.create_multisig_request(dec_minter_allowance_action);
        usdc.approve_multisig_request(dec_minter_allowance_request_id);

        set_caller(controller2a.clone());
        usdc.approve_multisig_request(dec_minter_allowance_request_id.clone());
        usdc.execute_multisig_request(dec_minter_allowance_request_id.clone());

        // Assert.
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"minter_configured\",\"data\":{{\"minter_id\":\"{}\",\"minter_allowance\":{}}}}}",
                    minter2.to_string(),
                    &near_sdk::serde_json::to_string(&U128::from(1)).unwrap())
        );
    }

    #[test]
    fn test_get_next_multisig_request_id() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        set_caller(owner());
        let new_pauser_id: AccountId = "new_pauser".parse().unwrap();
        let update_pauser_action: FiatTokenAction = FiatTokenAction::ConfigureMultisigRole {
            role: Role::Pauser,
            account_id: new_pauser_id.clone(),
        };
        // The first multisig request created by a contract should always be 0.
        assert_eq!(usdc.get_next_multisig_request_id(), 0);

        // Act.
        let update_pauser_request_id: u32 = usdc.create_multisig_request(update_pauser_action);

        // Assert.
        assert_eq!(update_pauser_request_id, 0);
        // We have used up request ID 0, so the next one should be 1.
        assert_eq!(usdc.get_next_multisig_request_id(), 1);
    }

    #[test]
    fn test_approve_for_upgrade() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let approve_for_upgrade_action: FiatTokenAction = FiatTokenAction::ApproveForUpgrade;
        set_caller(admin());
        let admin2: AccountId = "admin2".parse().unwrap();
        usdc.configure_multisig_role(Role::Admin, admin2.clone());

        // Act.
        let approve_for_upgrade_action_request_id: u32 =
            usdc.create_multisig_request(approve_for_upgrade_action);
        usdc.approve_multisig_request(approve_for_upgrade_action_request_id);
        set_caller(admin2);
        usdc.approve_multisig_request(approve_for_upgrade_action_request_id.clone());
        usdc.execute_multisig_request(approve_for_upgrade_action_request_id.clone());

        // Assert.
        assert_eq!(
            test_utils::get_logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"approved_for_upgrade\",\"data\":null}}")
        );
        assert_eq!(usdc.approved_for_upgrade, true);
    }

    #[test]
    fn test_storage_deposit() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Some(accounts(0)), Option::None);

        // Assert.
        assert!(usdc.storage_balance_of(accounts(0)).is_some());
        assert_eq!(
            usdc.storage_balance_of(accounts(0)).unwrap().total.0,
            account_storage_deposit
        );
    }

    #[test]
    fn test_storage_deposit_none_account_passed() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Option::None, Option::None);

        // Assert.
        assert!(usdc.storage_balance_of(accounts(0)).is_some());
        assert_eq!(
            usdc.storage_balance_of(accounts(0)).unwrap().total.0,
            account_storage_deposit
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: alice is blocklisted")]
    fn test_storage_deposit_caller_blocklisted() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let blocklisted_account: AccountId = accounts(0);
        _blocklist(&mut usdc, blocklisted_account.clone());

        // Act.
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(blocklisted_account);
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Some(accounts(1)), Option::None);
    }

    #[test]
    #[should_panic(expected = "FiatToken: bob is blocklisted")]
    fn test_storage_deposit_beneficiary_blocklisted() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let blocklisted_account: AccountId = accounts(1);
        _blocklist(&mut usdc, blocklisted_account.clone());

        // Act.
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Some(accounts(1)), Option::None);
    }

    #[test]
    #[should_panic(expected = "The attached deposit is less than the minimum storage balance")]
    fn test_storage_deposit_insufficient_deposit() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        // Set the deposit to be just under the minimum required amount.
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0 - 1;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Some(accounts(1)), Option::None);
    }

    #[test]
    fn test_storage_deposit_already_registered() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let already_registered_account: AccountId = accounts(1);
        init_account(&mut usdc, already_registered_account.clone(), None);

        // Act.
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(account_storage_deposit);
        testing_env!(context.build());
        usdc.storage_deposit(Some(accounts(1)), Option::None);

        // Assert.
        assert_eq!(
            test_utils::get_logs()[0],
            "The account is already registered, refunding the deposit"
        );
    }

    #[test]
    fn test_storage_unregister() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        init_account(&mut usdc, accounts(0), None);

        // Act.
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(ONE_YOCTO);
        testing_env!(context.build());
        usdc.storage_unregister(Some(false));

        // Assert.
        assert!(usdc.storage_balance_of(accounts(0)).is_none());
    }

    #[test]
    #[should_panic(expected = "FiatToken: cannot force storage unregister")]
    fn test_storage_unregister_force_not_allowed() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(ONE_YOCTO);
        testing_env!(context.build());
        usdc.storage_unregister(Some(true));
    }

    #[test]
    fn test_storage_unregister_not_registered() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(ONE_YOCTO);
        testing_env!(context.build());
        usdc.storage_unregister(Some(false));

        // Assert.
        assert!(usdc.storage_balance_of(accounts(0)).is_none());
        assert_eq!(
            test_utils::get_logs()[0],
            format!("The account {} is not registered", accounts(0))
        );
    }

    #[test]
    #[should_panic(expected = "FiatToken: alice is blocklisted")]
    fn test_storage_unregister_caller_blocklisted() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let blocklisted_account: AccountId = accounts(0);
        _blocklist(&mut usdc, blocklisted_account.clone());

        // Act.
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(ONE_YOCTO);
        testing_env!(context.build());
        usdc.storage_unregister(Some(false));
    }

    #[test]
    #[should_panic(expected = "FiatToken: cannot unregister an account with a positive balance")]
    fn test_storage_unregister_cannot_unregister_with_positive_balance() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        // init_account also registers.
        init_account(&mut usdc, accounts(0), Some(U128::from(123)));

        // Act.
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(ONE_YOCTO);
        testing_env!(context.build());
        usdc.storage_unregister(Some(false));

        // Assert.
        assert!(usdc.storage_balance_of(accounts(0)).is_some());
    }

    #[test]
    fn test_storage_withdraw() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        let account_storage_deposit: u128 = usdc.storage_balance_bounds().min.0;
        init_account(&mut usdc, accounts(0), None);

        // Act.
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(ONE_YOCTO);
        testing_env!(context.build());
        let storage_balance: StorageBalance = usdc.storage_withdraw(Some(U128::from(0)));

        // Assert.
        // The default storage_withdraw implementation doesn't actually do anything but return a StorageBalance object.
        assert!(usdc.storage_balance_of(accounts(0)).is_some());
        assert_eq!(storage_balance.total.0, account_storage_deposit);
    }

    #[test]
    #[should_panic(expected = "The amount is greater than the available storage balance")]
    fn test_storage_withdraw_panic_if_amount_greater_than_zero() {
        // Arrange.
        let mut usdc: Contract = init_contract();
        init_account(&mut usdc, accounts(0), None);

        // Act.
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(ONE_YOCTO);
        testing_env!(context.build());
        usdc.storage_withdraw(Some(U128::from(1)));
    }

    #[test]
    #[should_panic(expected = "The account alice is not registered")]
    fn test_storage_withdraw_account_not_registered() {
        // Arrange.
        let mut usdc: Contract = init_contract();

        // Act.
        let mut context: VMContextBuilder = get_context(accounts(0));
        context.attached_deposit(ONE_YOCTO);
        testing_env!(context.build());
        usdc.storage_withdraw(Some(U128::from(0)));
    }
    // TODO: Test for removing request and validity_period
}
