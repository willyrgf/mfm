# RFC: refactor MFM authoring, compilation, and executable association

## Status and authority

This RFC records the agreed target for MFM's public authoring API before the E2E redesign.
It specifies a replacement design, not an additional convenience layer. The API names and Rust
examples below are proposed; they are not claims that the code exists or has compiled.

[Design](docs/design.md) and [architecture](docs/architecture.md) remain authoritative for the
current implementation. Their affected contracts, executable consumers, and tests must change in
the same implementation cutover. This documentation change does not alter persisted contracts,
enable transaction execution in shipping composition, or replace any executable test.

This RFC owns the target construction and compilation model. It supersedes the API sketches in
the [public interfaces and tests RFC](RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md), the
[composition design](docs/e2e-composition-design.md), and the
[extension design](docs/e2e-extension-design.md) wherever they prescribe the mutable
`OperationExpansion` DSL, `author_operation`, `config.contracts()`, root failure maps, or execution
of an Operation instead of a Program. Their independent scenarios, oracles, and required recovery
coverage remain requirements. They are not competing authoring designs.

## 1. Objective and acceptance criteria

MFM supports three activities through one compiler and one Runtime:

| Activity | Caller supplies | Framework and production components supply |
| --- | --- | --- |
| Select existing behavior | Inputs, supported options, and explicit live capabilities | Maintained Operation or State selection, checked contracts, executable association, execution, and checked results |
| Compose existing Operations and States | Order, compatible connections, and intentional policy | Typed construction, capability injection, compilation, and association of every selected implementation |
| Implement a new State | New semantics and typed input/output/failure contracts | Value qualification, capability protocols, execution, recovery, and persistence |

An existing-component consumer must not define a context struct, recipe alias, codec, mechanical
failure map, or executable-registration list. An author introducing new semantics legitimately
defines new contracts and implementations.

The baseline product is `ContractDeploymentLifecycle`:

```text
maintained Operation:
  Deploy -> Configure -> Observe -> Validate -> Report
  requested 42 -> effective 42 -> reported 42

caller-authored Operation:
  Deploy -> CheckedAddConfigurationValue -> Configure -> Observe -> Validate -> Report
  requested 42, increment 42 -> effective 84 -> reported 84
```

Deploy and Configure are Effect States, Observe is a Read State, and Addition, Validate, and Report
are Pure States. Their production implementations and contracts must exist before the E2E consumes
them. The caller changes the authored sequence; it does not rewrite calldata, edit an admitted
Program, or replace an acknowledged command.

Standalone Pure and injected Effect selections, maintained Operations, mixed compositions, and a
new State composed with maintained components must all use this same public path.

## 2. Decisions

1. A checked State selection is independently compilable. Root construction does not require an
   Operation or a one-State Operation wrapper.
2. Operations remain first-class functional abstractions: they group reusable behavior and own
   supported configuration, validation, and scoped recovery defaults.
3. Typed sequence construction proves compatible adjacent input/output contracts, including the
   capability's typed prefix and suffix. Runtime checks value-dependent facts and authority.
4. One neutral authoring protocol supports selections, sequences, and Operations. It has input and
   output contracts, but no common failure contract.
5. Capability injection owns supporting States around a designated Read or Effect. States acquire
   no `expand` hook. Injected States retain separate persistence and recovery boundaries.
6. Compilation produces an immutable Program. `Runtime::execute` accepts that Program, not its
   authoring source. A typed Program preserves checked input/output ergonomics.
7. The compiler emits exact executable requirements while lowering the selected source. Runtime
   associates them through its existing tables. There is no separately maintained registration list.
8. State and capability failures retain their own exact original contracts. Remove mandatory root
   aggregation and its maps; do not introduce `Either` failure trees or a universal error bag.
9. Intrinsic classification belongs to the exact original error contract. Operations select how
   recovery responds; they do not silently reclassify an existing error.
10. Discovery may execute a Program and use its checked output to construct a new Program. It
    cannot mutate admitted history or supersede unresolved command authority.
11. One Runtime provides bounded ordinary progression and direct progression/read/resume access.
    Attempt limits remain separate from admitted recovery policy.

## 3. Current evidence and what must change

The [current authoring implementation](crates/kernel/program/src/authoring.rs) requires an
`Operation` at `expand_program`. Its private `OperationExpansion::new` prevents callers from simply
constructing a root sequence. An Operation supplies input/output/failure types, root validation,
and mutable expansion. Adjacent contracts are checked during expansion.

`OperationExpansion::read` and `effect` already invoke capability-owned injection directly.
Injection does not require a nested Operation. The root restriction and wrapper-owned validation
explain the unnecessary `EvmTransaction<C, R>` wrapper in
[transaction stages](crates/domains/evm/src/transaction/stages.rs).

The [workflow fixture](crates/live/evm/tests/support/contract_workflow.rs) independently defines
context and recipe aliases, root failure conversions, an Operation, and `register_fixture_states`.
The [Runtime assembly](crates/kernel/runtime/src/assembly.rs) registers exact implementations,
but the selection in authoring does not supply that registration automatically.

The [Program validator](crates/kernel/program/src/program.rs) currently requires every domain
failure map chain to terminate in one root failure contract. Runtime association and cold failure
validation enforce the same contract. A typed failure sum would accommodate that requirement,
but the requirement is not necessary for linear sequencing or Runtime recovery. Remove it rather
than encode it into the replacement API.

Keep the existing owners for canonical values, exact ABI association, the private linear draft,
checkpoint relocation, continuation, causal failures, and command authority. Replace their public
construction boundary and remove superseded machinery in the same cutover.

## 4. Concepts and ownership

| Concept | Meaning and owner |
| --- | --- |
| State | One meaningful deterministic executable step. Domain/framework authors implement Pure, Read, or Effect semantics. Adapters perform IO. |
| State selection | One occurrence of an existing State with checked immutable public setup and optional occurrence policy. Framework selection types carry its concrete State/capability types. |
| `states` | An ordinary local variable containing production-provided reusable selections. It is not a registry, running State collection, global, or Operation. |
| Operation | A reusable functional grouping of States or child Operations, with supported configuration, validation, and scoped policy. |
| Typed sequence | A construction value connecting compatible selections or Operations. It adds no execution semantics or persistence boundary. |
| Program | The immutable, content-addressed expanded State sequence, input commitment, exact contracts, and resolved policy. |
| Runtime | Association of exact implementations and live bindings, execution, continuation, authorized recovery, and checked observation. |

