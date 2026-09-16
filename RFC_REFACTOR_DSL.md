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

1. A typed State selection is independently compilable. Root construction does not require an
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
12. Reusable Operations are Rust type definitions over typed tuples and default-policy types.
    Caller construction uses the same tuple representation, without a universal closure or fluent DSL.
13. Production exports State types directly. There is no required State-getter collection per Operation.
14. Concrete public bindings are resolved at compilation from checked input/configuration, not
    passed into each reusable definition. Live handles remain separately bound to Runtime.
15. Preserve input/plan agreement without a public arbitrary root-only predicate. The concrete
    coupling for input-dependent source shape is a required proof before removing existing checks.
16. Retain Runtime's canonical Object representation for heterogeneous persistence and cold decoding.

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
| State selection | A typed definition of one State/capability occurrence and optional policy. Its public setup is resolved during compilation. |
| `states` | An optional local name for a tuple of imported State selections. No collection type, constructor, or getter set is required. |
| Operation | A reusable functional grouping of States or child Operations, with supported configuration, validation, and scoped policy. |
| Typed tuple | The sequence representation connecting compatible selections or Operations. It adds no execution semantics or persistence boundary. |
| Program | The immutable, content-addressed expanded State sequence, input commitment, exact contracts, and resolved policy. |
| Runtime | Association of exact implementations and live bindings, execution, continuation, authorized recovery, and checked observation. |

Use `operation` for an authored functional grouping and `states` when naming its tuple is useful.
Concrete maintained names remain descriptive, such as ContractDeploymentLifecycle and
ConfigureAndObserve. No getter collection or original Operation instance is needed to import States.

An Operation does not execute as a hidden State. Its grouping affects construction, validation,
and policy scope; it adds no Journal frame and creates no atomic transaction around its children.

## 5. Rust type definitions and tuple construction

### 5.1 One representation

Use ordinary Rust types to define maintained Operations. The proposed representation is
`Operation<Body, Defaults>`, where Body is a typed tuple of State selections or child Operations:

```rust
pub type ContractDeploymentLifecycle = Operation<
    (
        Effect<Deploy, EvmTransactionEffect>,
        Effect<Configure, EvmTransactionEffect>,
        Read<Observe, EvmAnchoredContractCallRead>,
        Pure<Validate>,
        Pure<Report>,
    ),
    LifecycleDefaults,
>;
```

These are production-owned types or specialized State aliases. `Effect`, `Read`, and `Pure` are
framework selection types, not new State implementations or one-State Operations. The concrete
capability parameter identifies the injection contract; it does not identify an account or endpoint.

The consumer selects a maintained definition with:

```rust
let operation = ContractDeploymentLifecycle::default();
```

`default()` constructs an authoring value. It performs no IO, input parsing, capability lookup,
State execution, or compilation. The recipe is defined once in source code and can be selected
many times with different checked inputs. Neither the recipe nor the resulting input-specific
Program needs to be a literal Rust `static`. A Program still commits one exact initial value.
Selection defaults construct type markers; they must not require `S: Default` or instantiate the
underlying executable State. The new-State author in section 12.5 implements no constructor.

The framework-controlled source protocol retains only the essential endpoint relationship:

```rust
pub trait AuthoringSource: sealed::Sealed {
    type Input: MfmValue;
    type Output: MfmValue;
}
```

It has no aggregate failure associated type. Implement it for the framework's typed source
representations; new-State authors enter through their State selection, not an unchecked arbitrary
emitter. The tuple implementation requires each preceding expanded output to equal the next
expanded input. Its endpoints are the first input and final output. Operations are valid tuple
elements and retain their own policy scopes.

Tuple traits alone do not supply public capability setup or root-input projection bounds. Section
9 specifies those additional compiler obligations. Do not claim the simplified trait above is a
complete implementation signature.

### 5.2 Caller construction without a closure

A caller uses the same representation without defining a struct, type alias, or trait implementation:

```rust
let operation = Operation::new((
    Effect::<Deploy, EvmTransactionEffect>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    Effect::<Configure, EvmTransactionEffect>::default(),
    Read::<Observe, EvmAnchoredContractCallRead>::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
));
```

`Operation::new(body)` is infallible construction of a well-typed source value, with unspecified
policy fields inherited. Checked runtime values and their agreement are qualified before Program
publication. Invalid adjacency fails trait checking; invalid configured values do not become valid
merely because a definition value can be constructed.
Define this constructor on `Operation<Body, Inherit>` so the caller need not infer an unconstrained
Defaults parameter. Inherit is the framework policy marker for unspecified fields, not a new
handler or a default that silently overrides the enclosing scope.

