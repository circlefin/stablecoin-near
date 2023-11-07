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

use near_sdk::{
    borsh::{self, BorshDeserialize, BorshSerialize},
    serde::{Deserialize, Serialize},
    BorshStorageKey,
};
use std::fmt::Display;

/// Defines roles that can interact with the multi-signature scheme implemented through
/// `#[simple_multisig]`. When adding new roles, *make sure they are added to the bottom of the
/// enum list*, otherwise when the contract is being migrated other accounts will lose their roles.
#[derive(
    Clone, Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, BorshStorageKey,
)]
pub enum Role {
    Multisig,
    Admin,
    Blocklister, // Non-multisig
    Controller,
    MasterMinter,
    Minter, // Non-multisig
    Owner,
    Pauser,
    Blocklisted, // This was added after deployment, so it has to be at the bottom.
}

impl Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Role::Multisig => "Multisig",
                Role::Admin => "Admin",
                Role::Blocklister => "Blocklister",
                Role::Controller => "Controller",
                Role::MasterMinter => "MasterMinter",
                Role::Minter => "Minter",
                Role::Owner => "Owner",
                Role::Pauser => "Pauser",
                Role::Blocklisted => "Blocklisted",
            }
        )
    }
}
