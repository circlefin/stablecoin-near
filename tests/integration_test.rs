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
use near_contract_standards::storage_management::StorageBalance;
use near_sdk::json_types::U128;
use near_sdk::{
    base64,
    serde_json::{json, Value},
    AccountId,
};
use near_sdk_contract_tools::standard::nep148::FungibleTokenMetadata;
use near_workspaces::{Account, Contract};

const FIAT_TOKEN_WASM: &[u8] = include_bytes!("./data/fiat_token.wasm");
// Upgraded version of the contract that changes the multi-sig request's validity period to 1 ns.
const UPGRADED_FIAT_TOKEN_1NS_VALIDITY_PERIOD_WASM: &[u8] =
    include_bytes!("./data/1ns_validity_period.wasm");
// Upgraded version of the contract that changes the token name to "USDC V2", adds a was_upgraded
// field exposed by a was_contract_upgraded function, and also increases the approval threshold
// from 2 to 3.
const UPGRADED_FIAT_TOKEN_NEW_NAME_3_APPROVALS_WASM: &[u8] =
    include_bytes!("./data/new_struct_and_name_3_approvals.wasm");
const NUM_REQUIRED_ACCOUNTS: usize = 14;
const ACCOUNT_STORAGE_COST: u128 = 1250000000000000000000;

struct Setup {
    pub contract: Contract,
    pub accounts: Vec<Account>,
}

/// Setup for individual tests
async fn setup(extra_accounts: usize, wasm: &[u8]) -> Setup {
    let worker = near_workspaces::sandbox().await.unwrap();
    // Initialize user accounts
    let mut accounts = vec![];
    for _ in 0..(NUM_REQUIRED_ACCOUNTS + extra_accounts) {
        accounts.push(worker.dev_create_account().await.unwrap());
    }

    let token_account = &accounts[0].clone();

    let mut admin_ids = Vec::new();
    admin_ids.push(accounts[1].id());
    admin_ids.push(accounts[2].id());
    admin_ids.push(accounts[3].id());

    let blocklister_id = &accounts[4].id();

    let mut master_minter_ids = Vec::new();
    master_minter_ids.push(accounts[5].id());
    master_minter_ids.push(accounts[6].id());
    master_minter_ids.push(accounts[7].id());

    let mut owner_ids = Vec::new();
    owner_ids.push(accounts[8].id());
    owner_ids.push(accounts[9].id());
    owner_ids.push(accounts[10].id());

    let mut pauser_ids = Vec::new();
    pauser_ids.push(accounts[11].id());
    pauser_ids.push(accounts[12].id());
    pauser_ids.push(accounts[13].id());

    let contract = token_account.deploy(&wasm.to_vec()).await.unwrap().unwrap();

    contract
        .call("init")
        .max_gas()
        .args_json(json!({
            "admin_ids": admin_ids,
            "master_minter_ids": master_minter_ids,
            "owner_ids": owner_ids,
            "pauser_ids": pauser_ids,
            "blocklister_id": blocklister_id,
            "metadata": json!({
                "spec": "ft-1.0.0",
                "name": "USD Coin",
                "symbol": "USDC",
                "decimals": 6
            }),
        }))
        .transact()
        .await
        .unwrap()
        .unwrap();

    Setup { contract, accounts }
}

#[tokio::test]
async fn test_contract_init() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;
    let contract_account: &Account = &accounts[0];
    let admin1: &Account = &accounts[1];
    let admin2: &Account = &accounts[2];
    let admin3: &Account = &accounts[3];
    let blocklister: &Account = &accounts[4];
    let master_minter1: &Account = &accounts[5];
    let master_minter2: &Account = &accounts[6];
    let master_minter3: &Account = &accounts[7];
    let owner1: &Account = &accounts[8];
    let owner2: &Account = &accounts[9];
    let owner3: &Account = &accounts[10];
    let pauser1: &Account = &accounts[11];
    let pauser2: &Account = &accounts[12];
    let pauser3: &Account = &accounts[13]; // 14 (NUM_ACCOUNTS) accounts up to here required for all the main contract roles

    let admins: Vec<AccountId> = contract_account
        .call(contract.id(), "admins")
        .transact()
        .await
        .unwrap()
        .json::<Vec<AccountId>>()
        .unwrap();

    assert_eq!(admins.len(), 3);
    assert_eq!(admins[0].as_str(), admin1.id().as_str());
    assert_eq!(admins[1].as_str(), admin2.id().as_str());
    assert_eq!(admins[2].as_str(), admin3.id().as_str());

    let contract_blocklister: AccountId = contract_account
        .call(contract.id(), "blocklister")
        .transact()
        .await
        .unwrap()
        .json::<AccountId>()
        .unwrap();

    assert_eq!(contract_blocklister.as_str(), blocklister.id().as_str());

    let master_minters: Vec<AccountId> = contract_account
        .call(contract.id(), "master_minters")
        .transact()
        .await
        .unwrap()
        .json::<Vec<AccountId>>()
        .unwrap();

    assert_eq!(master_minters.len(), 3);
    assert_eq!(master_minters[0].as_str(), master_minter1.id().as_str());
    assert_eq!(master_minters[1].as_str(), master_minter2.id().as_str());
    assert_eq!(master_minters[2].as_str(), master_minter3.id().as_str());

    let owners: Vec<AccountId> = contract_account
        .call(contract.id(), "owners")
        .transact()
        .await
        .unwrap()
        .json::<Vec<AccountId>>()
        .unwrap();

    assert_eq!(owners.len(), 3);
    assert_eq!(owners[0].as_str(), owner1.id().as_str());
    assert_eq!(owners[1].as_str(), owner2.id().as_str());
    assert_eq!(owners[2].as_str(), owner3.id().as_str());

    let pausers: Vec<AccountId> = contract_account
        .call(contract.id(), "pausers")
        .transact()
        .await
        .unwrap()
        .json::<Vec<AccountId>>()
        .unwrap();

    assert_eq!(pausers.len(), 3);
    assert_eq!(pausers[0].as_str(), pauser1.id().as_str());
    assert_eq!(pausers[1].as_str(), pauser2.id().as_str());
    assert_eq!(pausers[2].as_str(), pauser3.id().as_str());

    let total_supply: U128 = contract_account
        .call(contract.id(), "ft_total_supply")
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(total_supply, U128::from(0));
}