This replaces the fluent `.then` algebra and the universal `Operation::new(|scope| ...)` closure.
Do not keep them as parallel DSLs. The closure was introduced to obtain checkpoint scope; ordinary
Operation definitions do not need it. Scoped boundary markers handle checkpoints as described in
section 7.4. There is no `author_operation` or `recompose` constructor.

Definition structure is reusable independently of checked per-run values. Production Operations
own documented defaults and supported configuration choices. Custom composition may choose
explicit policy overrides using the same typed policy representation; it does not silently inherit
all the defaults of a maintained Operation merely by importing its constituent States.

### 5.3 Supported choices and bounded repetition

Where supported configuration selects different concrete source shapes, a framework-owned
`Choice<A, B>` requires equal input/output endpoints. Compilation expands only the selected branch.
This is construction-time choice, not a failure sum, runtime branching State, or second engine.
Opaque Rust return types do not erase incompatible source shapes.
A branch selected from input must be selected or qualified against the same checked input being
committed, under section 5.4's coupling requirement. Construction-time choice is not permission
to accept an independently selected incompatible plan.

Bounded homogeneous repetition is required for existing products, not deferred as an arbitrary
new DSL. Portfolio currently repeats EnterCollection -> CollectEvmBalances -> ResumeCollection for
its configured collections. Its repeated body must return the same exact context type it consumes.
Specify and compile a typed repeated-body representation before migrating that product: checked
count/source values, empty repetition as typed identity, cumulative expanded-State limits, and
fresh policy/checkpoint scope for each occurrence. No State may disappear behind unchecked erasure.

An empty repeated body is typed identity because its body supplies the endpoint contract. Preserve
zero-State Program behavior through an explicitly typed identity definition; a bare empty tuple
cannot establish an arbitrary root contract by itself.

Do not prescribe speculative tuple arity machinery or a heterogeneous dynamic list before the
consuming proof. The proof must establish the supported finite tuple arities and nesting behavior,
and preserve existing bounded product repetition. An arbitrary configuration-driven list remains
outside this RFC; it is not needed to satisfy the current Portfolio case.

### 5.4 Validation without an arbitrary input predicate

Remove the proposed public `operation.with_input_check(...)` callback. A predicate that runs when
an Operation is the root and disappears when it is nested is not a sound general precondition.
There is no authoring closure retained for later validation or execution.

The validation requirements themselves remain:

| Check | Owner |
| --- | --- |
| Value structure and local invariants | Checked input constructors and decoders |
| Selected capability binding/action agreement | Typed public setup resolution and the owning deterministic execution boundary |
| Execution-time semantic prerequisite | Checked State input/output contracts and deterministic State behavior |
| Agreement between input-dependent source shape and admitted input | Planning and compilation from the same checked root input |

Portfolio's current collection-count and route agreement checks are concrete migration obligations.
The replacement must derive input-dependent construction from the actual input whose identity is
committed, rather than accept unrelated independently planned source values. Removing the callback
and retaining two unchecked independently supplied representations would not preserve those checks.
The exact coupling for repeated/configured sources is a handoff gate; it is not solved by the
fixed lifecycle tuple or by claiming that a factory validates everything.

A semantic prerequisite such as an effective-value constraint must not vanish when its Operation
follows an addition State. Establish it through a checked contract or a deterministic State check
at the owning boundary. Compilation must not simulate preceding States to guess future values.

### 5.5 Limits of static enforcement

Static construction excludes incompatible adjacent contracts in tuples, nested Operations, and
capability prefixes/suffixes. Semantic stage types must encode meaningful prerequisites, such as
requiring completed deployment facts before configuration.

It does not prove execution succeeds, two binding values agree, evidence is authentic, or a
checkpoint remains eligible after an Effect. It cannot forbid an ordering that the declared types
permit. Repeating two transformations with the same endpoints remains valid unless the product
expresses a stronger contract.

Retain Program validation during compilation and decoding. Static Rust equality does not replace
hostile-input validation, exact schema admission, content identity, capacity limits, or Runtime
association. Grouping alone must not introduce aggregate failure schemas or extra execution frames.

## 6. Capability-owned injection and public setup

