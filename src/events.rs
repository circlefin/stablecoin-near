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

pub mod fiat_token_event {
    use crate::role::Role;
    use near_sdk::json_types::U128;
    use near_sdk::AccountId;
    use near_sdk_contract_tools::event;

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when approving a spender account to spend an allowance from a holder's account.
    pub struct Approve {
        pub holder_id: AccountId,
        pub spender_id: AccountId,
        pub allowance: U128,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when contract is approved for upgrade.
    pub struct ApprovedForUpgrade;

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when account ID is blocklisted.
    pub struct Blocklist {
        pub account_id: AccountId,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when blocklister account ID is changed
    pub struct BlocklisterChanged {
        pub new_blocklister_id: AccountId,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when a controller is configured with a minter.
    pub struct ControllerConfigured {
        pub controller_id: AccountId,
        pub minter_id: AccountId,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when a controller is disabled.
    pub struct ControllerRemoved {
        pub controller_id: AccountId,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when minter account ID is configured.
    pub struct MinterConfigured {
        pub minter_id: AccountId,
        pub minter_allowance: U128,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when minter account ID is removed.
    pub struct MinterRemoved {
        pub minter_id: AccountId,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when a multisignature transaction is created.
    pub struct MultisigRequestCreated {
        pub request_id: u32,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when contract is paused.
    pub struct Paused;

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when an account is configured as one of the contract's main multi-sig roles, e.g.
    /// Admin, MasterMinter, etc.
    pub struct RoleConfigured {
        pub role: Role,
        pub account_id: AccountId,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when one of the contract's main multi-sig roles, e.g. Admin, MasterMinter, etc.
    /// is revoked from their role.
    pub struct RoleRevoked {
        pub role: Role,
        pub account_id: AccountId,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when account ID is unblocklisted.
    pub struct Unblocklist {
        pub account_id: AccountId,
    }

    #[event(standard = "x-fiat-token", version = "1.0.0", rename = "snake_case")]
    /// Emitted when contract is unpaused.
    pub struct Unpaused;
}
