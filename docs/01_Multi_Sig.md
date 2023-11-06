# `near-usdc` - USDC on NEAR multi-sig

This document describes how transactions requiring multiple signatures (i.e. cold storage transactions) work on NEAR.
It should be noted that transactions on the NEAR protocol do not natively support more than 1 signer.

## 1. Traditional multi-sig.

In traditional multi-signature schemes, a signed transaction's bytes is made up of the transaction itself and values formed 
by a list of signers--usually their signatures. For example, [Solana](https://docs.solana.com/developing/programming-model/transactions#signatures) natively supports multiple signatures for a transaction.
However, that alone does not enforce what we think of multi-sig. We then need a manager of sorts to only allow a transaction
to be executed if it reaches a threshold of signatures.

Some protocol-level multi-sig schemes default to 2-of-3 multi-sig, that is, a transaction can only be successfully broadcasted
when signed by 2/3 "approved" signers. Some use a smart contract to deal with this. On NEAR we use something called
an ApprovalManager and multi-sig requests.

## 2. Multi-sig on NEAR.

### ApprovalManager
In order to simulate this m-of-n multi-sig, we use NEAR's [ApprovalManager](https://github.com/NEARFoundation/near-sdk-contract-tools/blob/2bc7afbcc3d66962bfda5a957b5444d855edd228/src/approval/mod.rs#L159).
In summary, it follows a request-based approval system where they are created, approved, and then executed. The number of
approvals required is configured in ApprovalConfiguration. How we represent this in the FiatToken contract is through 
the `create_multisig_request`, `approve_multisig_request`, `execute_multisig_request` functions and
[Rbac](https://github.com/NEARFoundation/near-sdk-contract-tools/blob/2bc7afbcc3d66962bfda5a957b5444d855edd228/src/rbac.rs) (Role-Based access control).
The multisig requests will be referred to via `request_id`s, which is outputted when we create a new multisig request.
At any point in time we can also query the ApprovalManager to figure out the next multisig `request_id`.

### Roles
For permissioned functions that require multiple approvers/cold-storage we only want certain accounts to be able to create,
approve, or execute (multi-sig) requests tied to those functions. In order to do this, we assign Roles (Admin,
MasterMinter, etc.) to accounts, and ensure only a specific role can work with a specific function via the
`FiatTokenAction` enum (through `role_required`).

### FiatTokenAction
These are current hard-coded enums for functions that require multi-sig. They do not implement logic, but are rather
inputs into the `create_multisig_request` function so that we can't create a request to do something unexpected.

### Full flow
With these three components, multi-sig functions on NEAR's FiatToken will look like this:
1. An account with the correct role calls `create_multisig_request`, passing in, for example, the `ApproveForUpgrade` `FiatTokenAction` (requires the `Admin` role).
2. Two different accounts with the Admin roles will now need to call `approve_multisig_request` with the previously created `request_id`. There are also checks that makes sure roles can only approve requests that require the same role, i.e. `Admin`s can only approve requests with `FiatTokenAction`s requiring `Admin`s.   
3. Finally, any `Admin` can then `execute_multisig_request`, and the actual function mapped by the `FiatTokenAction` will be called.