A reusable definition declares the State/capability pair without embedding a sender, endpoint,
authority epoch, or other concrete public binding. Those values belong to checked compilation
input/configuration, not to the static Operation definition or every State occurrence constructor.
They must still be resolved before Program admission; no implicit default account is introduced.

Replace mutable before/after emission with typed prefix and successful suffix definitions:

```text
prefix:      ExpandedInput -> State::Input
designated:  State::Input  -> State::Output
suffix:      State::Output -> ExpandedOutput
```

The capability owns these typed definitions, designated binding qualification, and derivation of
supporting setup from its checked public setup. The framework inserts the designated occurrence
exactly once. Empty expansion is typed identity and is valid only for equal endpoints. Prefix and
suffix have separate scopes without becoming one-State or scope-only Operations.

For Deploy and Configure the expanded sequence remains:

```text
ReserveNonce -> PrepareTransaction -> designated Effect -> ProjectOutcome
```

The consumer supplies a deployment request or deployed-contract context, not prepared transaction
facts. Every expanded State retains its own exact ABI, original failure contract, policy selection,
persistence boundary, and recovery eligibility. The suffix runs on success, not as a finally handler.

Retain one injection protocol with associated source types or equivalent typed definitions proving
these endpoint equalities. A runtime adjacency check inside the old mutable callback does not meet
this requirement. The cross-crate proof must include an invalid prefix that cannot compile and a
valid injected State used both standalone and inside an Operation.

Root public setup is resolved as described in section 9.2. The capability derives the reservation
and preparation setup from the designated transaction setup; callers must not implement separate
projections for hidden framework stages. If supported workflows use multiple bindings of one
capability, their definitions must identify the appropriate typed role or slot. Do not use a global
fallback or an untyped context bag to choose among them.

Move the existing EvmTransaction wrapper's binding, action-mode, and root input checks to checked
setup resolution/capability qualification before deleting it. Later execution checks still reject
local mismatches before dependent IO or append. An internal mismatch is not authenticated external
evidence and cannot become an integrity-block event.

Keep expansion-depth and expanded-State limits. Typed definitions are not a sandbox for arbitrary
Rust recursion. Failed compilation must publish neither a partial Program nor a registration delta.

## 7. Operations, defaults, and recovery scopes

### 7.1 Operation-owned defaults

An Operation is a reusable functional abstraction, not merely a named tuple. Its definition owns
its supported choices, meaningful sequence, public endpoint contracts, and documented policy
scope. `Operation<Body, Defaults>` makes that ownership explicit without a construction closure.

`LifecycleDefaults` is a production-owned policy provider type. It identifies the supported typed
handler and supplies its checked parameters and default allowances, using supported compilation
configuration where necessary. Numeric retry/restart limits are values, not const-generic type
parameters. The handler's concrete type must remain available for automatic and cold association.
A supported alternative handler is a typed choice, not a string naming a dynamic registry entry.

For example, an Operation could select StandardRecovery with three retries and no restarts per
State occurrence. That illustrates supported policy, not a change to shipping defaults. Framework
fallback remains Stop and zero allowances; the lifecycle baseline must document its selected
defaults. Permission for three retries does not force three retries: the handler and Runtime still
check classification, phase, evidence, and retained authority.

### 7.2 Inheritance and overrides

Resolve defaults from outer scope to the occurrence:

```text
framework fallback
    -> enclosing Operation defaults
    -> nearer child Operation or injection-scope defaults
    -> explicit State-occurrence override
```

Every expanded State, including ReserveNonce and PrepareTransaction, inherits the applicable
Operation defaults unless a more local declaration overrides them. Importing State types does
not import the policy of every maintained Operation that uses them.

Policy declarations distinguish unspecified from explicit zero. A more local declaration replaces
only the fields it explicitly specifies. A selected handler, its parameters, and checkpoint targets
form one replacement unit; allowances inherit independently. Outer defaults do not rewrite explicit
policy inside a maintained child. Reusing a child twice creates two occurrence scopes.

Use one typed policy representation for Operation defaults and occurrence overrides. An occurrence
can carry a typed policy wrapper around its State selection; it is not another State, registry,
or execution engine. Concrete modifier/associated-type spelling must be established by the compile
proof, not a second fluent sequence DSL.

Per-occurrence allowances are separate from the Program-wide recovery budget. An inherited local
retry count cannot increase the global budget or authorize a forbidden restart. Future settings
extend the typed contract only when their concrete semantics and owner are defined; no unrestricted
configuration bag is introduced for hypothetical options.

