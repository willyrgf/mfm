# RFC: accumulating typed contexts

Status: proposed. This RFC records the agreed direction and the concrete implementation scope;
it does not change the current executable contracts. Implementation must update
[design](docs/design.md) and [architecture](docs/architecture.md) in the same cutover as the code.

## Decision

Use ordinary typed records whose named fields accumulate public execution results. Each State
receives a complete immutable context snapshot, reads its declared typed requirements, and
returns a new snapshot containing its result and the preceding facts. Copying accumulated values
between persisted snapshots is an accepted cost.

Include internal transaction stages: the complete nonce-free command, reservation descriptor,
preparation evidence, settlement evidence, and projected outcome. Later States and the final
Report can inspect those facts without querying Journal or forwarding them manually through
intermediary States.

The mechanical operation is small: borrow one typed field, or replace that field while preserving
all siblings. A narrow derive generates this once per context definition. Reusable domain States
perform the domain work and use that operation to construct the next snapshot.

For the contract fixture, delete `PrepareConfiguration` and `PrepareObservation`. Build the
configuration command in the first reservation Effect's `prepare()` method. Build the anchored
observation intent in the existing Read's `prepare()` method. Keep the four transaction States,
which represent distinct durable responsibilities.

## Problem and scope

[PROBLEM_COMPOSITION_TYPED_PLUMBING.md](PROBLEM_COMPOSITION_TYPED_PLUMBING.md) inventories the wider
composition problem. This RFC addresses accumulated information, context forwarding, typed access,
and the two EVM preparation bridges. It specifies a reusable value mechanism and a complete cutover
of the transaction/anchored-call consumers that demonstrate the problem.

It does not redesign the scheduler, Program graph, heterogeneous run inspection, retry policy,
all failure authoring, or generic assembly registration. It does not admit development transaction
Effects into production CLI/REST composition. Portfolio's balance/aggregation context remains its
own supported domain API; changing that workflow is not necessary to replace the EVM transaction
and anchored-call context contracts.

This scope avoids introducing a general query language, runtime field registry, dynamic value bag,
automatic dependency scheduler, or a second interpretation of run history.

## Current implementation

The current [transaction context](crates/domains/evm/src/transaction.rs) contains:

```rust
EvmTransactionContext<K, T> {
    caller_context: K,
    command: T,
}
```

`K` is opaque to transaction execution. `T` changes from complete command to reserved, prepared,
and executed transaction. This describes the current payload, rather than a complete accumulated
record. In [the stages](crates/domains/evm/src/transaction/stages.rs), execution extracts the
original command from the prepared descriptor, dropping the reservation and preparation details
from its output. Final projection retains only caller context, binding, receipt, and outcome.

The fixture uses one completion as the next transaction's caller context. It introduces local
Pure States to convert deployment completion into a command and configuration completion into
an anchored-call input. The [anchored-call context](crates/domains/evm/src/anchored_call.rs) repeats
the opaque caller-context-plus-current-payload pattern.

The new design preserves exact typed inputs and outputs. It changes how domain components access
and extend those values; Runtime does not acquire a special mutable context service.

## 1. Named typed fields and immutable snapshots

The following Rust sketches define intended responsibilities and signatures. Derives, imports,
privacy, and detailed bounds are abbreviated; these are proposed APIs, not compiling examples
of the current repository.

```rust
#[derive(MfmValue, MfmContext)]
struct ContractWorkflow<D, C, O> {
    request: ContractWorkflowRequest,
    deployment: D,
    configuration: C,
    observation: O,
}
```

`request` retains the original public request, including the checked plans. Initially the three
working fields contain checked deployment, configuration, and observation plans. Initial value
construction copies those plans from the request; it performs no runtime workflow step.

An abbreviated sequence is:

```text
deployment: CheckedCreatePlan
    -> ReservedEvmTransaction
    -> PreparedTransactionFacts
    -> ExecutedTransactionFacts
    -> CompletedTransactionFacts<Created>

configuration: CheckedCallPlan
    -> ReservedEvmTransaction
    -> PreparedTransactionFacts
    -> ExecutedTransactionFacts
    -> CompletedTransactionFacts<Called>

observation: CheckedObservationPlan
    -> AnchoredObservationFacts
```

Every arrow changes the exact context type. Deployment's transitions preserve `request`,
`configuration`, and `observation`; configuration's transitions preserve all deployment facts.
The configuration recipe requires a completed creation, so a context containing only a deployment
plan cannot satisfy its requirements.

There is no family of optional fields whose presence must be guessed. A field awaiting execution
contains its checked plan, not a fabricated result. Branches use explicit sum types when their
available facts differ.

The final output can expose ordinary accessors such as:

```rust
ctx.deployment.reservation().nonce()
ctx.deployment.preparation().transaction_hash()
ctx.deployment.settlement().receipt()
ctx.deployment.outcome().created_address()
ctx.configuration.reservation().nonce()
ctx.observation.result().return_bytes()
```

No transaction result contains the entire preceding workflow context. Transaction-local records
may contain their preceding transaction-local facts once. This preserves useful data without
recursively copying the whole workflow into each new field.

## 2. One mechanical field abstraction

Add the value-only contract to `mfm-values`:

```rust
pub trait ContextSlot<C: MfmValue> {
    type Value: MfmValue;
    type With<V: MfmValue>: MfmValue;

    fn get(context: &C) -> &Self::Value;
    fn replace<V: MfmValue>(context: C, value: V) -> Self::With<V>;
    fn slot_id() -> Result<StableId, ValueError>;
}
```

`MfmContext` lives in the existing `mfm-program-derive` crate and generates a marker for each
replaceable field. For `DeploymentSlot`, replacement is equivalent to:

```rust
ContractWorkflow {
    request: context.request,
    deployment: new_deployment,
    configuration: context.configuration,
    observation: context.observation,
}
```

That reconstruction is generated once. Domain States call `replace`; applications do not write
one reconstruction implementation per stage. Ownership can move fields in memory even though
successive durable snapshots serialize their complete accumulated values.

Keep the derive deliberately narrow:

- Named structs only; replaceable fields are distinct bare type parameters.
- Each replaceable parameter occurs in exactly one field. Other fields are preserved unchanged.
- Slot parameters have no additional trait bounds or coupled `where` clauses: `With<V>` must
  accept every `MfmValue`. Unsupported bounds receive a compile diagnostic. Moving siblings
  requires no `Clone` bound on the context or arbitrary slot values.
- No runtime strings, reflection, overlapping selectors, nested path language, or recursive
  type-list search.
- Generated marker visibility follows the containing context's visibility. Generated identities
  use an explicit context namespace/identity and the stable field name.
- Ordinary `MfmValue` and Serde contracts still own the exact value schemas and serialization.
  The new derive generates access/reconstruction, not another codec or validation system.

The slot primitive itself is mechanical replacement, not a universal proof of append-only facts.
The domain's stage types, private fact constructors, and State implementations own preservation
of previous transaction facts. Runtime still executes trusted associated State code.

An unrelated new field must require changing the context declaration, not every State in its
workflow. Consuming-crate tests must prove this property and reject unsupported derive layouts.

## 3. Reusable recipes supply the meaningful connections

A transaction recipe identifies the destination field and constructs a complete command from
checked values in the admitted context. Conceptually:

```rust
pub trait TransactionRecipe<C: MfmValue> {
    type Slot: ContextSlot<C>;
    type Success: MfmValue;

    fn recipe_id() -> Result<StableId, EvmDomainError>;
    fn command(context: &C) -> Eip1559TransactionCommand;
    // Success projection is implemented by the supported create/call mode.
}
```