#[tokio::test]
async fn test_configure_minter_allowance_mint_and_burn() {
    let Setup { contract, accounts } = setup(4, FIAT_TOKEN_WASM).await;

    // We need 2/3 master minters to be able to configure a controller, and at least
    // 2/3 controllers to control a minter.
    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        1234567,
    )
    .await;

    let total_supply_before_mint = minter
        .call(contract.id(), "ft_total_supply")
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    let mint_amount: u128 = 1234567;
    mint(
        contract.clone(),
        minter.clone(),
        minter.clone(),
        mint_amount,
    )
    .await;
    let total_supply_after_mint = minter
        .call(contract.id(), "ft_total_supply")
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(
        total_supply_before_mint.0 + mint_amount,
        total_supply_after_mint.0
    );

    let burn_amount: u128 = 123;
    burn(contract.clone(), minter.clone(), burn_amount).await;
    let total_supply_after_burn = minter
        .call(contract.id(), "ft_total_supply")
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(
        total_supply_after_mint.0 - burn_amount,
        total_supply_after_burn.0
    );
}

#[tokio::test]
async fn test_increase_and_decrease_minter_allowance() {
    let Setup { contract, accounts } = setup(4, FIAT_TOKEN_WASM).await;
    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    let initial_minter_allowance: u128 = 1234567;
    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        initial_minter_allowance,
    )
    .await;
    let increment: u128 = 10;
    increase_minter_allowance(
        contract.clone(),
        controller1.clone(),
        controller2.clone(),
        increment,
    )
    .await;
    let minter_allowance_after_increment: U128 = minter
        .call(contract.id(), "minter_allowance")
        .args_json(json!({ "minter_id": minter.id() }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(
        minter_allowance_after_increment,
        U128::from(initial_minter_allowance + increment)
    );
    let decrement: u128 = 5;
    decrease_minter_allowance(
        contract.clone(),
        controller1.clone(),
        controller2.clone(),
        decrement,
    )
    .await;
    let minter_allowance_after_decrement: U128 = minter
        .call(contract.id(), "minter_allowance")
        .args_json(json!({ "minter_id": minter.id() }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(
        minter_allowance_after_decrement,
        U128::from(initial_minter_allowance + increment - decrement)
    );
}

#[tokio::test]
#[should_panic = "FiatToken: attempted to overflow minter allowance"]
async fn test_increase_minter_allowance_attempted_to_overflow() {
    let Setup { contract, accounts } = setup(4, FIAT_TOKEN_WASM).await;
    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    let initial_minter_allowance: u128 = u128::MAX;
    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        initial_minter_allowance,
    )
    .await;
    let increment: u128 = 1;
    increase_minter_allowance(
        contract.clone(),
        controller1.clone(),
        controller2.clone(),
        increment,
    )
    .await;
}

#[tokio::test]
#[should_panic = "FiatToken: attempted to underflow minter allowance"]
async fn test_decrease_minter_allowance_attempted_to_underflow() {
    let Setup { contract, accounts } = setup(4, FIAT_TOKEN_WASM).await;
    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    let initial_minter_allowance: u128 = 1;
    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        initial_minter_allowance,
    )
    .await;
    let decrement: u128 = 2;
    decrease_minter_allowance(
        contract.clone(),
        controller1.clone(),
        controller2.clone(),
        decrement,
    )
    .await;
}

#[tokio::test]
async fn test_approve_increase_decrease_allowance_transfer_from() {
    let Setup { contract, accounts } = setup(6, FIAT_TOKEN_WASM).await;

    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    let token_holder = &accounts[NUM_REQUIRED_ACCOUNTS + 3];
    let spender = &accounts[NUM_REQUIRED_ACCOUNTS + 4];
    let transfer_receiver = &accounts[NUM_REQUIRED_ACCOUNTS + 5];

    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        1234567,
    )
    .await;
    mint(
        contract.clone(),
        minter.clone(),
        token_holder.clone(),
        12345,
    )
    .await;

    let initial_approval_amount: u128 = 12345;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        initial_approval_amount,
    )
    .await;
    let increment: u128 = 10;
    increase_allowance(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        increment,
    )
    .await;
    let increased_allowance_amount: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(
        increased_allowance_amount,
        U128::from(initial_approval_amount + increment)
    );

    let decrement: u128 = 5;
    decrease_allowance(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        decrement,
    )
    .await;
    let decreased_allowance_amount: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(
        decreased_allowance_amount,
        U128::from(initial_approval_amount + increment - decrement)
    );

    // Register transfer receiver with token to be able to receive transfers.
    transfer_receiver
        .call(contract.id(), "storage_deposit")
        .args_json(json!({ "account_id": transfer_receiver.id() }))
        .deposit(ACCOUNT_STORAGE_COST)
        .transact()
        .await
        .unwrap()
        .unwrap();

    // Transfer tokens from the token holder to a receiver.
    // Note that this transfer will be initiated by the spender on behalf of the token holder.
    let transfer_amount: u128 = 123;
    let transfer_result = spender
        .call(contract.id(), "transfer_from")
        .args_json(json!({
            "from": token_holder.id(),
            "to": transfer_receiver.id(),
            "value": U128::from(transfer_amount),
        }))
        .transact()
        .await
        .unwrap();
    assert_eq!(
        transfer_result.logs()[0],
        format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_transfer\",\"data\":[{{\"old_owner_id\":{:?},\"new_owner_id\":{:?},\"amount\":\"{:?}\"}}]}}", token_holder.id().as_str(), transfer_receiver.id().as_str(), transfer_amount)
    );
    transfer_result.unwrap();
}