### 7.3 Classification is not redefined by an Operation

Intrinsic `ClassifyError` belongs to the exact original error contract. Its classification is
Retryable, OutcomeUnknown, InputInvalidated, or Permanent. The Operation chooses how its handler
responds to that classification and recovery context; Runtime alone authorizes the request.

A policy can stop a retryable failure without changing its classification. Changing intrinsic
classification semantics requires a changed error contract identity. Operation-specific
reclassification is a separate contract change, not an inherited default smuggled into this RFC.
There is no classifier registry, discarded original, or policy-facing substitute error.

### 7.4 Checkpoints without construction lambdas

Use nominal typed boundary markers in tuple definitions, for example `Checkpoint<AfterConfigure>`.
The marker's declared context is ConfiguredContract, so its authoring endpoints are identical and
must connect to that exact boundary. It emits no executable State or Journal frame. A typed policy
target refers to the marker, not a caller-supplied declaration index.

```text
Operation occurrence
  Deploy
  Configure
  Checkpoint<AfterConfigure>
  Observe [StandardRecovery, target AfterConfigure, one local restart]
  Validate
  Report
```

The compiler owns scope and relocation. Resolve a newly installed target only within its owning
Operation/injection scope. Reject missing or duplicate markers, foreign installations, forward
references, terminal markers without a following target State, and contract disagreement. An
already-resolved inherited parent policy keeps its original target; it is not rebound to a child
marker with the same nominal type.

Repeated or reused Operations get distinct occurrence scopes. A marker type is not a global
TypeId-to-position mapping, and allocation/type identities do not enter Program hashing. Prefix
and suffix preserve separate scopes. This uses the existing scope/relocation responsibility with
new typed marker construction; it does not require an Operation wrapper just to create a scope.

The example targets observation after the completed configuration Effect. It does not permit a
restart across that Effect. Both local allowance and the global budget must authorize the decision.
Exact marker/target trait signatures, inheritance, and reuse require a consuming proof before the
current scope mechanism is removed.

## 8. Production lifecycle contracts and configuration

### 8.1 Direct production exports

Production exports reusable State types directly from its domain module:

```rust
use mfm_evm::contract_lifecycle::{
    Deploy, Configure, Observe, Validate, Report,
    CheckedAddConfigurationValue,
    ContractDeploymentLifecycle, ConfigureAndObserve,
};
```

These exports are proposed. Production supplies concrete implementations or specialized State
aliases fixing the supported internal context/recipe contracts. A consumer does not redefine those
aliases. Neither import nor selection requires an existing Operation instance.

Delete the proposed `ContractLifecycleStates` collection, its constructor, its stored setup, and
its per-State forwarding getters. Do not replace it with generated per-Operation accessors or a
component registry. States are reusable domain components independently of which Operations use
them. Use `states` as an optional local name for a tuple, not a required collection object.