Use `states` for the component collection and `operation` for an authored functional grouping.
Do not shadow `states` with its own chained sequence. Concrete maintained names remain descriptive,
such as `ContractDeploymentLifecycle` and `ConfigureAndObserve`.

An Operation does not execute as a hidden State. Its grouping affects construction, validation,
and policy scope; it adds no Journal frame and creates no atomic transaction around its children.

## 5. Typed construction

### 5.1 Neutral source protocol

The target protocol has the following essential shape:

```rust
pub trait AuthoringSource: Sized {
    type Input: MfmValue;
    type Output: MfmValue;

    fn then<B>(self, next: B) -> Sequence<Self, B>
    where
        B: AuthoringSource<Input = Self::Output>;
}
```

The compiler traversal is framework-controlled. Seal this protocol; existing-component consumers
never implement it, and new-State authors enter through their State selection. Extension points
remain State, capability, and handler contracts. Do not expose unchecked arbitrary descriptor
emission as an escape from typed adjacency.

`Sequence<A, B>` has `Input = A::Input` and `Output = B::Output`. It has no failure associated
type and creates no aggregate failure value. Nesting or regrouping without changed policies must
not introduce failure-contract changes merely because the construction tree is different.

Mode-specific selection constructors are:

```rust
pure::<S>() -> PureSelection<S>
read::<S, C>(setup: C::Setup) -> Result<ReadSelection<S, C>, ProgramError>
effect::<S, C>(setup: C::Setup) -> Result<EffectSelection<S, C>, ProgramError>
```

Pure selections expose the State's exact input/output. Read and Effect selections expose the
capability's expanded input/output, while the compiler retains the designated raw State ABI.
There is no required `ExpandedFailure` aggregation.

### 5.2 One Operation constructor

The proposed common constructor is:

```rust
Operation::new<S>(
    build: impl FnOnce(&OperationScope) -> Result<S, ProgramError>,
) -> Result<Operation<S>, ProgramError>
where
    S: AuthoringSource;
```

This replaces the existing Operation implementation requirement for declaring a sequence. It also
replaces the speculative `author_operation` and `recompose` helpers. The closure runs once during
construction and returns one concrete typed source. It does not receive a mutable emitter and
cannot append erased States to a draft. Ordinary closures are sufficient because this closure is
not stored as a callback generic over a future registration sink.

For ordinary construction the scope is unused:

```rust
let operation = Operation::new(|_| {
    Ok(states.deploy()
        .then(states.add_configuration_value())
        .then(states.configure())
        .then(states.observe())
        .then(states.validate())
        .then(states.report()))
})?;
```

Maintained Operations use this same constructor internally. They expose their own descriptive
factory and checked supported options, returning compositions of framework source types. Consumers
do not supply aliases to make those return types usable. Opaque Rust returns do not automatically
erase different source shapes. Where a supported configuration chooses between shapes, a
framework-owned `Choice<A, B>` requires equal input/output endpoints and contains only the selected
source. Compilation traverses that branch only. This is construction-time selection, not a
failure sum, runtime branch State, or second execution model.

An Operation can carry a typed root planning check:

```rust
operation.with_input_check(
    check: impl Fn(&S::Input) -> Result<(), ProgramError>,
)
```

This modifier retains an authoring-only validator and its immutable public planning assumptions.
It is needed when a maintained source has been selected from input-dependent facts, such as
collection counts, route choices, or supported ABI choices. Production factories supply it where
required; existing-component consumers do not reimplement validation. It performs no IO and is
not an expansion callback, executable State, or Runtime-retained closure. Its concrete type may be
an inferred Operation parameter; consumers need no validator aliases.

Compilation validates every enclosing planning check on the root path, then the first selected
component's input checks, before publishing a Program or registration delta. It does not skip an
Operation's root check merely because the first leaf has the same Rust input type. For a nested
Operation reached only after execution, its input is not available during compilation: parent
planning and checked transitions must establish its assumptions, with value-dependent checks at
the owning deterministic execution boundary. No validator simulates earlier States. Cold
association installs implementations without running initial-input checks or reconstructing C0.

### 5.3 What the type system proves

Static construction excludes incompatible adjacent contracts in caller sequences, nested
Operations, and injected prefixes/suffixes. Semantic stage types must encode meaningful
prerequisites: a configuration State requires completed deployment facts, not a generic context
in which those facts might be absent.

It does not prove that execution succeeds, two binding values agree, a response is authentic, or
a checkpoint remains eligible after an Effect. It cannot forbid an order that the declared types
deliberately allow. For example, two transformations with the same input/output type can be
repeated unless the product contract expresses a stronger restriction.

Stored bytes and runtime-selected alternatives require exact admission checks. A future arbitrary
configuration-driven component list requires its own bounded checked representation; this RFC
does not disguise an untyped list as a statically verified Rust sequence.

Retain Program validation on decoding and compilation. Static Rust equality does not replace
hostile-input validation, content identity, capacity checks, or exact Runtime association.

## 6. Capability-owned injection and root validation

Replace mutable before/after emission with typed construction of the capability's prefix and
successful suffix. For an exact State/capability pair, the obligations are:

```text
prefix:      ExpandedInput -> State::Input
designated:  State::Input  -> State::Output
suffix:      State::Output -> ExpandedOutput
```

The capability supplies checked setup, binding identity, expanded-input validation, and the two
typed sources. The framework inserts the designated occurrence exactly once. Prefix and suffix
receive their own scoped construction authority; creating a scope does not require wrapping them
in an Operation. An empty source is typed identity and is valid only for equal endpoints.

Retain one capability protocol. Its implementation signature must carry the endpoint equalities
above through associated source types or equivalent typed returns. The implementation proof must
include a compile-fail prefix mismatch; keeping today's mutable callback with a final runtime
check does not meet the target static guarantee.

For Deploy and Configure the expanded execution remains:

```text
ReserveNonce -> PrepareTransaction -> designated Effect -> ProjectOutcome
```

The caller supplies `DeploymentRequest` or `DeployedContract`, not prepared transaction facts.
Every injected State has its own exact ABI, failure contract, handler selection, persistence
boundary, and recovery eligibility. The suffix runs on success, not as a finally handler.

Move wrapper-owned root validation into the capability's checked selection contract:

```rust
fn validate_expanded_input(
    setup: &Self::Setup,
    input: &Self::ExpandedInput,
) -> Result<(), ProgramError>;
```

This retains binding agreement, transaction action mode, and input checks before admission when
their inputs are available. Product configuration/input constructors own their cross-field checks.
The first selected component validates the actual root input; later values do not yet exist at
compile time. Their deterministic preparation and checked fact constructors retain the relevant
checks at execution, before dependent IO or append. Local mismatch remains Internal and cannot
become an authenticated integrity-block event.

Retain bounded expansion depth and State counts. Injection is not a sandbox for unrestricted Rust
recursion. Failure must leave no usable partial Program or partially installed assembly.

## 7. Operations, defaults, and recovery scopes

### 7.1 Functional ownership

`ContractDeploymentLifecycle::new(&config, binding)` owns the maintained lifecycle, validates its
supported choices, obtains its reusable State selections, and applies documented recovery
defaults. Selecting the maintained Operation does not require the consumer to construct `states`.

`ContractLifecycleStates::new(&config, binding)` exposes the same reusable components independently.
It checks supported public setup; it does not import the lifecycle Operation's enclosing policy.
A caller creating a new Operation selects that policy intentionally.

The baseline retains framework Stop and zero allowances. A product may expose checked supported
policy choices. It must not quietly install unbounded retries or alter its child's intrinsic error
classification. All selected handler parameters and allowances are resolved into the Program.

### 7.2 Typed policy selection and precedence

The proposed typed modifiers are:

```rust
source.with_handler::<H>(params: H::Params)
source.with_allowances(allowances: RecoveryAllowances)
```

They return authoring values retaining `H` until compiler emission. They must not first erase a
handler into `HandlerBinding` and then require a separate registration call. Persisted descriptors
remain compiler output.

Retain the current precedence: an explicit occurrence override, then the nearest explicitly
configured Operation/injection scope, then framework defaults. A child Operation's explicit
selection takes precedence over inherited outer defaults. Replacing a handler replaces its
parameters and checkpoint targets together. Allowances inherit independently, including explicit
zero. An outer setting does not secretly rewrite explicit policy inside a maintained child.

Intrinsic `ClassifyError` remains on each exact original error contract. Classification produces
Retryable, OutcomeUnknown, InputInvalidated, or Permanent; Runtime passes the classification and
recovery context to the selected handler and authorizes its request. A policy may stop a retryable
failure without reclassifying it. Changed intrinsic semantics require a changed error contract
identity. This RFC introduces no classifier registry or configurable reclassification layer.

### 7.3 Checkpoints without mutable sequence emission

Retain allocation-based scope ownership and compiler relocation. A scope constructs a typed
checkpoint token whose boundary is a zero-State identity source:

```rust
let operation = Operation::new(|scope| {
    let checkpoint = scope.checkpoint::<ConfiguredContract>();
    let recovery = scope.handler::<StandardRecovery>(NoParams)
        .checkpoint(&checkpoint)?;

    Ok(states.deploy()
        .then(states.configure())
        .then(checkpoint.boundary())
        .then(states.observe()
            .with_handler_selection(recovery)
            .with_allowances(RecoveryAllowances::new(0, 1)))
        .then(states.validate())
        .then(states.report()))
})?;
```

`with_handler_selection` installs the same typed handler selection as `with_handler`; it is the
form carrying checked targets, not a second recovery mechanism. The checkpoint is not a State,
adds no frame, and owns no runtime mutation. Its token carries scope identity, not a caller-chosen
declaration index. Construction tokens own their scope identity and do not require borrowed
lifetimes to escape the ordinary construction closure.

The compiler resolves the marker to the expanded input boundary. Reject absent or duplicate
markers, foreign-scope installations, forward targets, and mismatched contracts. An installed
handler cannot target a terminal marker with no following State. An installed
parent handler can be inherited by a child, but the child cannot directly install a captured
parent token. Reused source values receive fresh compilation scope identities so separate
occurrences cannot alias checkpoint positions. Prefix and suffix retain separate scopes.

Runtime still checks eligibility, allowance usage, restored input, and Effect barriers. The
checkpoint example marks observation after the configuration Effect; it does not permit restarting
across that Effect. It grants one local restart; the Program's global recovery budget must also
permit that decision. No marker or closure enters persisted Program data, and allocation
identities never enter Program hashing.

## 8. Production lifecycle contracts and configuration

### 8.1 Where `states` comes from