#[tokio::test]
async fn test_approve_twice_transfer_from() {
    let Setup { contract, accounts } = setup(6, FIAT_TOKEN_WASM).await;

    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    let token_holder = &accounts[NUM_REQUIRED_ACCOUNTS + 3];
    let spender = &accounts[NUM_REQUIRED_ACCOUNTS + 4];
    let transfer_receiver = &accounts[NUM_REQUIRED_ACCOUNTS + 5];

    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        1234567,
    )
    .await;
    mint(
        contract.clone(),
        minter.clone(),
        token_holder.clone(),
        12345,
    )
    .await;

    let initial_approval_amount: u128 = 123;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        initial_approval_amount,
    )
    .await;
    let new_approval_amount: u128 = 124;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        new_approval_amount,
    )
    .await;

    // Register transfer receiver with token to be able to receive transfers.
    transfer_receiver
        .call(contract.id(), "storage_deposit")
        .args_json(json!({ "account_id": transfer_receiver.id() }))
        .deposit(ACCOUNT_STORAGE_COST)
        .transact()
        .await
        .unwrap()
        .unwrap();

    // Transfer tokens from the token holder to a receiver.
    // Note that this transfer will be initiated by the spender on behalf of the token holder.
    let transfer_amount: u128 = 124;
    let transfer_result = spender
        .call(contract.id(), "transfer_from")
        .args_json(json!({
            "from": token_holder.id(),
            "to": transfer_receiver.id(),
            "value": U128::from(transfer_amount),
        }))
        .transact()
        .await
        .unwrap();
    assert_eq!(
        transfer_result.logs()[0],
        format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_transfer\",\"data\":[{{\"old_owner_id\":{:?},\"new_owner_id\":{:?},\"amount\":\"{:?}\"}}]}}", token_holder.id().as_str(), transfer_receiver.id().as_str(), transfer_amount)
    );
    transfer_result.unwrap();
}

#[tokio::test]
async fn test_approve_different_holders_same_spender_should_be_different() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;
    let token_holder = &accounts[0];
    let spender = &accounts[1];
    let token_holder2 = &accounts[2];

    let initial_approval_amount: u128 = 12345;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        initial_approval_amount,
    )
    .await;

    let allowance_amount_before: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(initial_approval_amount, allowance_amount_before.0);

    approve(
        contract.clone(),
        token_holder2.clone(),
        spender.clone(),
        initial_approval_amount + 20000,
    )
    .await;

    let allowance_amount_before2: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder2.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(initial_approval_amount + 20000, allowance_amount_before2.0);

    let allowance_amount_after: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(initial_approval_amount, allowance_amount_after.0);

    let allowance_amount_after2: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder2.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(initial_approval_amount + 20000, allowance_amount_after2.0);
}

#[tokio::test]
async fn test_approval_same_holders_different_spender_should_be_different() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;
    let token_holder = &accounts[0];
    let spender = &accounts[1];
    let spender2 = &accounts[2];

    let spender_allowance: u128 = 12345;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        spender_allowance,
    )
    .await;

    let retrieved_spender_allowance: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(spender_allowance, retrieved_spender_allowance.0);

    let spender2_allowance = spender_allowance + 20000;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender2.clone(),
        spender2_allowance,
    )
    .await;

    let retrieved_spender2_allowance: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender2.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(spender2_allowance, retrieved_spender2_allowance.0);

    let retrieved_again_spender_allowance: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(spender_allowance, retrieved_again_spender_allowance.0);

    let retrieved_again_spender2_allowance: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender2.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(spender2_allowance, retrieved_again_spender2_allowance.0);
}

