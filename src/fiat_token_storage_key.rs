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
    borsh::{self, BorshSerialize},
    AccountId,
};
/// Defines a set of [`StorageKey`]s for [`UnorderedSet`]'s and [`UnorderedMap`]'s prefixes.
/// It is used to namespace the collections in the NEAR VM and prevent collisions in this contract.
#[derive(Debug, Clone, BorshSerialize, near_sdk::BorshStorageKey)]
pub(crate) enum FiatTokenStorageKey {
    // StorageKey for a temporary UnorderedMap to map a spender_id to its allowed spending amount.
    // This is the nested UnorderedMap value inside Allowed, mapping to the key: the holder_id.
    Allowance {
        holder_id: AccountId,
        spender_id: AccountId,
    },
    Allowed,
    Controllers,
    FungibleToken,
    Metadata,
    MinterAllowed,
}