The proposed production boundary is explicit:

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let states = ContractLifecycleStates::new(&config, binding.clone())?;
let input = config.initial_input()?;
```

The caller reads bytes through its configuration/IO boundary. `decode` is pure checked parsing.
The configuration contains a complete checked public binding selection or public reference that
the caller resolves before construction; the constructor compares it with the supplied binding.
No environment lookup, provider request, signer access, or secret resolution occurs in these
constructors. `initial_input` constructs production-owned typed values; it does not supply contract
metadata through a hypothetical `config.contracts()` accessor.

`ContractLifecycleStates` owns checked immutable public setup and returns selections. For example:

```rust
impl ContractLifecycleStates {
    pub fn deploy(&self) -> EffectSelection<Deploy, EvmTransactionEffect>;
    pub fn configure(&self) -> EffectSelection<Configure, EvmTransactionEffect>;
    pub fn observe(&self) -> ReadSelection<Observe, EvmAnchoredContractCallRead>;
    pub fn add_configuration_value(&self)
        -> PureSelection<CheckedAddConfigurationValue>;
    pub fn validate(&self) -> PureSelection<ValidateConfiguration>;
    pub fn report(&self) -> PureSelection<BuildContractReport>;
}
```

These are signature declarations, not an implementation. The production package supplies the
concrete States, supported slot/recipe bindings, and constructor checks once. No accessor executes
a State or returns an Operation masquerading as a single Effect. The collection is not a second
executable catalogue: only selected occurrences contribute requirements during compilation.

### 8.2 Public expanded contracts

| Selection | Input | Output | Responsibility |
| --- | --- | --- | --- |
| Deploy | DeploymentRequest | DeployedContract | Create the supported contract and retain checked deployment facts |
| CheckedAddConfigurationValue | DeployedContract | DeployedContract | Add the admitted increment to the effective scalar, with checked overflow and sibling preservation |
| Configure | DeployedContract | ConfiguredContract | Construct the command from the effective scalar and created address; retain checked call facts |
| Observe | ConfiguredContract | ObservedConfiguration | Read the configured target at the configuration receipt anchor |
| Validate | ObservedConfiguration | ValidatedConfiguration | Check the lifecycle-specific equality between observation and effective command argument |
| Report | ValidatedConfiguration | ContractDeploymentReport | Produce useful checked public output retaining inputs, effective value, transaction facts, and observation evidence |

Configure and Observe must preserve the target/anchor relationships already checked by lower
owners. Validate does not duplicate chain integrity validation. Report is not an identity State
included solely to give a step a name. Its checked decoding verifies the declared report relations.

Underlying generic slots, recipes, and intermediate raw transaction contexts remain valid internal
tools. Consumers of this supported workflow neither redefine nor name them. No untyped context
bag, string-key lookup, or generic JSON patching is introduced.

### 8.3 Scalar and command construction

The admitted request contains checked requested value 42 and supported increment 42, along with
the artifact, supported ABI selection, public binding, and checked transaction options. Keep the
requested value distinct from the effective value.

Deploy preserves effective 42. Addition returns a new checked context with effective 84 and the
same requested value and deployment facts. Configure deterministically constructs calldata and
the complete nonce-free command from effective 84 before reservation or acknowledgement. Observe
and Validate use those retained facts; Report returns observed 84.

The maintained sequence omits Addition and reports 42. Merely configuring an increment does not
execute it. The increment is admitted data, not an unpersisted closure capture or State-instance
field. Overflow returns a typed original failure before configuration preparation or submission.

Configuration chooses values and supported behavior. Production types own the exact contracts.
This RFC does not provide arbitrary ABI expressions, general output references, or an unbounded
configuration-driven program language.

## 9. Compilation and executable association

### 9.1 One traversal and one compiler

The Program crate owns typed lowering into the existing private linear representation, resolved
recovery descriptors, complete sequence qualification, and input commitment. It receives a
statically dispatched requirements receiver defined in the Program layer:

```rust
trait ExecutableRequirements {
    type Error: From<ProgramError>;

    fn value<V: MfmValue>(&mut self) -> Result<(), Self::Error>;
    fn pure<S: PureState>(&mut self) -> Result<(), Self::Error>;

    fn read<S, C>(&mut self) -> Result<(), Self::Error>
    where
        C: ReadCapabilityContract,
        C::OperationalError: ClassifyError,
        S: ReadState<C>;

    fn effect<S, C>(&mut self) -> Result<(), Self::Error>
    where
        C: EffectCapabilityContract,
        C::OperationalError: ClassifyError,
        S: EffectState<C>;