The checked input boundary for the lifecycle examples is:

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let input = config.initial_input()?;
```

The caller loads the bytes through its IO boundary. `decode` and `initial_input` validate supported
artifact/ABI choices, public transaction binding, scalar values, and transaction options. For these
library examples the checked request contains the complete public binding, exposed through
`input.transaction_binding()`. Products resolving named public binding references must complete
that checked resolution at their IO/configuration boundary before this input is admitted.

No credentials, provider handles, environment lookup, or signer access belong in the definition
or admitted input. Configuration supplies values and supported choices, not contracts through a
`config.contracts()` accessor. The compiler projects required public setup from this actual checked
input; there is no repeated concrete binding argument on each State definition.

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

### 9.1 One structural traversal and one compiler

The Program crate owns traversal of typed definitions, injection expansion, policy lowering,
checkpoint relocation, sequence qualification, and input commitment. At typed emission it supplies
exact executable requirements to a Program-owned statically dispatched receiver:

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

There is no root-map callback. Requirements cover raw and expanded value contracts, original
failures, intent/command/evidence, operational errors, and policy parameter codecs. The compiler
retains concrete types until the requirement and corresponding descriptor have been emitted.
Runtime implements the receiver using its existing exact ABI tables and conflict checks.

Generic methods use static dispatch, not a `dyn` visitor. A Program-only compiler uses a no-op
receiver. Compile-time expansion and cold type inventory use the same framework-owned structural
definition with explicit traversal modes. Components do not maintain independent compilation and
registration trees or expose a second `register` method. The feasibility proof must demonstrate
that the shared structure is sufficient for both modes, including injection and policy choices.

This mechanism removes duplicate executable-registration knowledge. It does not remove Runtime's
canonical Object representation. Object retains canonical bytes and content identity across
heterogeneous continuation and persistence boundaries; exact descriptor admission and native
decoding still qualify values at execution and cold-load boundaries. Typed authoring and public
results do not imply a second native continuation representation or an unchecked Any bag.

Compilation and association errors retain concrete source information through the receiver's
associated error. Do not collapse them into InvalidContract or a formatted string.

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

Before expansion, compilation derives public capability setup and supported policy values from
the actual checked root input being committed. There is no additional concrete binding argument
on the Operation definition. A selected leaf requires a typed projection from that root input,
not from an unavailable future State input. Production owns projections for its supported public
components; consumers do not write per-injected-stage plumbing.

The private traversal must carry root-type-aware bounds for these projections. AuthoringSource's
Input/Output types alone do not prove that every nested State's setup can be resolved. Injection
then derives its supporting setup from the designated capability's resolved setup. If a binding
can only be learned through execution, use discovery and compile a subsequent Program; do not
leave an unresolved or mutable binding inside an admitted Program.

Exact projection signatures, multi-binding typed roles, and input-dependent repetition coupling
must be proved together. Do not fill this gap with a generic setup bag, a second config object
that can disagree with C0, or the removed arbitrary input-check callback.

The builder stages requirements privately. It qualifies the complete Program, root/input agreement,
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

A fresh builder can install the executable types declared by a maintained definition:

```rust
builder.associate::<ContractDeploymentLifecycle>()?;
```

This is proposed type-inventory association. It requires no initial input, scalar values, concrete
public setup, handler parameter values, or deleted run configuration. It installs exact State,
capability, handler, and value implementations. Public live bindings remain explicit builder
inputs; the retained Program supplies its acknowledged binding identities and parameter Objects.
Cold read/resume qualify those retained descriptors against the installed implementations and
available live bindings.

Inventory visits all supported typed Choice alternatives and a repeated body's types once.
Normal compilation expands only selected alternatives and actual occurrences. Capability prefixes
and suffixes, including supported setup-dependent alternatives, must expose their complete typed
requirements without constructing fake input or setup values. This is a concrete cross-crate
proof obligation for the one shared traversal, not permission for a hand-maintained registry.

The assembly may contain supported implementations unused by one Program. That grants no extra
execution authority: only the retained Program and Runtime's current-state checks select work.
Unavailable exact ABIs remain explicit errors; type inventory cannot recover missing code.

Cold continuation executes the retained Program, not a newly expanded replacement. No
ContractLifecycleStates/from_setup object, fake C0, or reloaded configuration is required merely
to discover its executable requirements. This does not imply a legacy decoder or automatic support
for superseded contracts.

### 9.5 Later compilation and immutable assembly

The proposed Program-only entry compiles a later definition/input using the same compiler with a
no-op requirements receiver. Runtime verifies all exact requirements before admitting that
Program to an already-built assembly. A missing requirement is an explicit failure, not a request
to mutate a running Runtime or silently install code.

If a later supported definition requires additional implementations, construct a new immutable
Runtime assembly through a fresh builder with explicitly retained/reacquired live handles and the
same Store. Select definitions through the same type-inventory/compilation path; do not copy a
manual registration list. Existing Runtime instances remain unchanged. Application and discovery
workflows must exercise this boundary before the full cutover; exact public helper signatures
remain part of that handoff gate.

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

These are proposed authoring/execution bodies, not runnable current tests. Imports and surrounding
async signatures are omitted. Callers supply explicit RunId, entry_point, limits, attempt, and
Arc<dyn Store>. EVM bodies also receive explicit signer, authority, transaction-provider, and
read-provider handles where needed. These handles come from their IO/custody owners; no implicit
resource acquisition is hidden in a definition constructor.

The proposed production exports in section 8.1 supply every State, Operation, context, and codec
below except the new State in 12.5. All lifecycle examples load a checked request containing its
complete public transaction binding, requested value 42, and increment 42. The binding used for
live adapters is read from that same checked request. Compilation independently qualifies setup
projections and live association before admission.

Assertions are acceptance-test expectations. Application callers inspect the explicit outcome;
absence of success is not itself a decoded failure or proof of terminal completion.

### 12.1 One existing Pure State

Production supplies CheckedAddition, CheckedAdd, its checked scalar output, and overflow failure.
The lifecycle addition reuses the same checked arithmetic while preserving its context.

```rust
let input = CheckedAddition::new("42", "42")?;
let state = Pure::<CheckedAdd>::default();