#[tokio::test]
#[should_panic = "FiatToken: transfer amount exceeds allowance"]
async fn test_transfer_from_exceeds_allowance() {
    let Setup { contract, accounts } = setup(6, FIAT_TOKEN_WASM).await;

    // We need 2/3 master minters to be able to configure a controller, and at least
    // 2/3 controllers to control a minter in order to mint.
    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    let token_holder = &accounts[NUM_REQUIRED_ACCOUNTS + 3];
    let spender = &accounts[NUM_REQUIRED_ACCOUNTS + 4];
    let transfer_receiver = &accounts[NUM_REQUIRED_ACCOUNTS + 5];

    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        1234567,
    )
    .await;
    mint(
        contract.clone(),
        minter.clone(),
        token_holder.clone(),
        12345,
    )
    .await;
    let approval_amount: u128 = 12345;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        approval_amount,
    )
    .await;

    // Attempt to transfer more than the approved amount
    let transfer_amount: u128 = approval_amount + 1;
    spender
        .call(contract.id(), "transfer_from")
        .args_json(json!({
            "from": token_holder.id(),
            "to": transfer_receiver.id(),
            "value": U128::from(transfer_amount),
        }))
        .transact()
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[should_panic = "FiatToken: must approve initial allowance before incrementing"]
async fn test_increase_allowance_not_previously_approved() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;
    let token_holder = &accounts[0];
    let spender = &accounts[1];
    increase_allowance(contract.clone(), token_holder.clone(), spender.clone(), 5).await;
}

#[tokio::test]
#[should_panic = "FiatToken: allowance increment must be greater than 0"]
async fn test_increase_allowance_bad_increment() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;
    let token_holder = &accounts[0];
    let spender = &accounts[1];
    let allowance: u128 = 1234567;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        allowance,
    )
    .await;
    increase_allowance(contract.clone(), token_holder.clone(), spender.clone(), 0).await;
}

#[tokio::test]
#[should_panic = "FiatToken: allowance decrement must be greater than 0"]
async fn test_increase_allowance_bad_decrement() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;
    let token_holder = &accounts[0];
    let spender = &accounts[1];
    let allowance: u128 = 1234567;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        allowance,
    )
    .await;
    decrease_allowance(contract.clone(), token_holder.clone(), spender.clone(), 0).await;
}

#[tokio::test]
#[should_panic = "FiatToken: must approve initial allowance before decrementing"]
async fn test_decrease_allowance_not_previously_approved() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;
    let token_holder = &accounts[0];
    let spender = &accounts[1];
    decrease_allowance(contract.clone(), token_holder.clone(), spender.clone(), 5).await;
}

#[tokio::test]
#[should_panic = "FiatToken: attempted to underflow allowance"]
async fn test_decrease_allowance_attempted_to_underflow() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;
    let token_holder = &accounts[0];
    let spender = &accounts[1];
    let allowance: u128 = 1234567;
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        allowance,
    )
    .await;
    decrease_allowance(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        allowance + 1,
    )
    .await;
}

#[tokio::test]
#[should_panic = "is blocklisted"]
async fn test_approve_blocklisted_account_to_spend() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;

    let token_holder = &accounts[2];
    let blocklisted_account = &accounts[3];
    let blocklister = &accounts[4];

    blocklist(
        contract.clone(),
        blocklister.clone(),
        blocklisted_account.clone(),
    )
    .await;
    approve(
        contract.clone(),
        token_holder.clone(),
        blocklisted_account.clone(),
        123,
    )
    .await;
}