This is a static Rust implementation associated with the State, not a closure retained from
Operation expansion. It has no provider, signer, Store, or ambient configuration access.
The associated success contract is restricted to the EVM-owned `Created` and `Called` modes;
the recipe selects the mode, and the projection uses that same associated contract. There is no
independently selectable projection mode that can disagree with the recipe. A custom recipe
returning the wrong command action violates its implementation contract and must be rejected as
an internal preparation failure before append/IO, not reported as an authenticated reversion.

Ship concrete production recipes for the actual reusable cases:

| Recipe | Required fields | Behavior |
| --- | --- | --- |
| `CreateAt<DeploymentSlot>` | Deployment slot contains `CheckedCreatePlan`. | Constructs a creation command. |
| `CallCreatedAt<ConfigurationSlot, DeploymentSlot>` | Checked call plan plus `CompletedTransactionFacts<Created>`. | Uses that deployment's created address as the target. |
| `CallAt<TransactionSlot>` | `CheckedTargetCallPlan` containing a checked call plan and a required checked target. | Supports an ordinary wallet/contract call, including the external-nonce follow-up run. |
| `ObserveAt<ObservationSlot, ConfigurationSlot>` | Checked observation plan plus a completed call's target and receipt anchor. | Constructs an anchored Read intent. |

The observation recipe has the analogous intent-producing contract. Its checked plan and the
selected transaction facts provide the route, bounded calldata, target, and exact anchor. Derive
and check any route reference needed by the recipe before provider entry; fallible canonical
identity construction is a framework error, never a fabricated domain rejection.
`CheckedCallPlan` deliberately has no target: `CallCreatedAt` supplies one from the selected
creation. `CheckedTargetCallPlan` adds a required `EvmAddress` for `CallAt`; do not combine these
cases using an optional target or a fallback selection rule.

The fixture must use these production recipes. It must not replace `PrepareConfiguration` with
a test-local recipe that repeats the same generic EVM field forwarding.

Custom recipes remain possible for new domain semantics. They implement one meaningful selection
or command policy, not State identity/registration/forwarding at every internal stage. The RFC
does not attempt to infer arbitrary ABI behavior from context field names.

## 4. Retain every public transaction stage result

Use these transaction-local facts:

| Record | Retained contents |
| --- | --- |
| `ReservedEvmTransaction` | Existing exact command and complete `Reservation`, including reservation EffectId, command reference, nonce domain, and nonce. |
| `PreparedTransactionFacts` | Reserved descriptor plus the complete `PreparedEvmTransactionEvidence`, including preparation EffectId and transaction hash. |
| `ExecutedTransactionFacts` | Prepared facts plus the complete `EvmTransactionSettlement`, including execution EffectId, nonce, receipt, and action outcome. |
| `CompletedTransactionFacts<Created>` | Executed facts plus the checked created-address result. |
| `CompletedTransactionFacts<Called>` | Executed facts plus the checked call-target result. |
| `RevertedTransactionFacts` | Executed facts and the typed reversion outcome. |

Reuse existing checked evidence and primitive types. Do not create parallel copies of reservation,
receipt, address, or hash contracts merely to label them as context data. Add convenient borrowing
accessors rather than storing the same derived receipt/hash again at every nesting level.

Keep `ReservedEvmTransaction` and `PreparedEvmTransaction` as capability command descriptors:
they are consumed by live preparation/execution and are not obsolete merely because State inputs
change. The preparation State constructs its existing command descriptor from reserved facts;
the execution State constructs its existing descriptor from reserved facts and preparation evidence.
Neither descriptor needs to contain the whole workflow or the new report record.

Fact construction preserves the checks currently split between descriptor constructors and
capability evidence binding: command reference, nonce domain, Effect identity, prepared hash,
settlement nonce/hash, and action agreement. Checked deserialization must preserve intrinsic
relationships; Runtime/Journal qualification continues to establish history provenance. A value
obtained from arbitrary JSON does not prove that a transaction actually ran.

