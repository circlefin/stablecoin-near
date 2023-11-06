# `near-usdc` - Contract Upgrade flow

This document describes how to upgrade the USDC fiat_token smart contract on the NEAR Protocol network.\
The testnet contract account is [3e2210e1184b45b64c8a434c0a7e7b23cc04ea7eb7a6c3c32520d03d4afcb8af](https://testnet.nearblocks.io/token/3e2210e1184b45b64c8a434c0a7e7b23cc04ea7eb7a6c3c32520d03d4afcb8af).\
The mainnet contract account is [17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1](https://nearblocks.io/token/17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1).\
<br>The upgrade pattern for contracts on NEAR is best described as a [self-update and state migrate pattern](https://docs.near.org/tutorials/examples/update-contract-migrate-state). 
A contract can be upgraded by calling a function on itself that passes in the new contract `.wasm` and migrates its state (and `struct` if needed) on-chain.

## 1. Approving for Upgrade

Upgrading can only be performed by those with the `Admin` Role. To facilitate this,
there is an `approved_for_upgrade` property on the contract that must be set to `true`
by an `Admin`.

The first step to upgrading is to set this property. Create and execute a multi-sig request
(refer to the [multi-sig doc](01_Multi_Sig.md) for more details) for the `ApproveForUpgrade` action with an `Admin`.

## 2. Constructing the new contract

The next version of the contract must be able to be deserialized from the current `struct` to the new `struct`, while manually
transferring all data from the old to the new.
This is covered in the [State Migration](https://docs.near.org/tutorials/examples/update-contract-migrate-state#state-migration) NEAR docs.


**Update as of (4/10/2023)**
> Adding Role enums (`src/role.rs`) must be added to the very bottom of the list, otherwise the `Rbac` library will lose
the mapping between accounts and the roles they had. At the time of writing this seems to be a behavior
that hasn't been patched yet.

For example, say we had a contract:

```rust
pub struct OldContract {
    token: FungibleToken,
    owner: AccountId,
    paused: bool,
}
...
```

and we wanted to upgrade the contract to have a new `admin` field:

```rust
pub struct NewContract {
    token: FungibleToken,
    owner: AccountId,
    paused: bool,
    admin: AccountId, // New field.
}
...
```

We would need to deserialize OldContract into NewContract (Self) and then return it:

```rust
#[derive(BorshDeserialize, BorshSerialize)]
struct OldContract {
   token: FungibleToken,
   owner: AccountId,
   paused: bool,
}

let old: OldContract = env::state_read().expect("Contract should be initialized");

Self { // NewContract
   token: old.token,
   owner: old.owner,
   paused: old.paused,
   admin: "admin", // New field.
};
```

**Note**: we are also setting the values of the new struct to the old structs respective properties.
Remember, if you have other fields or data you want to keep, you must retrieve it from the old contract and migrate it to the new contract!
 
Finally, this logic needs to live in a `migrate() -> Self` function that is annotated by `#[init(ignore_state)]` and 
`#[private]`:

```rust
 /// Should only be called by this contract on migration.
 /// This method is called from the upgrade() method.
 /// For next version upgrades, change this function, and remember to set
 /// [`approved_for_upgrade`] to false.
 #[init(ignore_state)]
 #[private]
 pub fn migrate() -> Self {
     env::log_str("Deserializing current contract...");
     #[derive(BorshDeserialize, BorshSerialize)]
     struct PrevContract {
         token: FungibleToken,
         metadata: LazyOption<FungibleTokenMetadata>,
         accounts: UnorderedSet<AccountId>,
         allowed: UnorderedMap<AccountId, UnorderedMap<AccountId, U128>>,
         ...
        approved_for_upgrade: bool,
     }
     let prev: PrevContract = env::state_read().expect("Contract should be initialized");
     env::log_str("Upgrading contract...");
     
     // Upgrade anything else not specified in the struct.

     Self {
         token: prev.token,
         metadata: prev.metadata,
         allowed: prev.allowed,
         ...
         approved_for_upgrade: false, // Need to reset this to false!
     };
 }
```

This function will only be called by the contract itself (hence `#[private]`) from an `upgrade()` function. 
After you have specified what the migration will look like, we can move onto the actual upgrade. 

## 3. Upgrading to the new contract

Performing the contract upgrade is relatively straightforward: call an upgrade function on the contract and pass in the upgraded contract.

In the fiat_token implementation, we use NEAR's [Upgrade derive macro](https://docs.rs/near-sdk-contract-tools/latest/near_sdk_contract_tools/derive.Upgrade.html), 
which, when the contract is annotated with `#[upgrade]`, exposes an `upgrade(code: Vec<u8>)` function that takes in the new contract's
WASM/code in Base64. Then, once the upgrade function is called, there is a default [UpgradeHook](https://github.com/NEARFoundation/near-sdk-contract-tools/blob/2bc7afbcc3d66962bfda5a957b5444d855edd228/src/upgrade/serialized.rs#L13)
that is triggered, and then the `migrate()` function (configured [here](https://github.com/NEARFoundation/near-sdk-contract-tools/blob/2bc7afbcc3d66962bfda5a957b5444d855edd228/src/upgrade/mod.rs#L33))
is called to complete the contract upgrade. For more information, see the implementation [here](https://github.com/NEARFoundation/near-sdk-contract-tools/blob/2bc7afbcc3d66962bfda5a957b5444d855edd228/src/upgrade/serialized.rs#L17).
### Note 
> Please make sure the upgrade is performed with exhaustive testing. At the time of writing (4/10/2023), there isn't functionality to restore the previous state.

Since we also want to only allow `Admin`s to upgrade the contract, we can add a `require` statement for that in the `UpgradeHook`.
We will also add a check for the `approved_for_upgrade` field.

This is then what the function would look like:
```rust
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
```
As explained above, if `Contract` is annotated with `#[upgrade]` (and we `#[derive(...Upgrade)]` on the struct), then we
have already exposed upgradeability for this contract. We then need to explicitly specify an `UpgradeHook`, which could be
empty, but in our case we will add some Admin permission control.