#[tokio::test]
async fn test_approve_caller_blocklisted_then_unblocklisted() {
    let Setup { contract, accounts } = setup(1, FIAT_TOKEN_WASM).await;

    let token_holder = &accounts[2];
    let blocklister = &accounts[4];
    let spender = &accounts[NUM_REQUIRED_ACCOUNTS];
    let approval_amount: u128 = 12345;

    blocklist(contract.clone(), blocklister.clone(), token_holder.clone()).await;

    // Can still view blocklisted account's allowance.
    let original_allowance: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();

    let approve_result = token_holder
        .call(contract.id(), "approve")
        .args_json(json!({
            "spender_id": spender.id(),
            "value": U128::from(approval_amount),
        }))
        .transact()
        .await
        .unwrap();
    // Approve should not have gone through as the token holder/caller was blocklisted.
    assert!(approve_result.is_failure());

    let blocklisted_approve_allowance: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(original_allowance, blocklisted_approve_allowance);

    // Unblocklist the token holder.
    let unblocklist_result = blocklister
        .call(contract.id(), "unblocklist")
        .args_json(json!({
            "account_id": token_holder.id()
        }))
        .transact()
        .await
        .unwrap();
    assert_eq!(
        unblocklist_result.logs()[0],
        format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"unblocklist\",\"data\":{{\"account_id\":{:?}}}}}", token_holder.id().as_str())
    );
    unblocklist_result.unwrap();

    // Approve with the now-unblocklisted token holder.
    approve(
        contract.clone(),
        token_holder.clone(),
        spender.clone(),
        approval_amount,
    )
    .await;

    let approve_allowance: U128 = token_holder
        .call(contract.id(), "allowance")
        .args_json(json!({
            "holder_id": token_holder.id(),
            "spender_id": spender.id()
        }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(
        original_allowance.0 + approve_allowance.0,
        approve_allowance.0
    );
}

#[tokio::test]
#[should_panic = "is blocklisted"]
async fn test_transfer_from_caller_blocklisted() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;

    let token_holder = &accounts[2];
    let blocklisted_account = &accounts[3];
    let blocklister = &accounts[4];

    blocklist(
        contract.clone(),
        blocklister.clone(),
        blocklisted_account.clone(),
    )
    .await;
    blocklisted_account
        .call(contract.id(), "transfer_from")
        .args_json(json!({
            "from": token_holder.id(),
            "to": blocklister.id(),
            "value": U128::from(123),
        }))
        .transact()
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[should_panic = "is blocklisted"]
async fn test_transfer_from_from_blocklisted() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;

    let token_holder = &accounts[2];
    let blocklisted_account = &accounts[3];
    let blocklister = &accounts[4];

    blocklist(
        contract.clone(),
        blocklister.clone(),
        blocklisted_account.clone(),
    )
    .await;
    token_holder
        .call(contract.id(), "transfer_from")
        .args_json(json!({
            "from": blocklisted_account.id(),
            "to": blocklister.id(),
            "value": U128::from(123),
        }))
        .transact()
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[should_panic = "is blocklisted"]
async fn test_transfer_from_to_blocklisted() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;

    let token_holder = &accounts[2];
    let blocklisted_account = &accounts[3];
    let blocklister = &accounts[4];

    blocklist(
        contract.clone(),
        blocklister.clone(),
        blocklisted_account.clone(),
    )
    .await;
    token_holder
        .call(contract.id(), "transfer_from")
        .args_json(json!({
            "from": blocklister.id(),
            "to": blocklisted_account.id(),
            "value": U128::from(123),
        }))
        .transact()
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_ft_transfer() {
    let Setup { contract, accounts } = setup(6, FIAT_TOKEN_WASM).await;

    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    let token_holder = &accounts[NUM_REQUIRED_ACCOUNTS + 3];
    let transfer_receiver = &accounts[NUM_REQUIRED_ACCOUNTS + 5];

    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        1234567,
    )
    .await;
    mint(
        contract.clone(),
        minter.clone(),
        token_holder.clone(),
        12345,
    )
    .await;

    // Fail to transfer tokens to a receiver that has not registered with the token yet.
    let transfer_amount: u128 = 124;
    let transfer_result = token_holder
        .call(contract.id(), "ft_transfer")
        .args_json(json!({
            "receiver_id": transfer_receiver.id(),
            "amount": U128::from(transfer_amount),
        }))
        .deposit(1)
        .transact()
        .await
        .unwrap();
    assert!(transfer_result.is_failure());

    // Have transfer receiver register itself.
    transfer_receiver
        .call(contract.id(), "storage_deposit")
        .args_json(json!({ "account_id": transfer_receiver.id(), "registration_only": false}))
        .deposit(ACCOUNT_STORAGE_COST)
        .transact()
        .await
        .unwrap()
        .unwrap();

    // Fail to call ft_transfer without attaching a deposit.
    let no_deposit_transfer_result = token_holder
        .call(contract.id(), "ft_transfer")
        .args_json(json!({
            "receiver_id": transfer_receiver.id(),
            "amount": U128::from(transfer_amount),
        }))
        .transact()
        .await
        .unwrap();
    assert!(no_deposit_transfer_result.is_failure());

    // Transfer tokens from the token holder to a receiver.
    let successful_transfer_result = token_holder
        .call(contract.id(), "ft_transfer")
        .args_json(json!({
            "receiver_id": transfer_receiver.id(),
            "amount": U128::from(transfer_amount),
        }))
        .deposit(1)
        .transact()
        .await
        .unwrap();
    assert_eq!(
        successful_transfer_result.logs()[0],
        format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_transfer\",\"data\":[{{\"old_owner_id\":{:?},\"new_owner_id\":{:?},\"amount\":\"{:?}\"}}]}}", token_holder.id().as_str(), transfer_receiver.id().as_str(), transfer_amount)
    );
    successful_transfer_result.unwrap();
}

#[tokio::test]
#[should_panic = "FiatToken: mint amount exceeds minter allowance"]
async fn test_mint_exceeds_allowance() {
    let Setup { contract, accounts } = setup(3, FIAT_TOKEN_WASM).await;
    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        minter.clone(),
        123,
    )
    .await;
    mint(contract.clone(), minter.clone(), minter.clone(), 12345).await;
}

#[tokio::test]
#[should_panic = "FiatToken: burn amount exceeds balance"]
async fn test_burn_exceeds_allowance() {
    let Setup { contract, accounts } = setup(3, FIAT_TOKEN_WASM).await;
    let master_minter1 = &accounts[5];
    let master_minter2 = &accounts[6];
    let controller1 = &accounts[NUM_REQUIRED_ACCOUNTS];
    let controller2 = &accounts[NUM_REQUIRED_ACCOUNTS + 1];
    let burner = &accounts[NUM_REQUIRED_ACCOUNTS + 2];
    configure_minter_allowance(
        contract.clone(),
        master_minter1.clone(),
        master_minter2.clone(),
        controller1.clone(),
        controller2.clone(),
        burner.clone(),
        1234567,
    )
    .await;
    burn(contract, burner.clone(), 123).await;
}