For anchored calls, retain the exact intent and accepted evidence as `AnchoredObservationFacts`.
Returned evidence already contains the anchored result. Rejected/safe-failure/integrity-blocked
evidence is also retained in the failure context. Business decoding may add another result without
discarding the raw anchored evidence.

Only completed public facts are accumulated. An Effect's pending command remains in its prepare
frame until conclusion; pending does not manufacture reservation, preparation, or settlement
evidence. Context is a domain result record, not a mirror of frame heads, every retry attempt,
provider traffic, private signed wire, or secrets.

## 5. Existing State execution does the accumulation

All four transaction States remain, parameterized by the initial context and recipe. Intermediate
context aliases are derived by replacing the recipe's field with the corresponding fact type:

```text
ReserveEvmNonce<C, R>
    Input: C
    Output: ReservedContext<C, R>

PrepareEvmTransaction<C, R>
    Input: ReservedContext<C, R>
    Output: PreparedContext<C, R>

ExecuteEvmTransaction<C, R>
    Input: PreparedContext<C, R>
    Output: ExecutedContext<C, R>

ProjectEvmTransactionOutcome<C, R>
    Input: ExecutedContext<C, R>
    Output: CompletedContext<C, R>
    Failure: EvmTransactionFailure<RevertedContext<C, R>>
```

Each alias is an ordinary exact `MfmValue` type produced through `ContextSlot::With`; it is not a
new Runtime container. The generated slot must also be accessible on each replacement type.
Rust bounds express these requirements; graph expansion continues to check exact edge contracts.
Do not claim that the existing mutable authoring builder proves all graph connections at Rust
compile time.
The first three transaction States retain `Never` as their domain failure contract. Authenticated
reversion becomes a typed failure only in the existing final outcome projection.

The failure envelope contains the accumulated context with the reverted transaction facts. It
replaces the old receipt-plus-opaque-caller-context failure contract. Anchored Read failures use
the corresponding accumulated evidence context and existing reviewed failure reason.

The existing capability injection expands `ExecuteEvmTransaction<C, R>` to the four States with
the same initial context/recipe parameters. Consumers select the recipe once. They do not implement
a State for every intermediate record or manually invoke State trait methods.

Use one explicit EVM assembly helper in `mfm-evm-live` to register these four exact State ABIs for
`<C, R>`. Keep it separate from `register_evm_transaction_adapters`: the former installs executable
types, the latter binds explicit IO capabilities. This narrowly packages the repeated transaction
registration recipe; it does not make Program depend on Runtime or introduce a universal assembly
compiler. Update the live-adapter responsibility documentation to explicitly allow this pure
registration helper alongside adapter registration.

Provide a public one-transaction Operation wrapper parameterized by `<C, R>` so the generic and
wallet-follow-up tests use the same authoring entry point. A multi-step product Operation composes
those children and the anchored Read. Runtime still knows nothing about Operation implementations.

## 6. Command preparation and failure semantics

There are two different uses of the word preparation:

- `EffectState::prepare` deterministically produces one Effect command and performs no IO.
- `PrepareEvmTransaction` is an Effect State whose adapter obtains or reuses retained signed wire.

The configuration command is constructed in **`ReserveEvmNonce::prepare`**, before any nonce
reservation, signing, or broadcast. It is a complete nonce-free EIP-1559 command containing the
binding, target/action, calldata, value, gas, and fees. It contains no signature or signed wire.

The intended sequence is:

```text
read checked call plan and completed creation from context
    -> construct complete command deterministically
    -> Runtime qualifies command and derives EffectId
    -> append prepare frame and establish known insertion
    -> reservation adapter reserves or recovers exact custody
    -> interpret accepted evidence and add reserved facts
    -> append conclusion with the new accumulated context
```

The current `interpret(input, evidence)` signature does not receive the command separately.
Reservation interpretation may reconstruct it with the same total recipe from the unchanged
typed input when building the reserved descriptor. Do not change Runtime's callback interface
solely to avoid this small deterministic reconstruction. Never reconstruct using fresh external
configuration or a different selection policy.

