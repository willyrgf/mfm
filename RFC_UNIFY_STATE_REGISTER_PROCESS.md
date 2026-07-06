# RFC: Unify State Registration Process

## Status

Proposed.

## Problem

The typed program API currently has two authority paths for planning states and operations:

1. Builder-registry lookup, where `ScopeBuilder` / `OperationExpansion` resolve the typed state or
   operation from their registry snapshot.
2. Direct registration-token planning, where callers pass `RegisteredState<S>` or
   `RegisteredOperation<O>` into `*_registered` APIs.

The second path is no longer needed. It makes graph authoring harder to reason about because a node
can be planned from a token minted outside the builder's registry snapshot. The token is not
forgeable, but it is still a separate planning authority. That weakens the simpler rule we now want:

> A state or operation can be planned only if it is registered in the builder registry.

This matters more after transition context enforcement. Every state-planning call now requires an
explicit transition context (`NoContext` or `&DeclaredContext<C>`). Keeping direct registered-token
APIs preserves an older capability-based planning model that no longer buys useful flexibility.

No current production operation has a use case for `*_registered` APIs or direct
`RegisteredState<S>` / `RegisteredOperation<O>` values.

## Decision

Remove the registered-token planning path completely.

All state and operation planning must resolve through the builder-owned registry snapshot. Breaking
changes are acceptable; no compatibility shim, fallback, or deprecated alias should be kept.

## Solution

### Program API

Delete the public registration token types and their private evidence types:

- `StateRegistrationEvidence<S>`
- `RegisteredState<S>`
- `OperationRegistrationEvidence<O>`
- `RegisteredOperation<O>`

Change registry builders so registration mutates the registry only:

- `StateRegistryBuilder::register<S>() -> Result<()>`
- `OperationRegistryBuilder::register<O>() -> Result<()>`

Remove registry snapshot methods that return tokens:

- `StateRegistry::registered_state<S>()`
- `OperationRegistry::registered_operation<O>()`

Keep descriptor resolution as internal or explicitly descriptor-named APIs, not token-named APIs.
For example:

- `StateRegistrySnapshot::state_descriptor<S>() -> Result<StateDescriptorIdentity>`
- `OperationRegistrySnapshot::operation_descriptor<O>() -> Result<OperationDescriptorIdentity>`

If descriptor helpers are needed outside registry snapshots, name them by what they return:

- `state_descriptor::<S>()`
- `operation_descriptor::<O>()`

Do not keep names like `registered_state_descriptor`; they preserve the old mental model.

### Planning API

Delete all direct registered-token planning APIs:

- `ScopeBuilder::state_registered`
- `ScopeBuilder::state_registered_with_domain_keys`
- `ScopeBuilder::side_effect_registered`
- `ScopeBuilder::call_registered`
- `OperationExpansion::state_registered`
- `OperationExpansion::state_registered_with_domain_keys`
- `OperationExpansion::side_effect_registered`
- `OperationExpansion::call_registered`

The remaining planning APIs are:

- `state`
- `state_with_domain_keys`
- `side_effect`
- `side_effect_with_compensation`
- `call`

Each state-planning API continues to require:

```rust
context: impl StateTransitionContext<'program, 'scope, S::Context>
```

Internally, these methods resolve `S` or `O` through the builder's registry snapshot, derive the
validated descriptor, and then plan the node or operation lineage frame.

### Certification API

Change certification registration to resolve by type instead of accepting token references:

- `CertificationRegistry::register_state::<S>()`
- `CertificationRegistry::register_operation::<O>()`

Delete helpers whose only purpose is lowering token-backed descriptors:

- `state_descriptor_identity_from_registered`
- `operation_descriptor_identity_from_registered`

Update `define_program_descriptor_registry!` so it registers state and operation descriptors by type
without minting `registered` locals.

### Runtime And Adapter Descriptor Usage

Replace all `registered_state_descriptor::<S>()` uses with a non-token descriptor helper, or resolve
from an explicit registry snapshot where registry membership is the point of the test.

Known current call sites:

- `crates/transports/proof/src/lib.rs`
- `crates/states/btc/src/lib.rs`
- `crates/states/evm-contracts/src/lib.rs`
- `crates/adapters/evm-contracts/src/lib.rs`
- `crates/kernel/runtime/src/runner_kit.rs`
- selected runtime and adapter tests

Replace operation descriptor lookup in `crates/ops/btc-chain-head-collector-op/tests/`.

## Tests To Remove

Delete UI tests that exist only to prove token/evidence fields cannot be forged:

- `crates/kernel/program/tests/ui/fail/forged_registered_state.rs`
- `crates/kernel/program/tests/ui/fail/forged_registered_state.stderr`
- `crates/kernel/program/tests/ui/fail/forged_registered_operation.rs`
- `crates/kernel/program/tests/ui/fail/forged_registered_operation.stderr`
- `crates/kernel/program/tests/ui/fail/forged_operation_registration_evidence.rs`
- `crates/kernel/program/tests/ui/fail/forged_operation_registration_evidence.stderr`

Delete unit tests for direct token planning:

- `explicit_registered_state_token_plans_without_builder_registry`
- `explicit_registered_operation_token_calls_without_builder_registry`

Update, but do not delete, tests that verify registry membership still gates planning:

- registered state registry plans a state node
- registered operation registry records lineage
- unregistered state cannot be planned
- unregistered operation cannot be called

## Documentation Updates

Update `crates/kernel/program/README.md` and rustdoc comments to remove `call_registered` and all
direct registered-token language.

The documented rule should be:

> Operation implementations compose registered states and nested operations through the builder
> registry only. Registration is a property of the builder registry, not a transferable planning
> token.

## Suggested Commit Plan

1. Remove registered state planning tokens and state `*_registered` APIs.
2. Remove registered operation planning tokens and `call_registered`.
3. Simplify certification registration to register by type.
4. Rename descriptor helpers and update runtime, adapter, state, transport, and test call sites.
5. Delete token-forgery UI tests and refresh any remaining trybuild stderr.

Each commit should compile independently when practical. If an intermediate commit must be larger,
prefer one coherent API transition over temporary compatibility shims.

## Verification

Focused checks during the change:

```bash
cargo test -p mfm-program
cargo test -p mfm-certify
cargo test -p mfm-runtime
cargo check --workspace
```

Final pre-merge checks remain the repository gates:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```