#[tokio::test]
async fn test_upgrade() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;

    // We need 2/3 admins to be able to upgrade.
    let admin1 = &accounts[1];
    let admin2 = &accounts[2];
    upgrade_contract(
        contract.clone(),
        UPGRADED_FIAT_TOKEN_NEW_NAME_3_APPROVALS_WASM,
        admin1.clone(),
        admin2.clone(),
    )
    .await;

    // Assert that the token's metadata name was also upgraded to "USDC V2".
    let upgraded_metadata = admin1
        .view(contract.id(), "ft_metadata")
        .await
        .unwrap()
        .json::<FungibleTokenMetadata>()
        .unwrap();
    assert_eq!(upgraded_metadata.name, "USDC V2");

    // Assert that the token's new struct member "was_upgraded" was set to true and can be called
    // by a newly added "was_contract_upgraded" function.
    let was_contract_upgraded = admin1
        .view(contract.id(), "was_contract_upgraded")
        .await
        .unwrap()
        .json::<bool>()
        .unwrap();
    assert!(was_contract_upgraded);

    // Attempt to upgrade again and fail, verifying that
    // approved_for_upgrade is now reset back to false.
    let approved_for_upgrade = admin2
        .call(contract.id(), "upgrade")
        .max_gas()
        .args(UPGRADED_FIAT_TOKEN_NEW_NAME_3_APPROVALS_WASM.to_vec())
        .transact()
        .await
        .unwrap();
    assert!(approved_for_upgrade.is_failure());
}

#[tokio::test]
#[should_panic = "ExecutionEligibility(InsufficientApprovals { current: 2, required: 3 }"]
async fn test_upgrade_to_three_approvals() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;

    // We need 2/3 admins to be able to upgrade.
    let admin1 = &accounts[1];
    let admin2 = &accounts[2];
    upgrade_contract(
        contract.clone(),
        UPGRADED_FIAT_TOKEN_NEW_NAME_3_APPROVALS_WASM,
        admin1.clone(),
        admin2.clone(),
    )
    .await;

    // Try to approve the contract for upgrade again, but since the upgraded contract now requires
    // 3 approvals, this will panic.
    let approve_for_upgrade_args: Value = json!({
        "action": "ApproveForUpgrade"
    });
    do_multisig_action(
        admin1.clone(),
        admin2.clone(),
        contract.clone(),
        Some(approve_for_upgrade_args),
    )
    .await;
}