Checked plan factories validate all constraints independent of the future target:

```rust
let plan = CheckedCallPlan::new(binding, calldata, value, gas, priority_fee, max_fee)?;
// Later, the checked created address is available:
let command = plan.command_for(created_address); // infallible domain assembly
```

The target APIs should share the existing command's validation owner. Do not duplicate fee,
byte-bound, or action validation between the plan and command constructors. All Serde and public
construction paths preserve the same guarantees. A successful creation has a required checked
address, rather than an optional address in a combined create/call success value.
Both fee values must fit the existing `u128` ceiling, the priority fee must not exceed the maximum
fee, calldata/initcode must fit their respective bounds, and gas must be nonzero.

Invalid initial configuration parameters fail before admission. This deliberately replaces the
fixture's possible later `InvalidConfigurationCommand` domain failure with a checked request
construction error. Update those expectations; this is not an invisible implementation detail.

The existing Runtime behavior remains: `PreparationError` becomes `RuntimeError::Internal`, with
no new prepare frame or adapter entry for that attempt. A legitimate execution-dependent business
rejection must not be converted into this error. If a recipe cannot construct a command from
checked inputs without such rejection, it requires explicit domain validation with a typed failure
before the Effect. It is outside the promised removal of mechanical bridges.

For this fixture, the checked call plan and required successful creation address must be sufficient
to remove `PrepareConfiguration` entirely. The checked observation plan and completed call facts
must likewise remove `PrepareObservation` entirely.

## 7. Identity, recovery, and persistence

The State's implementation identity must commit to its implementation version, recipe identity,
selected slot identities, and create/call outcome mode. Selecting deployment A instead of B is a
semantic change even when both fields have the same Rust value type. Build the identity from a
domain-separated canonical descriptor using existing hashing primitives, then use the existing
`State::state_id`/implementation-reference mechanism. Never use pointer addresses, Rust
`type_name`, anonymous closure identity, or inspection-only metadata as semantic identity.
Use a bounded digest suffix under a fixed State namespace; concatenating arbitrary source IDs
would risk exceeding `StableId`'s 512-byte limit.

Explicit recipe identity has the same trusted-code versioning obligation as current State identity:
it does not magically hash arbitrary Rust implementation semantics. Change that identity when
selection behavior changes. Exact input/output/failure schemas remain additional association keys.

All runtime parameters live in admitted data. Operation expansion and source recipes cannot depend
on changing environment values, provider state, or erased captured setup for command construction.
The initial binding still determines the adapter route and authority through existing capability
injection.

Cold fold validates every retained Effect prepare frame from its exact input snapshot and checks
the same command bytes/reference, including prepares whose conclusions are already retained.
Completed conclusion interpretation remains authoritative and is not re-executed.
Unchanged command data within one admitted run means unchanged command reference and EffectId
during recovery. Across the refactor, changed Program references and declaration positions can
change EffectIds even for semantically identical transactions; no old/new identity equality is
promised.

Program remains the immutable State/Match graph. Journal retains its exact frame grammar and
qualification. Store continues atomic exact-head append and complete-prefix load. No context
query API, delta reducer, graph wire revision, or SQL schema change is required by this design.
Context value schemas, State ABIs, Program content references, and affected contract fixtures do
change. Old assemblies/values are not supported by compatibility adapters or old-code registrations.
Historical generic Journal bytes remain governed by Journal's current wire contract; they are
not reinterpreted as the new EVM value schemas.

## 8. Reporting, failures, and branches

The success Report receives the accumulated context and can include both transactions' commands,
reservations, preparation evidence, settlements, outcomes, and anchored observation. It performs
domain reporting without provider calls or Journal access.

Keep ABI decoding and report construction explicit when they are real semantics. `DecodeValue`
may remain a Pure State adapted to append its decoded result while retaining observation evidence;
it is not retained merely to unwrap a context. A final report can also decode where that is the
single owner of the business interpretation. Implementation must choose one owner, not retain
both decode paths.