    fn handler<H: Handler>(&mut self) -> Result<(), Self::Error>;
}
```

There is no map callback because mandatory root mapping is removed. Exact value requirements
include input/output/original failures, intent/command/evidence, operational errors, and policy
parameters. Injected States and selected handlers are emitted through this same receiver.

Generic methods use static dispatch; do not attempt to make this a `dyn` visitor. There is no
separate `register` method on a State, Operation, or source. Framework traversal retains concrete
types until the requirement and corresponding descriptor have both been emitted.

Runtime's builder implements the receiver using existing exact ABI tables and conflict checks.
A Program-only compiler invocation uses a no-op receiver and remains usable without Runtime.
These are uses of one compiler, not parallel lowering implementations. Assembly errors preserve
their source types through the receiver's associated error; they are not collapsed into
`InvalidContract` or a formatted string.

### 9.2 Runtime-builder compilation

The proposed integrated entry is:

```rust
fn compile<S: AuthoringSource>(
    &mut self,
    entry_point: EntryPointId,
    source: &S,
    input: &S::Input,
    limits: ProgramLimits,
) -> Result<Program<S::Input, S::Output>, CompileError>;
```

The builder stages requirements privately. It qualifies the complete Program, root validation,
input commitment, exact executable associations, and required live bindings before publishing
either the Program or new registrations. On failure, existing builder contents are unchanged.
Preflight conflicts before committing a staged delta; do not leave half-installed implementations.

The builder may compile multiple selected Programs before `build` freezes one Runtime. Existing
exact registrations can be shared idempotently; conflicting implementations remain errors.
Calling `execute` does not mutate that immutable assembly or perform a second compilation.

`Program<I, O>` is the public Program with typed endpoints, not an executable bundle. It contains
no native callbacks, live handles, authoring closure, or registration plan. A private representation
supports heterogeneous persisted Programs in Runtime. Any typed reconstruction verifies the exact
endpoint contracts; phantom markers alone are not qualification. The initial value commitment
remains part of Program identity and is checked again before genesis and during cold restoration.

### 9.3 Explicit live bindings

Live adapter owners still bind checked capability identities to explicit provider, signer, and
custody handles. For example, update the existing `register_evm_transaction_adapters` and
`register_evm_anchored_contract_calls` functions to accept the Runtime builder. They register live
adapters only. Do not reintroduce a companion list of transaction States or maps.

The transaction binder retains epoch, sender, signing-purpose, and binding checks. Secrets and
handles never enter selections, Program, admitted context, Journal, reports, or diagnostics.

### 9.4 Cold association without configuration custody

A fresh builder exposes `associate(&source)` for installing requirements without an initial input
or new Program. It invokes the same typed lowering and qualification machinery, discarding the
uncommitted draft. It is not a separately maintained visitor or list. The source contains code
choices and public setup, not the deleted configuration document or initial scalar values.

Maintained components expose construction from checked public setup independently of config
repository custody:

```rust
ContractLifecycleStates::from_setup(setup: ContractLifecycleSetup)
ContractDeploymentLifecycle::from_setup(setup: ContractLifecycleSetup, recovery)
```

Both constructors are fallible. `ContractLifecycleSetup` contains the checked public binding and
supported code-shape choices; it contains no initial scalar, secret, provider, or configuration
repository handle. `recovery` is the product's checked supported typed policy selection, not an
erased descriptor requiring manual handler registration. The convenience `new(&config, binding)`
derives this setup and delegates. The caller supplies setup and policy from available code and
explicit deployment inputs independently of a deleted run configuration. The API cannot recover
unavailable setup by magic. Application composition must establish this availability for its
advertised cold-recovery surface.

Cold `read` and `resume` associate the retained Program against available exact implementations.
They do not replace it with a freshly authored Program. Changed constructor order is not permission
to reinterpret old history. If an exact implementation is unavailable, fail explicitly; content
hashes cannot recover code. No compatibility decoder or automatic old-version support is implied.

## 10. Failure handling without aggregate roots

Every State retains its exact original domain failure contract. Read and Effect operational
failures retain their separate exact capability contracts. The existing execution failure envelope
identifies which original occurred and retains its originating input, intent/command, evidence,
and execution position according to the current causal contract.

Remove the Operation/source failure associated type, injected `ExpandedFailure`/`FailureMap`,
Program root failure contract, per-State root map chains, automatic identity/from-never maps, and
the requirement for stopped domain failures to contain a mapped root. Delete map-only association
and execution paths once affected product consumers have cut over. Do not leave mandatory mapping
machinery underneath an optional field or replace it with nested `Either` values.

There is still a typed original. A failure accessor checks the actual declared contract before
decoding a requested type. Wrong-contract access is an explicit error, not unchecked JSON casting
or successful absence. Hot and cold reports retain the same original facts and causal layers.
`Never` remains the uninhabited original failure contract for States that cannot fail operationally
through their domain outcome.

Preserve the sequence: commit the declared original, classify it, invoke the handler, authorize
and commit the recovery decision. A handler, capacity, decoding, or Store failure cannot replace
or erase the acknowledged original. Internal failures remain invocation failures, not manufactured
durable operational events. Preserve first-encoding failure behavior without serializer retries
or parallel opaque native-original custody.

An independently required product-facing failure projection belongs to the existing domain/App
reporting boundary. Audit Portfolio and transport consumers before deleting their root payloads.
Retain required user-facing facts through an explicit checked projection of the original execution
report; never replace the durable original or make that projection a prerequisite for composing
States. Pure reporting projections perform no hidden history/configuration lookup. If a required
product fact cannot be recovered from retained originals and inputs, fix its declared original
contract in the same cutover rather than silently omit it.

The durable record is the original failure and its recovery context. A reconstructed product
projection is not itself durably committed, and projection failure must retain access to the
checked source incident. If a consumer requires durable projection bytes or identity, that is a
separate persistence requirement to settle before deleting its current mapped payload; transient
rendering cannot silently claim to preserve that guarantee.

Report size bounds still apply to the complete current representation. Removing duplicate mapped
payloads can change byte sizes; preserving capacity behavior means preserving limits and failure
semantics, not manufacturing duplicates to reproduce old thresholds. Overflow before terminal
append leaves the acknowledged recovery head, original facts, and command authority intact. Use
small explicit limits to test this boundary.

## 11. Runtime execution and checked results

The proposed execution surface is:

```rust
async fn execute<I: MfmValue, O: MfmValue>(
    &self,
    run_id: RunId,
    program: Program<I, O>,
    input: I,
    attempt: AttemptOptions,
) -> Result<ExecutionResult<O>, InvocationFailure>;
```

Program already carries its entry point, limits, resolved recovery choices, exact expanded
sequence, and input commitment. Runtime verifies its association and input before admission.
Executing it never invokes an Operation constructor or capability injection callback.

`ExecutionResult<O>` is a checked result, not another driver. It distinguishes terminal success,
terminal original failure, and bounded incomplete progression. `success()` borrows an `O` only
for qualified success; `failure()` exposes the complete checked original report only for terminal
failure. Callers inspect the explicit outcome when either is absent. Neither accessor manufactures
success from raw bytes. `RecoveryStopped` retains its existing unresolved-authority meaning through
`InvocationFailure`; it is not an ordinary terminal domain failure.

Attempt options contain a deadline, a finite progression bound, and a supported pending-poll
interval. They are process-local controls, not admitted recovery policy. The bounded loop delegates
every transition to the existing engine and follows this action contract:

| Observation | Action |
| --- | --- |
| Runnable, AwaitingRecovery, AwaitingInterpretation | Continue eligible progression within the budget; Runtime alone authorizes transitions |
| Pending Effect without RecoveryStopped | Reconcile the same retained command at the selected bounded polling cadence |
| Durable success or terminal failure | Return its checked result |
| RecoveryStopped | End automatic driving; explicit later resume may reconcile existing authority |
| Store/internal/invocation failure or ambiguous acknowledgement | Return its causal failure and exact recovery identity; do not invisibly retry admission or append |
| Deadline or work-bound exhaustion between transitions | Return explicit incomplete progression and its qualified observation |
| Deadline interrupting in-flight work | Preserve known acknowledgement/authority and uncertainty; do not substitute an older view for the current head |
| Cancellation or disconnected caller | Preserve cancellation safety; no response or cancellation record is promised |

Before any acknowledged view exists, a stopped invocation must still identify the selected RunId
and available recovery context. After a known acknowledgement with failed projection, preserve
the acknowledgement separately from the last qualified observation. A successful Store commit,
ambiguous commit, historical read, and unavailable observation cannot share a misleading status.

Retain direct start/progression, `read`, and `resume`. Cold result access uses the same exact value
decoders as hot execution. Do not introduce `RuntimeDriver`, a second scheduler, or consumer-owned
`drive_to_success` loops for ordinary supported progression.

## 12. Caller examples

These are complete authoring/execution bodies against the proposed API, not runnable current tests.
Imports and the surrounding async function signature are omitted. The caller supplies explicit
`run_id`, `entry_point`, `limits`, `attempt`, and `Arc<dyn Store>`. EVM bodies also receive a checked
`EvmTransactionBinding`, `Arc<dyn Secp256k1Signer>`, `Arc<dyn EvmTransactionAuthority>`,
`Arc<dyn EvmTransactionProvider>`, and `Arc<dyn EvmReadProvider>` where needed. Resource acquisition
belongs to their IO/custody owners; no ambient discovery helper is implied.

`config_bytes` is a public configuration document already loaded through the caller's IO boundary.
It supplies requested value 42 and increment 42. Assertions express acceptance-test expectations;
ordinary applications match the result's explicit terminal/incomplete outcome. All State, context,
and selection names below except the extension in 12.5 are supplied by production.

### 12.1 One existing Pure State

Production supplies a checked scalar addition State and its input constructor. The shared checked
arithmetic semantics are also used by the context-preserving lifecycle addition.

```rust
let input = CheckedAddition::new("42", "42")?;
let selection = pure::<CheckedAdd>();

