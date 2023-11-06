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

use crate::role::Role;
use near_sdk::{
    borsh::{self, BorshDeserialize, BorshSerialize},
    env,
    json_types::U128,
    serde::{Deserialize, Serialize},
    AccountId,
};

/// Defines the types of accepted actions the [`ApprovalManager`]/multi-signature requests can accept.
/// If a multi-sig request is attempted to be created without an action that conforms to one of
/// these [`FiatTokenActions`], the request will fail with a Deserialization error.
#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum FiatTokenAction {
    ApproveForUpgrade,
    ConfigureController {
        controller_id: AccountId,
        minter_id: AccountId,
    },
    ConfigureMinterAllowance {
        controller_id: AccountId,
        minter_allowance: U128,
    },
    ConfigureMultisigRole {
        role: Role,
        account_id: AccountId,
    },
    DecreaseMinterAllowance {
        controller_id: AccountId,
        decrement: U128,
    },
    IncreaseMinterAllowance {
        controller_id: AccountId,
        increment: U128,
    },
    Pause,
    RemoveController {
        controller_id: AccountId,
    },
    RemoveMinter {
        controller_id: AccountId,
    },
    RevokeMultisigRole {
        role: Role,
        account_id: AccountId,
    },
    UpdateBlocklister {
        new_blocklister_id: AccountId,
    },
    Unpause,
}

/// Defines additional information and requirements for each [`FiatTokenAction`].
impl FiatTokenAction {
    /// Specifies the [`Role`]s required for each [`FiatTokenAction`].
    pub(crate) fn role_required(&self) -> Role {
        match self {
            FiatTokenAction::ConfigureMultisigRole { role, .. }
            | FiatTokenAction::RevokeMultisigRole { role, .. } => match role {
                Role::Admin => Role::Admin,
                _ => Role::Owner,
            },
            FiatTokenAction::ApproveForUpgrade => Role::Admin,
            FiatTokenAction::ConfigureController { .. }
            | FiatTokenAction::RemoveController { .. } => Role::MasterMinter,
            FiatTokenAction::ConfigureMinterAllowance { .. }
            | FiatTokenAction::DecreaseMinterAllowance { .. }
            | FiatTokenAction::IncreaseMinterAllowance { .. }
            | FiatTokenAction::RemoveMinter { .. } => Role::Controller,
            FiatTokenAction::Pause | FiatTokenAction::Unpause => Role::Pauser,
            FiatTokenAction::UpdateBlocklister { .. } => Role::Owner,
        }
    }

    /// Returns true if the [`FiatTokenAction`] requires verifying the controller.
    pub(crate) fn requires_controller_check(&self) -> bool {
        matches!(
            self,
            FiatTokenAction::ConfigureMinterAllowance { .. }
                | FiatTokenAction::DecreaseMinterAllowance { .. }
                | FiatTokenAction::IncreaseMinterAllowance { .. }
                | FiatTokenAction::RemoveMinter { .. }
        )
    }

    /// Returns the Controller associated with a Controller-related [`FiatTokenAction`] request.
    pub(crate) fn controller(&self) -> &AccountId {
        match self {
            FiatTokenAction::ConfigureMinterAllowance { controller_id, .. }
            | FiatTokenAction::DecreaseMinterAllowance { controller_id, .. }
            | FiatTokenAction::IncreaseMinterAllowance { controller_id, .. }
            | FiatTokenAction::RemoveMinter { controller_id, .. } => controller_id,
            _ => {
                env::panic_str(
                    "FiatToken: can only fetch controller for ConfigureMinterAllowance actions",
                );
            }
        }
    }
}