For the current fixture, keep its existing ABI decode algorithm in one genuine decoding State,
adapt that State to preserve the accumulated observation, and make the final successful value a
typed report containing the accumulated facts and decoded value. Assertions verify the report
through Runtime's public terminal value surface. This test-local ABI/report behavior is explicitly
allowed; transaction/observation forwarding and stage bookkeeping must be production behavior.

Reversion retains all facts through authenticated settlement, and anchored domain failure retains
its accepted evidence. Update fixture failure variants/conversions to preserve these accumulated
contexts rather than discarding them into a unit enum variant. Existing `Abort` handlers may still
perform that explicit root failure mapping. They do not reconstruct the successful data path.

A business Report that must itself execute after failure can use an existing typed failure handler
and a sum of the available context shapes. Reporting must not fabricate facts from unexecuted
branches or silently convert a failed workflow into success. Runtime/adapter errors and Pending
remain caller-driven progress conditions, not durable domain failures handled by that graph.

## 9. Complete code cutover and deletion scope

All replacements below apply to every current producer and consumer in the same EVM cutover.
There is one supported transaction/anchored-call context API afterward.

| Current code | Required action |
| --- | --- |
| `EvmTransactionContext<K, T>` | Delete the opaque caller-context/current-payload wrapper and its exported API/schema. Replace State inputs/outputs with slot-selected accumulated records. |
| `EvmTransactionCompletion<K>` | Delete the caller-context completion wrapper; replace with cumulative typed completed facts in the workflow field. |
| `EvmTransactionSuccess` | Replace the combined create/call success value with the outcome-specific `Created` and `Called` facts selected by the recipe. Keep the capability settlement outcome contract. |
| `EvmTransactionReversion<K>` | Delete the old caller-context/receipt failure shape; replace with failure carrying the context whose transaction field contains full reverted facts. |
| `ExecutedEvmTransaction` | Delete the lossy State payload after replacing its uses with executed cumulative facts; preserve its relevant action-consistency validation in the new owner. |
| `AnchoredContractCallContext<K>` | Delete the caller-context/intent wrapper and its constructors; recipes construct intent from accumulated context. |
| `AnchoredContractCallCompletion<K>` and `AnchoredContractCallFailure<K>` | Delete old caller-context wrappers; replace with accumulated observation/evidence success and failure contexts. |
| Fixture `PrepareConfiguration`, `PrepareObservation` | Delete types, `State`/`PureState` implementations, IDs, graph occurrences, and registrations. No renamed forwarding States replace them. |
| Fixture `MissingCreatedAddress`, `MissingCallTarget`, `InvalidConfigurationCommand`, `InvalidObservationContext` paths | Remove when checked plan/typed outcome contracts replace them; retain regression coverage at the new owning constructors/authoring boundaries. |
| Fixture `Deployment` / `Configuration` aliases that nest whole completions | Replace with aliases for named accumulated context stages. |
| Repeated four-State registration lists | Replace with the one generic EVM State-registration helper. Adapter registration and supplied authority remain separate. |
| Local one-transaction `TransactionProgram` and `WalletCall` wrappers | Delete in favor of the production transaction Operation wrapper. |
| Old context codecs, schema expectations, exports, rustdoc examples, and obsolete recovery fixtures | Update or delete with the owning contract; no legacy registration path remains. |

The concrete consumer set includes:

- [EVM transaction domain](crates/domains/evm/src/transaction.rs),
  [stages](crates/domains/evm/src/transaction/stages.rs),
  [anchored calls](crates/domains/evm/src/anchored_call.rs), and public exports.
- [Transaction contract tests](crates/domains/evm/tests/transaction_contract.rs) and
  [anchored-call contract tests](crates/domains/evm/tests/anchored_call_contract.rs).