let mut builder = Runtime::builder(store)?;
let program = builder.compile(entry_point, &selection, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input, attempt).await?;

let value = result.success().expect("expected terminal success");
assert_eq!(value, &EvmU256::from_u64(84));
```

`CheckedAddition::new` parses checked scalars; `CheckedAdd` returns `EvmU256` or its exact overflow
failure. There are no capabilities, root maps, codecs, or registrations to supply.

### 12.2 One existing injected Effect State

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let states = ContractLifecycleStates::new(&config, binding.clone())?;
let input = config.initial_input()?;
let selection = states.deploy();

let mut builder = Runtime::builder(store)?;
register_evm_transaction_adapters(
    &mut builder,
    binding,
    signer,
    authority,
    transaction_provider,
)?;
let program = builder.compile(entry_point, &selection, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input, attempt).await?;

let deployed = result.success().expect("expected terminal success");
assert_eq!(deployed.requested_value(), &EvmU256::from_u64(42));
let created_address = deployed.deployment().created_address();
```

The input is `DeploymentRequest`. Checked success is `DeployedContract` with required creation
facts. Compilation automatically associates reservation, preparation, execution, and projection.
Their separate failures retain their own exact contracts and original operation context.

### 12.3 One maintained Operation

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let input = config.initial_input()?;
let operation = ContractDeploymentLifecycle::new(&config, binding.clone())?;

let mut builder = Runtime::builder(store)?;
register_evm_transaction_adapters(
    &mut builder,
    binding.clone(),
    signer,
    authority,
    transaction_provider,
)?;
register_evm_anchored_contract_calls(
    &mut builder,
    binding.route.clone(),
    read_provider,
)?;
let program = builder.compile(entry_point, &operation, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input, attempt).await?;

let report = result.success().expect("expected terminal success");
assert_eq!(report.requested_value(), &EvmU256::from_u64(42));
assert_eq!(report.effective_value(), &EvmU256::from_u64(42));
assert_eq!(report.observed_value(), &EvmU256::from_u64(42));
```

The caller selects maintained behavior and its supported configuration. The constructor owns the
sequence and policy defaults. It does not own live IO. The caller need not construct `states`.

### 12.4 Existing Operations and States in one composition

Production also supplies `ConfigureAndObserve::new(&config, binding)`, a meaningful reusable child
Operation with endpoints `DeployedContract -> ObservedConfiguration`. Its body uses the same
`states.configure().then(states.observe())` selections and documented policy scope. The maintained
lifecycle may reuse this child; no second implementation of its behavior is permitted.

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let states = ContractLifecycleStates::new(&config, binding.clone())?;
let input = config.initial_input()?;
let configure_and_observe = ConfigureAndObserve::new(&config, binding.clone())?;

let operation = Operation::new(|_| {
    Ok(states.deploy()
        .then(states.add_configuration_value())
        .then(configure_and_observe)
        .then(states.validate())
        .then(states.report()))
})?;

let mut builder = Runtime::builder(store)?;
register_evm_transaction_adapters(
    &mut builder,
    binding.clone(),
    signer,
    authority,
    transaction_provider,
)?;
register_evm_anchored_contract_calls(
    &mut builder,
    binding.route.clone(),
    read_provider,
)?;
let program = builder.compile(entry_point, &operation, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input, attempt).await?;

let report = result.success().expect("expected terminal success");
assert_eq!(report.requested_value(), &EvmU256::from_u64(42));
assert_eq!(report.effective_value(), &EvmU256::from_u64(84));
assert_eq!(report.observed_value(), &EvmU256::from_u64(84));
```

The new Operation uses framework defaults where it supplies no override; the maintained child
retains its explicit policy. Selecting all six States directly uses the constructor in section
5.2. Neither form defines a State, context, alias, mapper, codec, or registration list.

### 12.5 New semantics composed with existing components

The extension author introduces a rule requiring a nonzero effective configuration. It reuses
the production context and defines only its new State and original failure contract:

```rust
#[derive(
    Debug,
    serde::Serialize,
    serde::Deserialize,
    mfm_program_derive::MfmValue,
)]
#[serde(deny_unknown_fields)]
pub struct ZeroConfiguration {}

impl ClassifyError for ZeroConfiguration {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

pub struct RequireNonZeroConfiguration;

impl State for RequireNonZeroConfiguration {
    type Input = DeployedContract;
    type Output = DeployedContract;
    type Failure = ZeroConfiguration;

    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("example.require-nonzero-configuration@1")?)
    }
}

impl PureState for RequireNonZeroConfiguration {
    fn evaluate(
        input: DeployedContract,
    ) -> Result<
        ProposedStateOutcome<DeployedContract, ZeroConfiguration>,
        InvocationDiagnostic,
    > {
        if input.effective_value() == &EvmU256::from_u64(0) {
            Ok(ProposedStateOutcome::Failure {
                failure: ZeroConfiguration {},
            })
        } else {
            Ok(ProposedStateOutcome::Success { output: input })
        }
    }
}
```

The proposed `ProgramError` conversion used by `state_id` must retain the identity-construction
source; copying existing lossy `map_err` conversions is not acceptable. The original execution
record retains the rejected input, so the empty domain failure does not discard the scalar or
preceding deployment facts. Its classification is intrinsic to this new contract.

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let states = ContractLifecycleStates::new(&config, binding.clone())?;
let input = config.initial_input()?;
let configure_and_observe = ConfigureAndObserve::new(&config, binding.clone())?;

