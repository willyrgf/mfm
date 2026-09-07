# Accumulating context: engineer handoff evidence

This document records pre-cutover prototype evidence for [RFC_CTX_ACC.md](../RFC_CTX_ACC.md).
The production test mapping and measurements are in [implementation evidence](rfc-ctx-acc-implementation.md). The RFC is the target; the isolated
prototype is feasibility evidence, not an alternate supported implementation. Implement the three
ordered commits in RFC section 12 and meet all acceptance checks in section 11.

## Validation environment and boundary

The spike was based on commit `cac42fb4e0a79091fb589b606649a9e4b990616f`, using the default Nix
shell and dependencies resolved offline from the repository lockfile. It copied first-party EVM,
Program, and Runtime crates into `/tmp/mfm-ctx-acc-spike.NnojXr`, added a small consuming crate and
proc macro, and used the repository's actual Values, capabilities, Journal, and MemoryStore.
Production source files were not changed by this validation.

The consuming workflow creates A, creates B, calls B, observes that call's anchored target, and
decodes a report. Each transaction expands to the real three Effects plus outcome projection.
Adapters return deterministic, correctly bound public evidence. Tests call Runtime's public
start/read/resume surfaces; they do not manually drive the transaction callbacks as a replacement
for runtime execution. Separate negative tests exercise callback error boundaries directly and
through Runtime.

This proves compilation, association, execution, and qualified cold reload with those adapters.
It does not prove PostgreSQL crash recovery, live provider behavior, signer/custody behavior, or the
complete production migration. Existing managed and focused recovery tests remain mandatory.

## Exact mechanical contract and bounds

The compiled primitive is:

```rust
pub trait ContextSlot<C: MfmValue> {
    type Value: MfmValue;
    type With<V: MfmValue>: MfmValue;
    fn get(context: &C) -> &Self::Value;
    fn replace<V: MfmValue>(context: C, value: V) -> Self::With<V>;
    fn slot_id() -> mfm_values::Result<StableId>;
}
```

Rust does not infer that replacing an already-replaced slot produces the same type as replacing
that slot in the initial record. The reusable library owns explicit associated-type equalities.
With `R: TransactionRecipe<C>`, the aliases are:

```rust
pub type Replaced<C, R, V> =
    <<R as TransactionRecipe<C>>::Slot as ContextSlot<C>>::With<V>;
pub type ReservedContext<C, R> = Replaced<C, R, ReservedEvmTransaction>;
pub type PreparedContext<C, R> = Replaced<C, R, PreparedTransactionFacts>;
pub type ExecutedContext<C, R> = Replaced<C, R, ExecutedTransactionFacts>;
pub type CompletedContext<C, R> =
    Replaced<C, R, CompletedTransactionFacts<<R as TransactionRecipe<C>>::Success>>;
```

The combined bounds required by the Operation and registration helper are:

```rust
R::Slot:
    ContextSlot<ReservedContext<C, R>,
        Value = ReservedEvmTransaction,
        With<PreparedTransactionFacts> = PreparedContext<C, R>>
    + ContextSlot<PreparedContext<C, R>,
        Value = PreparedTransactionFacts,
        With<ExecutedTransactionFacts> = ExecutedContext<C, R>>
    + ContextSlot<ExecutedContext<C, R>,
        Value = ExecutedTransactionFacts,
        With<CompletedTransactionFacts<R::Success>> = CompletedContext<C, R>>
```

Each State implementation carries only the subset it uses. These bounds compiled without a
recursive context-family trait, extra user forwarding implementations, or context-wide `Clone`.
The prototype uses shorter names (`TxRecipe`, `PreparedFacts`, `Transaction`, etc.); the names
above and below specify their production equivalents.

The derive accepts named structs, distinct bare slot parameters, and fixed sibling fields. Reject
bounds/defaults, `where` clauses, lifetimes/const parameters, tuple structs, nested slot parameters,
and repeated use of a slot parameter. Require `#[context(namespace = "...")]`. Generate markers
as `<Struct><FieldInPascalCase>Slot` with the struct's visibility and identities from namespace plus
field name. Use structural `syn` traversal for unsupported generic occurrences in production;
the spike's conservative token scan is not the intended public diagnostic implementation.

## Recipe and authoring APIs

Use `TransactionRecipe<C>: Send + Sync + 'static` with associated `Slot: ContextSlot<C>` and
`Success: TransactionSuccessMode`. Its methods are:

```rust
fn command(context: &C) -> Eip1559TransactionCommand;
fn recipe_id() -> mfm_values::Result<StableId>;
fn source_ids() -> mfm_values::Result<Vec<StableId>>;
```

`source_ids` is ordered: destination first, then source dependencies. `TransactionSuccessMode`
is sealed to `Created` and `Called`; it owns the action discriminator and checked projection from
executed facts. Projection returns `Result<Self, StateExecutionError>`. It never supplies a
fabricated success for another mode. No new mode-specific capability/evidence family is needed.

`ObservationRecipe<C>` has `Slot: ContextSlot<C>`, the same two
identity methods, and:

```rust
fn intent(context: &C) -> Result<AnchoredContractCallIntent, PreparationError>;
```

Checked observation-plan construction validates its own route/calldata. `ObserveAt` must also
compare that route/chain with the selected completed transaction before provider entry. A valid
plan alone cannot prove this cross-field relationship. A mismatch is an internal preparation
error, not external rejection.

Production entry points are `EvmTransaction<C, R>::new(binding)` implementing `Operation`, and
`register_evm_transaction_states::<C, R>(&mut RuntimeAssemblyBuilder) -> mfm_runtime::Result<()>`.
The Operation belongs to `mfm-evm`; the helper belongs to `mfm-evm-live`. It registers the exact four
State ABIs and does not bind adapters. The prototype colocates the helper with its Operation for
convenience; do not introduce a domain-to-Runtime dependency during implementation.

A consumer uses one alias per meaningful operation boundary, for example:

```rust
#[derive(Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.example.workflow")]
struct Workflow<A, B, C, O> {
    a: A,
    b: B,
    configuration: C,
    observation: O,
    unrelated: u64,
}

type Initial = Workflow<CheckedCreatePlan, CheckedCreatePlan,
    CheckedCallPlan, CheckedObservationPlan>;
type DeployA = EvmTransaction<Initial, CreateAt<WorkflowASlot>>;
type AfterA = <DeployA as Operation>::Output;
type DeployB = EvmTransaction<AfterA, CreateAt<WorkflowBSlot>>;
type AfterB = <DeployB as Operation>::Output;
type Configure = EvmTransaction<AfterB,
    CallCreatedAt<WorkflowConfigurationSlot, WorkflowBSlot>>;
type AfterCall = <Configure as Operation>::Output;
type ObserveConfigured = ReadAnchoredContractCall<AfterCall,
    ObserveAt<WorkflowObservationSlot, WorkflowConfigurationSlot>>;
type Final = <ObserveConfigured as State>::Output;
```

The actual consuming test uses this pattern with the spike's short names. Product expansion uses
existing `body.operation`, `body.read`, and `with_failure_handler`. Product code still chooses
connections, maps each meaningful operation failure to its root policy, registers logical
operations, and owns ABI/report semantics. No user aliases, State implementations, or registration
lists are needed for each internal transaction stage. A second `Single<T>` context with a fixed
sibling also executes and reloads using the same library State family.

Executable identity must hash a canonical domain-separated descriptor containing implementation
version, stage, recipe identity, ordered selected slots, and outcome mode. The spike proves that
changing source A to B changes command and State identity; its identity helper does not yet
include the explicit custom-recipe identity. Implement that addition and test incompatible
assembly rejection before IO. Exact value schemas remain additional association keys.

## Checked plans and cumulative facts

Both `CheckedCallPlan::new` and `CheckedCreatePlan::new` accept, in order, binding, input bytes,
value, nonzero gas limit, priority fee, and maximum fee. They return `Result<Self, EvmDomainError>`.
Use one shared validation owner with the complete command factories. `CheckedCallPlan` has no
target; `command_for(&self, EvmAddress)` is infallible. `CheckedCreatePlan::command(&self)` is
infallible. `CheckedTargetCallPlan::new(CheckedCallPlan, EvmAddress)` and its `command()` are
infallible. No dummy address is needed to validate a call plan.

`CheckedObservationPlan::new(EvmTransactionRoute, Vec<u8>)` checks and stores chain, route reference,
and bounded calldata; `intent_for(target, anchor)` is infallible. The recipe still checks the
cross-field relationship described above. Deserialization cannot bypass plan validation.

Validated boundaries include the existing 49,152-byte initcode and 131,072-byte calldata maxima,
`u128` fee ceiling, priority-fee order, and rejection of a call disguised as a creation plan.
The nonzero gas type excludes zero. Tests compare late-target/anchor results with the current
complete checked constructors.

Use the nested transaction-local records in RFC section 4, with private fields and borrowing
accessors. Preserve command/reference, nonce-domain, preparation-hash, settlement-nonce/hash, and
action consistency in constructors and checked decoding. Validate intrinsic relationships there;
Effect IDs' relationship to the particular run/declaration remains Runtime/Journal qualification.
The spike retains the complete evidence but uses public aggregate fields for the new fact records;
its derives are not production-ready hostile-input decoders. Implement and test those boundaries
before deleting the current validation owners.

## Internal-error contract

Add the redaction-safe unit error `mfm_program::StateExecutionError`. Change Pure `evaluate` and
Read/Effect `interpret` to return:

```rust
Result<ProposedStateOutcome<Self::Output, Self::Failure>, StateExecutionError>
```

Keep their existing arguments and keep `prepare` returning its existing `PreparationError` result.
The new error means the trusted local implementation cannot produce a valid conclusion. Runtime
maps it to `RuntimeError::Internal` before qualification/append of a conclusion. Durable domain
failures remain `Ok(ProposedStateOutcome::Failure { ... })`.