- [Live transaction registration](crates/live/evm/src/transaction.rs),
  [transaction recovery tests](crates/live/evm/src/transaction_tests.rs),
  [generic transaction Runtime tests](crates/live/evm/tests/generic_transaction_runtime.rs), and
  [contract Effect e2e](crates/live/evm/tests/evm_contract_effect_e2e.rs).
- The Values/derive public APIs and consuming-crate tests for the new mechanical slot contract.
- Relevant domain/live READMEs, public-surface documentation, and the build guide's Effect e2e
  description when its terminal report changes. Repeat a repository reference search at cutover.

Explicitly retain:

- `ReserveEvmNonce`, `PrepareEvmTransaction`, `ExecuteEvmTransaction`, and
  `ProjectEvmTransactionOutcome`, with their new context parameters.
- Existing command/evidence capability contracts and the `ReservedEvmTransaction` /
  `PreparedEvmTransaction` wire descriptors used by live adapters.
- Custody, signer, provider, nonce, canonical receipt, and append/recovery responsibilities.
- Genuine ABI decode/report logic, typed failure mapping, and the bounded test progression driver.
- Portfolio's `EnterPortfolioCollection`, `ResumePortfolioCollection`, and `MapEvmBalanceFailure`:
  their aggregation, validation, and product failure decisions are not deleted by this EVM RFC.

There are no migrations, shims, deprecated wrappers, alternate context engines, or support for
executing superseded EVM histories through retained old State implementations.

## 10. Ownership

| Owner | Responsibility after this change |
| --- | --- |
| Values | Exact context value contracts and the small typed slot trait. |
| Existing derive crate | Generate field borrowing, sibling-preserving replacement, and stable slot identities. |
| EVM domain | Checked plans, cumulative transaction/observation facts, recipes, four-stage injection, reusable transaction Operation, and deterministic State implementations. |
| Product code | Context field names, which prior result feeds which action, ABI/report semantics, and root domain failure policy. |
| Live EVM | Existing IO adapters plus a pure helper that registers the reusable transaction's exact State family. No product graph planning. |
| Program / Runtime | Existing graph authoring/association and sole semantic fold. No field lookup service. |
| Journal / Store | Existing frame/history qualification and mechanical persistence. No context semantics. |

## 11. Verification and acceptance

The feature is complete only when all of these hold:

1. The Effect e2e uses the production create/call/observation recipes and no
   `PrepareConfiguration` or `PrepareObservation` implementation or equivalent forwarding State.
2. Its final report contains both transactions' command, reservation, full preparation evidence,
   settlement, projected outcome, anchored observation evidence, and decoded value 42.
3. Adding an unrelated context field preserves it through every stage without changing the State
   implementations or handwritten reconstruction. Two different context shapes execute and reload
   with the same reusable recipes and their correct exact schemas.
4. A wrong stage/field type cannot satisfy the recipe's required Rust bounds; graph mismatches
   remain rejected during expansion/association. Selecting a different same-typed field produces
   a distinct executable identity, and incompatible assembly is rejected before IO.
5. Checked plan tests cover calldata/initcode boundaries, fee ceiling/order, gas, deserialization,
   and total command construction after a valid target becomes available. Legitimate rejection
   never becomes an internal preparation error merely because a bridge was removed.
6. Each cumulative fact transition preserves preceding public results. Hostile fact combinations
   cannot bypass the current command/domain/evidence consistency checks.
7. Existing cancellation, ambiguous append, reservation acknowledgement loss, prepared-wire reuse
   with rejecting signer, and external nonce advancement remain covered at their owning boundaries.
   Hot/cold comparisons use the new Program and require exact retained command/history equality
   within that run, not old/new Program identity equality.
8. Reversion and anchored failure preserve the exact available facts. Branches with missing results
   do not obtain successful completion types. A terminal cold read/resume preserves report, head,
   and nonce under the existing tests' actual claims.