let mut builder = Runtime::builder(store)?;
let program = builder.compile(entry_point, &state, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input, attempt).await?;

let value = result.success().expect("expected terminal success");
assert_eq!(value, &EvmU256::from_u64(84));
```

The definition has no binding or setup argument. Its input constructor parses checked scalars;
compilation supplies exact State/value requirements. There are no root maps or separately
maintained executable-State registration entries.

### 12.2 One existing injected Effect State

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let input = config.initial_input()?;
let binding = input.transaction_binding().clone();
let state = Effect::<Deploy, EvmTransactionEffect>::default();

let mut builder = Runtime::builder(store)?;
register_evm_transaction_adapters(
    &mut builder,
    binding,
    signer,
    authority,
    transaction_provider,
)?;
let program = builder.compile(entry_point, &state, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input, attempt).await?;

let deployed = result.success().expect("expected terminal success");
assert_eq!(deployed.requested_value(), &EvmU256::from_u64(42));
let created_address = deployed.deployment().created_address();
```

DeploymentRequest is the public input, not prepared facts. Compilation resolves its public setup
and associates reservation, preparation, execution, and projection. No Operation wrapper, State
getter object, or concrete binding in the State definition is required.

### 12.3 One maintained Operation definition

ContractDeploymentLifecycle is the reusable type definition in section 5.1. It owns its fixed
functional sequence and LifecycleDefaults; this caller does not redeclare either.

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let input = config.initial_input()?;
let binding = input.transaction_binding().clone();
let operation = ContractDeploymentLifecycle::default();

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

Supported configuration values specialize compilation, not the static definition's account or
endpoint. Runtime receives Program, never the authoring definition as execution input.

### 12.4 Existing Operations and States in one tuple

Production exposes ConfigureAndObserve as another definition:

```rust
pub type ConfigureAndObserve = Operation<
    (
        Effect<Configure, EvmTransactionEffect>,
        Read<Observe, EvmAnchoredContractCallRead>,
    ),
    ConfigureAndObserveDefaults,
>;
```

Its endpoints are DeployedContract -> ObservedConfiguration. Its explicit policy scope is retained
when nested; it does not duplicate the underlying State implementations. The caller can insert
this child directly into a larger tuple:

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let input = config.initial_input()?;
let binding = input.transaction_binding().clone();

let operation = Operation::new((
    Effect::<Deploy, EvmTransactionEffect>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    ConfigureAndObserve::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
));

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

Unspecified outer policy fields inherit framework fallback; explicit child defaults and occurrence
overrides resolve by section 7.2. The all-State variant in section 5.2 has the same acceptance
result. Neither consumer defines a State, context, alias, mapper, codec, getter collection, or
separately maintained executable-State registration list. Live adapter binding remains explicit.

### 12.5 New semantics composed with maintained components

The extension author defines only its new State and original failure contract:

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

The proposed ProgramError conversion preserves the identity-construction source. The retained
original execution contains the rejected input and prior deployment facts; the empty failure
value discards neither. Its classification belongs to this error contract, not its enclosing
Operation defaults. The rule executes wherever this State is composed, not only at a root hook.

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let input = config.initial_input()?;
let binding = input.transaction_binding().clone();

let operation = Operation::new((
    Effect::<Deploy, EvmTransactionEffect>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    Pure::<RequireNonZeroConfiguration>::default(),
    ConfigureAndObserve::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
));

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

A companion case uses zero initial value and zero increment, decodes exactly ZeroConfiguration
from the terminal original report, and checks the retained input and deployment facts. It proves
that no configuration command was prepared. No enclosing conversion or executable-State
registration entry is added.

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
| Handwritten sequence-only Operation implementations | Migrate to reusable Operation tuple types or the same inferred tuple constructor |
| Proposed fluent sequence DSL and universal construction closure | Remove; use one typed tuple representation and nominal scoped boundary markers |
| Proposed ContractLifecycleStates getters and per-definition concrete bindings | Remove; export State types and resolve public setup from checked root input during compilation |
| Proposed public with_input_check callback | Remove the arbitrary predicate; prove input-dependent plan/input coupling before deleting current validation |
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