The real Runtime test proved: Pure and Read errors leave genesis as the head; an Effect
interpretation error leaves its prepare head; a later successful retry uses the same command
reference and EffectId; terminal cold read/resume preserves the head and never reruns the failing
interpreter. The EVM projection test rejects an incompatible action as internal error. This
requires the complete callback cutover listed in the RFC, including Portfolio and kernel fixtures.

## Terminal failure representation and capacity

The first root failure design embedded each full context in a separate enum variant. Real
assembly rejected its descriptor because it exceeded the 65,536-byte schema-identity limit.
Increasing value capacity would not solve this independent limit.

Keep exact accumulated contexts inside execution and operation failure envelopes. At the terminal
report boundary, use a finite named entry sequence. The validated example has exactly four entries
in A/B/configuration/observation order. Its entry-data sum contains each of these schemas once:

- Unexecuted checked creation, call, or observation plan.
- Executed transaction facts plus created-address, call-target, or reverted outcome.
- Anchored observation intent and accepted evidence.

A finite failure reason identifies transaction A/B/configuration reversion, observation failure,
or invalid report bytes. Checked construction/decoding enforce entry count/order and agreement
between failure point, planned versus executed entries, and outcome evidence. A reason for an
unexecuted or successful step is invalid. Preserve the existing closed observation failure reason
in production; the spike exercises rejection, and the other accepted evidence variants require
production tests. Fixed request metadata is retained once. Commands preserve the original plan
parameters for executed steps; untouched plans preserve them for unexecuted steps.

Measured schema-identity byte lengths:

| Exact value | Bytes |
| --- | ---: |
| Initial context | 19,543 |
| After creation A | 29,464 |
| After creation B | 39,385 |
| After configuration | 50,840 |
| After observation | 56,890 |
| Successful report | 57,345 |
| Observation failure envelope | 57,962 |
| Normalized root failure report | 39,029 |

Adding another full copy of all original plans as shared request metadata would erase much of
that headroom. Do not do it. More transactions can exhaust schema capacity even when payloads are
small; this design makes no arbitrary-workflow-length guarantee.

Successful runs have 24 frames: genesis, three seven-frame transactions, observation, and report.
Payload sizes below are per creation and per call/observation input. Return bytes are 32 bytes.
All numbers are bytes measured from actual canonical terminal values and encoded Journal frames:

| Creation input | Call / observation input | Terminal value | Total frame bytes | Largest frame |
| ---: | ---: | ---: | ---: | ---: |
| 3 | 3 | 7,095 | 146,355 | 35,538 |
| 1,024 | 1,024 | 12,543 | 241,695 | 40,986 |
| 16,384 | 16,384 | 94,463 | 1,675,295 | 122,906 |
| 49,152 | 49,152 | 269,223 | 4,733,595 | 337,506 |
| 49,152 | 131,072 | 487,677 | 8,447,313 | 665,187 |

These runs pass the unchanged object/frame/run limits. This is input-size evidence, not a maximum
return-evidence or long-chain capacity proof. Production capacity acceptance also requires failure
frame measurements and maximal supported return evidence. Failure schemas and all five failure
paths were exercised at small payloads; their maximal frame sizes were not measured in this spike.

## Commands and results

Run from the repository root. The temporary manifest path identifies this session's source;
recreate the spike or implement the specified APIs if that directory is no longer available.

```bash
nix develop -c cargo test --manifest-path /tmp/mfm-ctx-acc-spike.NnojXr/Cargo.toml \
  -p mfm-evm --test checked_plans --offline
nix develop -c cargo test --manifest-path /tmp/mfm-ctx-acc-spike.NnojXr/Cargo.toml \
  -p ctx-spike --lib --test callback_errors --offline -- --nocapture
nix develop -c cargo test --manifest-path /tmp/mfm-ctx-acc-spike.NnojXr/Cargo.toml \
  -p ctx-spike --test compile_fail --offline
```

Results: three checked-plan tests passed; six consuming/runtime/size tests passed; one callback
error-boundary test passed; seven trybuild cases passed in one test with previously generated
snapshots checked without overwrite. The six consuming tests completed in 203.57 seconds; this
is not a performance benchmark. The missing-producer diagnostic correctly reports that the
selected slot contains `CheckedCreatePlan` where `CompletedFacts<Created>` is required.

The original prototype handoff commit was documentation only. Its verification was local link and command review plus
`git diff --check`. This historical note claims no production CI result. Production acceptance uses the
RFC's scope-driven gates and final CI, not treat this spike as a substitute.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Production constructors, identities, derive diagnostics, and all consumers preserve the specified contracts. | The spike intentionally implements only the bounded feasibility surface; deviations are identified above. | Implementation could retain unchecked facts, collide custom identities, or leave stale callers. | Implement the specified boundaries and perform the RFC's hostile-input, compile-fail, identity, deletion, and recovery checks. |
| Each intended product workflow fits the fixed capacity envelope. | The measured graph and input sizes do not cover maximal returns, failure frame sizes, or longer chains. | Supported compositions could fail admission despite small individual commands. | Add capacity regressions at the actual product boundary, including failure/return maxima, before declaring the production cutover complete. |