9. Size checks cover complete snapshots and repeated frame closure, including failure paths. The
   8 MiB object, frame limits, and 512 MiB run limit remain enforced. Copying growing snapshots can
   make total retained bytes grow quadratically with a long chain of equal-sized added results;
   bound supported inputs instead of introducing an implicit reference/delta fallback.
10. A reference search finds no superseded EVM context exports, fixture bridge IDs, or duplicate
    transaction registration/wrapper recipes among current consumers.

Use focused verification under the pinned shell while implementing:

```bash
nix develop -c cargo test -p mfm-values -p mfm-program-derive --all-targets
nix develop -c cargo test -p mfm-program -p mfm-runtime --all-targets
nix develop -c cargo test -p mfm-evm -p mfm-evm-live --all-targets
nix develop -c cargo test -p mfm-portfolio -p mfm-app --all-targets
```

Use the managed `effect-e2e` task when iterating on its behavior. Finish the cross-crate cutover
with one `nix run .#ci` on the exact final candidate, as specified by
[build and verification](docs/build-and-verification.md). Do not run all broad component gates
immediately before CI. No SQL refresh or migration task is selected unless implementation changes
an actual SQL query/baseline, which this target does not require.

## 12. Logical commit sequence

1. **Add the mechanical typed slot contract and derive.** Include consuming-crate positive and
   compile-fail tests. This is an independent value capability, not a second runtime context model.
2. **Cut over EVM contexts completely.** Include checked plans, recipes, cumulative facts, all four
   States and injection, anchored Reads, production Operation/registration helpers, every consumer,
   the updated report/failure tests, deletions, and authoritative documentation. These changes are
   inseparable: do not split them into commits that leave two supported EVM context APIs or broken
   associations. Run affected focused tests and final CI.

RFC publication itself is one documentation commit. It requires local link/symbol review and
`git diff --check`; it does not require Rust tests or CI.

## 13. Alternatives rejected

- **A shared dynamic map:** loses the simple required-field/stage contracts and introduces runtime
  lookup/ambiguity without being needed for the accepted copying model.
- **A Store/Journal-backed context view:** adds authority and retrieval semantics to State execution
  when ordinary values already meet this requirement.
- **An accumulated record with handwritten copying in every State:** relocates the original plumbing
  and multiplies future change sites.
- **Keeping all earlier workflow contexts inside each new completion:** repeats whole prefixes
  inside a single snapshot; retain transaction-local facts and sibling fields instead.
- **A broad lens, dependency-injection, or field-expression framework:** exceeds the demonstrated
  needs. Restrict the derive and ship the three concrete composition recipes plus ordinary calls.
- **Removing every Pure State:** would erase genuine domain validation, ABI interpretation, and
  report behavior. This RFC deletes identified mechanical bridges.
- **Changing Effect preparation to support durable business rejection:** unnecessary for checked
  field assembly and a separate persisted execution contract change.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| The checked plan plus typed prior outcome makes command/intent domain construction total. | Existing factories combine value checks and canonical identity construction; the proposed decomposition is not implemented. | Removing a bridge could change a legitimate domain failure into `Internal`. | Audit each constructor and implement boundary/totality tests before deletion; retain genuine domain validation where required. |
| The narrow slot derive stays smaller than the forwarding code it replaces. | Associated replacement types, Serde bounds, and exact generic schemas have not been compiled in this form. | The mechanism could become a larger type framework than the problem warrants. | Implement the restricted consuming-crate example first; simplify supported record shapes rather than add recursive generic machinery. |
| Accepted copying fits representative maximum workflows. | Internal evidence and repeated full snapshots increase retained sizes; this RFC contains no measurements. | The example could work while intended larger workflows exceed fixed format limits. | Measure canonical object/frame/run bytes for maximum intended transaction/report sequences and bound admission accordingly. |

Retention of internal public transaction stages is a confirmed requirement. No uncertainty about
that scope should be used to omit reservation or preparation evidence from the final context.