#[tokio::test]
#[should_panic = "Smart contract panicked: FiatToken: not approved for upgrade"]
async fn test_upgrade_not_approved_for_upgrade() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;

    let admin1 = &accounts[1];
    let upgraded_contract_vec: Vec<u8> = UPGRADED_FIAT_TOKEN_NEW_NAME_3_APPROVALS_WASM.to_vec();
    let upgraded_contract_b64: String = base64::encode(upgraded_contract_vec);
    let upgrade_args_json: Value = json!({ "code": upgraded_contract_b64 });

    // Attempt to upgrade without setting approved_for_upgrade.
    admin1
        .call(contract.id(), "upgrade")
        .max_gas()
        .args_json(upgrade_args_json)
        .transact()
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[should_panic = "Smart contract panicked: FiatToken: caller is not a Admin"]
async fn test_upgrade_not_admin() {
    let Setup { contract, accounts } = setup(0, FIAT_TOKEN_WASM).await;

    let blocklister = &accounts[4];
    let upgraded_contract_vec: Vec<u8> = UPGRADED_FIAT_TOKEN_NEW_NAME_3_APPROVALS_WASM.to_vec();
    let upgraded_contract_b64: String = base64::encode(upgraded_contract_vec);
    let upgrade_args_json: Value = json!({ "code": upgraded_contract_b64 });

    // Attempt to upgrade as a blocklister.
    blocklister
        .call(contract.id(), "upgrade")
        .max_gas()
        .args_json(upgrade_args_json)
        .transact()
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[should_panic = "ApprovalError(RequestExpired(RequestExpiredError))"]
async fn test_expired_request() {
    let Setup { contract, accounts } = setup(1, FIAT_TOKEN_WASM).await;

    // We need 2/3 admins to be able to upgrade.
    let admin1 = &accounts[1];
    let admin2 = &accounts[2];
    let random_account = &accounts[NUM_REQUIRED_ACCOUNTS];
    // Upgrade contract to have multi-sig requests with validity periods of 1 ns.
    upgrade_contract(
        contract.clone(),
        UPGRADED_FIAT_TOKEN_1NS_VALIDITY_PERIOD_WASM,
        admin1.clone(),
        admin2.clone(),
    )
    .await;

    let request_id: u32 = admin1
        .call(contract.id(), "create_multisig_request")
        .args_json(json!({
            "action": json!({
                "ConfigureMultisigRole": json!({
                    "role": "Admin",
                    "account_id": random_account.id(),
                })
            })
        }))
        .transact()
        .await
        .unwrap()
        .json::<u32>()
        .unwrap();

    // Attempt to approve an expired request.
    admin1
        .call(contract.id(), "approve_multisig_request")
        .args_json(json!({ "request_id": request_id }))
        .transact()
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_remove_expired_request() {
    let Setup { contract, accounts } = setup(2, FIAT_TOKEN_WASM).await;

    let admin1 = &accounts[1];
    let admin2 = &accounts[2];
    // Upgrade contract to have multi-sig requests with validity periods of 1 ns.
    upgrade_contract(
        contract.clone(),
        UPGRADED_FIAT_TOKEN_1NS_VALIDITY_PERIOD_WASM,
        admin1.clone(),
        admin2.clone(),
    )
    .await;

    let master_minter1 = &accounts[5];
    let controller = &accounts[NUM_REQUIRED_ACCOUNTS];
    let minter = &accounts[NUM_REQUIRED_ACCOUNTS + 1];

    let first_request_id: u32 = master_minter1
        .call(contract.id(), "create_multisig_request")
        .args_json(json!({
            "action": json!({
                "ConfigureController": json!({
                    "controller_id": controller.id(),
                    "minter_id": minter.id(),
                })
            })
        }))
        .transact()
        .await
        .unwrap()
        .json::<u32>()
        .unwrap();

    // Remove an expired request.
    master_minter1
        .call(contract.id(), "remove_multisig_request")
        .args_json(json!({ "request_id": first_request_id }))
        .transact()
        .await
        .unwrap()
        .unwrap();

    // Verify the next multisig request ID still incremented.
    let second_request_id: u32 = master_minter1
        .call(contract.id(), "create_multisig_request")
        .args_json(json!({
            "action": json!({
                "ConfigureController": json!({
                    "controller_id": controller.id(),
                    "minter_id": minter.id(),
                })
            })
        }))
        .transact()
        .await
        .unwrap()
        .json::<u32>()
        .unwrap();

    assert_eq!(first_request_id + 1, second_request_id);
}

// Helper function to configure controllers and to configure a minter's allowance.
async fn configure_minter_allowance(
    contract: Contract,
    master_minter1: Account,
    master_minter2: Account,
    controller1: Account,
    controller2: Account,
    minter: Account,
    minter_allowance: u128,
) {
    let configure_controller1_args: Value = json!({
        "action": json!({
            "ConfigureController": json!({
                "controller_id": controller1.id(),
                "minter_id": minter.id(),
            })
        })
    });
    let configure_controller2_args: Value = json!({
        "action": json!({
            "ConfigureController": json!({
                "controller_id": controller2.id(),
                "minter_id": minter.id(),
            })
        })
    });

    do_multisig_action(
        master_minter1.clone(),
        master_minter2.clone(),
        contract.clone(),
        Some(configure_controller1_args),
    )
    .await;
    do_multisig_action(
        master_minter1.clone(),
        master_minter2.clone(),
        contract.clone(),
        Some(configure_controller2_args),
    )
    .await;

    let minter_allowance_before: U128 = minter
        .call(contract.id(), "minter_allowance")
        .args_json(json!({ "minter_id": minter.id() }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(minter_allowance_before, U128::from(0));

    // Configure controller1's and controller2's (they control the same minter) minter's allowance.
    let configure_minter_allowance_args = json!({
        "action": json!({
            "ConfigureMinterAllowance": json!({
                "controller_id": controller1.id(),
                "minter_allowance": U128::from(minter_allowance),
            })
        })
    });
    do_multisig_action(
        controller1.clone(),
        controller2.clone(),
        contract.clone(),
        Some(configure_minter_allowance_args),
    )
    .await;

    // Verify minter status and allowance.
    let is_minter: bool = minter
        .call(contract.id(), "is_minter")
        .args_json(json!({ "account_id": minter.id() }))
        .transact()
        .await
        .unwrap()
        .json::<bool>()
        .unwrap();
    assert!(is_minter);

    let minter_allowance_after: U128 = minter
        .call(contract.id(), "minter_allowance")
        .args_json(json!({ "minter_id": minter.id() }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(minter_allowance_after, U128::from(minter_allowance));
}

async fn increase_minter_allowance(
    contract: Contract,
    controller1: Account,
    controller2: Account,
    increment: u128,
) {
    let increase_minter_allowance_args: Value = json!({
        "action": json!({
            "IncreaseMinterAllowance": json!({
                "controller_id": controller1.id(),
                "increment": U128::from(increment),
            })
        })
    });
    do_multisig_action(
        controller1.clone(),
        controller2.clone(),
        contract.clone(),
        Some(increase_minter_allowance_args),
    )
    .await
}

async fn decrease_minter_allowance(
    contract: Contract,
    controller1: Account,
    controller2: Account,
    decrement: u128,
) {
    let decrease_minter_allowance_args: Value = json!({
        "action": json!({
            "DecreaseMinterAllowance": json!({
                "controller_id": controller1.id(),
                "decrement": U128::from(decrement),
            })
        })
    });
    do_multisig_action(
        controller1.clone(),
        controller2.clone(),
        contract.clone(),
        Some(decrease_minter_allowance_args),
    )
    .await
}

async fn mint(contract: Contract, minter: Account, to: Account, mint_amount: u128) {
    // Register to/receiver with token to be able to receive mints.
    to.call(contract.id(), "storage_deposit")
        .args_json(json!({ "account_id": to.id() }))
        .deposit(ACCOUNT_STORAGE_COST)
        .transact()
        .await
        .unwrap()
        .unwrap();

    let is_registered: Option<StorageBalance> = to
        .call(contract.id(), "storage_balance_of")
        .args_json(json!({ "account_id": to.id() }))
        .transact()
        .await
        .unwrap()
        .json::<Option<StorageBalance>>()
        .unwrap();
    assert!(is_registered.is_some());

    // Mint to receiver/to.
    let mint_result = minter
        .call(contract.id(), "mint")
        .args_json(json!({ "to": to.id(), "amount": U128::from(mint_amount) }))
        .transact()
        .await
        .unwrap();
    // Sometimes we test mints that might fail.
    if mint_result.logs().len() > 0 {
        assert_eq!(
            mint_result.logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_mint\",\"data\":[{{\"owner_id\":{:?},\"amount\":\"{:?}\"}}]}}", to.id().as_str(), mint_amount)
        );
    }
    mint_result.unwrap();

    // Validate balance of receiver of mint.
    let receiver_balance: U128 = minter
        .call(contract.id(), "ft_balance_of")
        .args_json(json!({ "account_id": to.id() }))
        .transact()
        .await
        .unwrap()
        .json::<U128>()
        .unwrap();
    assert_eq!(receiver_balance, U128::from(mint_amount));
}

async fn burn(contract: Contract, burner: Account, burn_amount: u128) {
    let burn_result = burner
        .call(contract.id(), "burn")
        .args_json(json!({ "amount": U128::from(burn_amount) }))
        .transact()
        .await
        .unwrap();
    // Sometimes we test burns that might fail.
    if burn_result.logs().len() > 0 {
        assert_eq!(
            burn_result.logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"nep141\",\"version\":\"1.0.0\",\"event\":\"ft_burn\",\"data\":[{{\"owner_id\":{:?},\"amount\":\"{:?}\"}}]}}", burner.id().as_str(), burn_amount)
        );
    }
    burn_result.unwrap();
}

async fn approve(
    contract: Contract,
    token_holder: Account,
    spender: Account,
    approval_amount: u128,
) {
    // Approve, as the token holder, for another account (spender) to spend its tokens.
    let approve_result = token_holder
        .call(contract.id(), "approve")
        .args_json(json!({
            "spender_id": spender.id(),
            "value": U128::from(approval_amount),
        }))
        .transact()
        .await
        .unwrap();
    // Sometimes we test approving accounts that should fail.
    if approve_result.logs().len() > 0 {
        assert_eq!(
            approve_result.logs()[0],
            format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"approve\",\"data\":{{\"holder_id\":{:?},\"spender_id\":{:?},\"allowance\":\"{:?}\"}}}}", token_holder.id().as_str(), spender.id().as_str(), approval_amount)
        );
    }
    approve_result.unwrap();
}

async fn increase_allowance(
    contract: Contract,
    token_holder: Account,
    spender: Account,
    increment: u128,
) {
    token_holder
        .call(contract.id(), "increase_allowance")
        .args_json(json!({
            "spender_id": spender.id(),
            "increment": U128::from(increment)
        }))
        .transact()
        .await
        .unwrap()
        .unwrap();
}

async fn decrease_allowance(
    contract: Contract,
    token_holder: Account,
    spender: Account,
    decrement: u128,
) {
    token_holder
        .call(contract.id(), "decrease_allowance")
        .args_json(json!({
            "spender_id": spender.id(),
            "decrement": U128::from(decrement)
        }))
        .transact()
        .await
        .unwrap()
        .unwrap();
}

async fn blocklist(contract: Contract, blocklister: Account, account_to_blocklist: Account) {
    let blocklist_result = blocklister
        .call(contract.id(), "blocklist")
        .args_json(json!({
            "account_id": account_to_blocklist.id()
        }))
        .transact()
        .await
        .unwrap();
    assert_eq!(
        blocklist_result.logs()[0],
        format!("EVENT_JSON:{{\"standard\":\"x-fiat-token\",\"version\":\"1.0.0\",\"event\":\"blocklist\",\"data\":{{\"account_id\":{:?}}}}}", account_to_blocklist.id().as_str())
    );
    blocklist_result.unwrap();
}

async fn upgrade_contract(
    contract: Contract,
    upgraded_contract: &[u8],
    admin1: Account,
    admin2: Account,
) {
    // Approve contract for upgrade.
    let approve_for_upgrade_args: Value = json!({
        "action": "ApproveForUpgrade"
    });
    do_multisig_action(
        admin1.clone(),
        admin2.clone(),
        contract.clone(),
        Some(approve_for_upgrade_args),
    )
    .await;

    // The upgrade function takes in the base64 representation of a contract.
    let upgraded_contract_vec: Vec<u8> = upgraded_contract.to_vec();
    let upgraded_contract_b64: String = base64::encode(upgraded_contract_vec);
    let upgrade_args_json: Value = json!({ "code": upgraded_contract_b64 });

    // Do the actual upgrade.
    let upgrade_result = admin2
        .call(contract.id(), "upgrade")
        .max_gas()
        .args_json(upgrade_args_json)
        .transact()
        .await
        .unwrap();
    assert_eq!(
        upgrade_result.logs()[0],
        "Deserializing current contract..."
    );
    assert_eq!(upgrade_result.logs()[1], "Upgrading contract...");
    upgrade_result.unwrap();
}

/// Helper function to bundle the creation and first approval of multi-sig requests.
/// This is akin to bundling these two transactions into a vault run so that this multi-sig
/// process is more optimized.
async fn do_multisig_action(
    approver1: Account,
    approver2: Account,
    contract: Contract,
    args_json: Option<Value>,
) {
    let request_id: u32 = approver1
        .call(contract.id(), "create_multisig_request")
        .args_json(args_json)
        .transact()
        .await
        .unwrap()
        .json::<u32>()
        .unwrap();

    approver1
        .call(contract.id(), "approve_multisig_request")
        .args_json(json!({ "request_id": request_id }))
        .transact()
        .await
        .unwrap()
        .unwrap();

    approver2
        .call(contract.id(), "approve_multisig_request")
        .args_json(json!({ "request_id": request_id }))
        .transact()
        .await
        .unwrap()
        .unwrap();

    approver2
        .call(contract.id(), "execute_multisig_request")
        .args_json(json!({ "request_id": request_id }))
        .transact()
        .await
        .unwrap()
        .unwrap();
}