let operation = Operation::new(|_| {
    Ok(states.deploy()
        .then(states.add_configuration_value())
        .then(pure::<RequireNonZeroConfiguration>())
        .then(configure_and_observe)
        .then(states.validate())
        .then(states.report()))
})?;

let mut builder = Runtime::builder(store)?;
register_evm_transaction_adapters(
    &mut builder,
    binding.clone(),
    signer,
    authority,
    transaction_provider,
)?;
register_evm_anchored_contract_calls(
    &mut builder,
    binding.route.clone(),
    read_provider,
)?;
let program = builder.compile(entry_point, &operation, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input, attempt).await?;

let report = result.success().expect("expected terminal success");
assert_eq!(report.observed_value(), &EvmU256::from_u64(84));
```

A companion case uses zero initial value and zero increment, obtains the terminal original report,
decodes exactly `ZeroConfiguration`, and verifies the retained rejected input and deployment facts.
It must prove that no configuration command was prepared. The author adds no enclosing error
conversion and no registration entry.

## 13. Discovery and subsequent Programs

Discovery is ordinary Program execution whose checked output informs later construction:

```text
compile and execute discovery Program with its RunId
    -> inspect qualified output and required provenance
    -> construct the next Operation or State sequence
    -> compile a new Program and input commitment
    -> execute under a new RunId
```

The selected second sequence can reflect supported choices from discovery. New construction is
not mutation of discovery's Program, the next Program after admission, or an acknowledged command.
Product-required linkage includes the exact source RunId, output identity, and head as appropriate,
and is checked before dependent admission. Retain the existing configuration-deletion and dependent
start semantics where those products apply.

Discovery does not replace Runtime recovery of an unresolved Effect. It cannot authorize a new
nonce, replacement transaction, or bypass a stopped handler while old command authority remains.
This RFC adds no inter-Program scheduler, saga engine, or arbitrary mutable graph.

## 14. Crate boundaries and persistence changes

The domain graph remains inward:

```text
Values / IDs / capability contracts
              ^
           Program <--- domain components
              ^                  ^
           Runtime <---------- live adapters
              ^
         Application / transports
```

Program owns authoring types, typed injection compilation, policy lowering, and Program
qualification. Runtime owns executable association and progression. Domains own their States,
Operations, checked configuration, contexts, and product reports. Live crates bind reusable
platform providers/signers/custody. Journal remains the exact opaque append-only frame owner;
Store remains mechanical admission/latest/probe and atomic exact-head append.

Removing root failure fields changes Program identity and Runtime payload/report schemas. Update
all producers, decoders, validators, transports, fixtures, and docs together. Choose schema versions
from the actual changed contracts during implementation; do not invent a speculative parallel wire.
Reject superseded data under the repository's clean-cutover policy. Do not rewrite acknowledged
history, provide a migration reader, or claim old runs remain executable with unavailable ABIs.

Journal's opaque envelope and Store's physical schema need not change solely because Runtime's
payload changes. Reassess them only if their owned format or capacity contract actually changes.
PostgreSQL admission snapshots, locked append transactions, and ambiguous COMMIT semantics remain.

No new dependency is required by this design. No cryptographic, keystore Send/Sync, transaction
replacement, or mainnet finality redesign is authorized. The current EVM receipt policy remains
limited to the pinned non-reorging development environment. Maintained production library code
does not by itself make that policy suitable for shipping transaction composition.

## 15. Complete deletion and migration scope

| Existing machinery | Target treatment |
| --- | --- |
| Operation-only `expand_program` root | Replace with the neutral source compiler; standalone selections use it directly |
| Public mutable `OperationExpansion` DSL | Remove; retain useful private draft/relocation logic behind typed source traversal |
| Mutable capability prefix/suffix emission | Replace with typed prefix/suffix sources; keep capability ownership and designated insertion |
| Wrapper-only `EvmTransaction<C, R>` | Move validation to checked selection/capability ownership and delete the wrapper |
| Handwritten sequence-only Operation implementations | Migrate to the common typed Operation constructor |
| Operation/expanded/root failure aggregation | Delete associated contracts, automatic lifts, maps, and map-only Runtime machinery |
| Product public failure projection | Audit actual requirements; preserve justified checked projections at their product reporting owner |
| `register_fixture_states` and transaction State registration helper | Delete after compiler requirement emission covers every exact State, codec, and handler |
| Public separately built assembly for ordinary callers | Fold construction into Runtime's builder; retain one internal association implementation |
| Fixture contexts, recipe aliases, DecodeValue, report reconstruction | Replace with maintained production contracts/States and exact checked access; delete superseded copies |
| Ordinary copied progress loops and output decoding | Replace with Runtime execution/results; retain deliberate direct progression tests |
| `recompose`, `author_operation`, `config.contracts()`, execute(Operation), failure `Either` proposals | Do not implement; remove conflicting target documentation with its replacement |

Retain explicit live adapter binding, reusable context-slot mechanics, typed State implementations,
causal diagnostics, original failure retention, canonical hashing, exact ABI checks, recovery
barriers, and independent test oracles. Moving old fixture code behind new public wrappers without
removing duplicated ownership does not satisfy the cutover.

## 16. Logical implementation commits

1. **refactor typed authoring and executable association**: implement neutral sources, typed
   selections/sequences, scoped Operations, typed injection, root validation relocation, and one
   requirement receiver. Migrate authoring and association consumers together. Delete the mutable
   DSL, wrapper-only Operations, and duplicate executable lists. Update current architecture/design
   and consuming tests in this commit. Include failure-contract removal here if the API and wire
   cannot coherently change separately; do not create a temporary mandatory failure algebra.
2. **remove aggregate root failure contracts**: only a separate commit if the preceding cutover can
   remain internally coherent. Remove root maps and mapped-root persistence, migrate actual product
   reporting requirements, and update every affected schema, transport, cold decoder, and capacity
   test together. Preserve exact originals and recovery authority.
3. **consolidate runtime execution and checked results**: accept compiled Program, add bounded
   progression and honest stopped outcomes over the existing engine, and replace ordinary copied
   drivers/decoders. Keep direct APIs and interruption/ambiguity coverage. Merge with the first
   commit if typed Program construction and Runtime public construction are inseparable.
4. **provide lifecycle contracts and consumer acceptance**: implement maintained product contexts,
   scalar-before-command semantics, reusable selections, meaningful validation/reporting, and the
   five consuming examples. Replace fixture equivalents and update managed case ownership in the
   same change. Keep independent fault infrastructure and external oracles.

This is an ordering of logical changes, not permission to leave incompatible intermediate APIs.
Implementation feasibility probes belong to the relevant cutover and need not become a shipped
parallel design. Every implementation commit reports what was simplified/deleted, necessary added
complexity, and measured production-code LOC change separately from test/docs changes.

## 17. Verification and acceptance matrix

| Boundary | Required evidence |
| --- | --- |
| Static authoring | Compile-fail incompatible State/Operation adjacency and incompatible injected prefix/suffix; positive standalone, nested, and mixed cases |
| Generic reuse | A second supported context and ordinary call to an existing address, without fixture-specific runtime machinery |
| Operation scopes | Nested precedence, explicit zero allowances, target replacement, inherited installed handler, foreign/absent/duplicate checkpoint rejection, and repeated source occurrence relocation |
| Root checks | Direct and nested injection equivalence; wrong binding/action mode rejected at the owning boundary before affected IO or append |
| Association | Injected States and custom handler included automatically; exact generic ABI conflicts rejected; failed compilation leaves builder unchanged |
| Cold construction | Fresh Runtime after configuration deletion, using available code and explicit public setup; exact original/output decoding and nondefault policy association |
| Product success | Maintained 42 and composed 84, with separate fresh signer/nonce domains where required |
| Product evidence | Configuration targets created address; observation matches configuration target/receipt anchor; admitted input remains 42 and command encodes effective 84 |
| Product rejection | Addition overflow before configuration; observed-value mismatch; malformed/wrong ABI result; new-State zero failure retains exact rejected input and prior facts |
| Causal failures | Distinguishable nested domain/provider/authority/signer causes survive hot/cold access, with classification unchanged and no deliberately appended secrets |
| Report limits | Small-bound complete-report overflow preserves original, acknowledged head, and command authority; no production-maximum allocations |
| Attempt control | Deadline before/after acknowledgement, stale last observation, cancellation, ambiguous admission/append, RecoveryStopped, and retained-command pending polling |
| Discovery | Checked source-output linkage, new immutable Program/RunId, no rewrite of old Program or pending authority |

Before deleting an existing assertion, record its executable replacement and managed owner.
Preserve reservation acknowledgement loss, prepared-wire recovery with a rejecting signer,
transaction-boundary cancellation/ambiguity, external nonce advancement, cold terminal head/output/
nonce, actual SQL causes, retained-epoch rejection without append, and closed-signer-owner failure.
Preserve client transport/configuration-deletion/enrichment coverage from the broader E2E RFC.
Future cases do not justify deleting exercised guarantees.

The managed Effect E2E's Runtime reconstruction retains the same keystore owner. Do not describe
that as host-process key recovery. Its terminal checks establish unchanged history/output/nonce,
not absence of provider calls. Use pinned Reth/PostgreSQL/solc through existing managed tasks;
compiled first-party contract artifacts remain temporary.

Follow [build and verification](docs/build-and-verification.md). Run affected focused tests in the
default Nix shell, expanding to dependents for changed public contracts. Run the relevant managed
cases and one final `nix run .#ci` on the complete implementation candidate; do not stack broad
gates immediately before CI. Change task selection only with executable coverage changes.

