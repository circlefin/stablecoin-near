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

use crate::fiat_token::Contract;
use crate::role::Role;

use near_sdk::{env, require, AccountId};
use near_sdk_contract_tools::rbac::Rbac;

pub(crate) fn require_not_blocklisted(account_id: &AccountId) {
    require!(
        !<Contract as Rbac>::has_role(account_id, &Role::Blocklisted),
        format!("FiatToken: {account_id} is blocklisted")
    )
}

/// Throws if called by any account that does not have the specified [`Role`].
pub(crate) fn require_only(role: Role) {
    require!(
        <Contract as Rbac>::has_role(&env::predecessor_account_id(), &role),
        format!("FiatToken: caller is not a {role}")
    );
}