First complete the bounded cross-crate feasibility slice and contract audits in section 18.
It must cover typed defaults, root setup projection, nominal checkpoint scopes, current Portfolio
repetition, cold type inventory, and the product failure-report migration. Do not start broad API
or persistence deletion merely because the five fixed lifecycle expressions look plausible.

1. **refactor typed authoring and executable association**: implement neutral sources, typed
   tuple selections, scoped Operation defaults, typed injection, input/plan qualification, and one
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
| Cold construction | Type inventory without C0/setup/parameter values after config deletion, with explicit live bindings; exact original/output decoding, supported alternatives, and nondefault handler association |
| Input-dependent planning | Portfolio repetition derives from the input being committed; count/routes mismatch cannot be admitted; empty repetition, nested same-type semantic constraints, and cumulative limits |
| Later compilation | Already-associated ABIs execute a later compiled Program; missing ABIs fail before admission; a fresh immutable assembly can add supported definitions without mutating an existing Runtime |
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

## 18. Material uncertainties and handoff gates

The RFC is ready for a bounded feasibility/contract-resolution handoff, not an unconditional
repository-wide implementation. The agreed representation and ownership decisions below must be
proved together; a compiling tuple alone is insufficient evidence.

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Tuple definitions, default-policy types, and typed injection are ergonomic on the pinned Rust toolchain. | The new representation has not been compiled against existing generic State/derive bounds. | Consumers could need forbidden aliases or a second construction path. | Compile all five callers across crates, nested Operations, a selected Choice, tuple nesting/arity limits, and incompatible injected endpoints without constructing executable States. |
| Public setup can be projected from the actual checked root input for every selected meaningful State. | Root-aware traversal bounds and multi-binding roles are not specified by AuthoringSource's endpoints alone. | A future input or ambient binding could be used, or callers could need hidden-stage setup implementations. | Prove root-input projections across crates, wrong/multiple bindings, and capability-derived reserve/prepare setup with no consumer scaffolding. |
| Input-dependent repetition can remain coupled to the committed input without arbitrary callbacks. | Portfolio's current expansion captures collection count/routes independently of C0. | Removing its validator could admit a mismatched plan, especially when nested. | Define bounded homogeneous repetition from the same checked input, preserve count/route checks before admission, and test empty/repeated scopes and nested semantic preconditions. |
| Nominal checkpoint markers preserve scope and reuse semantics. | Type-based markers have not been relocated across nested/repeated Operation occurrences. | Targets could alias, escape scope, or be rebound during inheritance. | Test duplicate/missing/foreign/terminal markers, repeated definitions, inherited already-bound handlers, typed context mismatch, and Effect barriers. |
| Cold type inventory can share the same structural traversal without setup or C0 values. | Current injection and policy construction receive values; inventory must instead cover their supported types. | Deleted config could block recovery or a second registration tree could appear. | Fresh-runtime association/read/resume with no C0 reconstruction, both Choice branches, a repeated body, injected alternatives, and a nondefault handler; unavailable exact ABIs fail explicitly. |
| Product reports can move from mapped roots to checked projections of retained originals. | Portfolio/App/transport consumers require a field and durable-identity audit. | Required facts or persistence guarantees could be lost. | Produce old-field-to-retained-source mappings and explicitly resolve any durable projection requirement before deleting its persisted payload. |
| Typed Program, staged assembly, and execution result types preserve exact acknowledgement distinctions. | Generic reconstruction, failure atomicity, and the complete result/error variants remain uncompiled. | Partial assemblies, stale-head claims, or silent authority changes could result. | Test unchanged builder on failure, exact typed reconstruction, later compilation, deadline before genesis, known acknowledgement followed by projection failure, ambiguous append, and stale observations. |
| The lifecycle's supported ABI/artifact and reports can reuse existing EVM primitives. | The fixture is evidence, not a completed production schema. | Product configuration could claim unsupported behavior or duplicate integrity checks. | Specify checked input/intermediate/report constructors and supported ABI choices; prove 42/84, malformed return, overflow, and independent evidence assertions. |

These gates do not reopen the agreed design: Rust types define Operations, typed tuples construct
them without lambdas, public bindings resolve during compilation, defaults inherit by scope,
classification remains intrinsic, Program is the execution input, and Runtime retains exact Objects
and original failures. No getter collection, public arbitrary root predicate, parallel DSL, or
mandatory aggregate failure type is required.