For this documentation-only RFC, review links, current-symbol references, contract consistency,
and `git diff --check`. No Rust, managed E2E, or CI gate is selected. Production-code LOC change is
zero. The implementation must measure its own delta; no numerical reduction is claimed here.

## 18. Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Typed source/injection bounds and the scoped constructor remain ergonomic on the pinned Rust toolchain. | The proposed API has not been compiled against existing derives and generic State bounds. | Callers could need forbidden aliases or the compiler could require another representation. | Compile all five callers, two contexts, typed injection failures, and a custom handler before fixing public signatures; change representation rather than weaken contracts. |
| Typed checkpoint markers can preserve existing scope and reuse semantics without mutable emission. | Marker relocation and fresh identities have not been implemented for nested/reused source values. | Targets could alias, escape scope, or resolve to the wrong expanded boundary. | Prove nested injection/Operation scopes, repeated selection, cross-scope rejection, and Effect-barrier recovery through consuming tests. |
| Product-facing root reports can be replaced by checked projections of retained originals and operation inputs. | Portfolio/App/transport consumers still use mapped roots; reliance on their durable bytes/identity needs auditing as well as displayed fields. | A required public fact or durability guarantee could be lost, or an implicit history/config lookup introduced. | Inventory consumers before deletion; preserve required facts and explicitly resolve any durable-projection contract before the schema cutover. |
| Cold association can reconstruct required typed selections and policy choices without configuration custody. | Current factories and registration are not organized around that boundary. | Deleted config or a missing exact implementation could prevent recovery. | Fresh-runtime read/resume after config deletion with injected Effects and nondefault policy; fail explicitly on unavailable ABIs. |
| Program endpoint typing and staged assembly updates can use existing qualification/registration owners. | Generic Program decoding and transactional builder publication need a consuming implementation proof. | A second wrapper/registry could appear or failed compilation could leave usable partial state. | Test exact typed reconstruction, changed input rejection, ABI conflicts, and unchanged builder state after each construction failure. |
| The lifecycle's supported ABI/artifact and product report contracts can reuse existing EVM primitives. | The fixture is evidence, not a completed production schema. | The product could expose unsupported configuration choices or duplicate lower integrity checks. | Specify the supported ABI catalogue and checked context/report constructors, then prove 42/84, malformed return, overflow, and independent evidence assertions. |

These are implementation proof obligations and product-consumer audits. They do not reopen the
agreed ownership decisions: standalone State selection, Operations as configured policy scopes,
Program as compilation output, Runtime as execution owner, and no mandatory aggregate failure type.
