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
5. The resolved capability implementation owns supporting States around the selected executable
   Read or Effect. States acquire no `expand` hook or replacement workflow body. Injected States
   retain separate persistence and recovery boundaries.
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
11. One Runtime provides ordinary progression and direct progression/read/resume access. No
    execution deadline, wall-clock limit, AttemptOptions, or optional progression-control API is added.
12. Reusable Operations are Rust type definitions over typed tuples and default-policy types.
    Caller construction uses the same tuple representation, without a universal closure or fluent DSL.
13. Production exports State types directly. There is no required State-getter collection per Operation.
14. Concrete public bindings are resolved at compilation from checked input/configuration, not
    passed into each reusable definition. Live handles remain separately bound to Runtime.
15. Preserve input/plan agreement without a public arbitrary root-only predicate. The concrete
    coupling for input-dependent source shape is a required proof before removing existing checks.
16. Retain Runtime's canonical Object representation for heterogeneous persistence and cold decoding.
17. A network-independent State uses a fixed semantic capability contract. Expansion selects an
    implementation; it does not specialize Deploy into Deploy<C> or replace it with another State.
18. Native command, evidence, and operational-error contracts belong to the selected implementation.
    Retain exact native settlement evidence; derive the semantic view during interpretation.
19. Resolve capabilities and assemble exact callbacks before execution. Execution-time value checks
    belong to their selected components and existing admission/binding boundaries, not a new generic
    Runtime State/implementation qualification hook.
20. Do not add optional selection/policy modifiers, a universal TransactionOutcome hierarchy, or
    general-purpose erased values. Add only interfaces justified by the supported scenarios.

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

The current capability contract also combines the State-facing command/evidence interface with
the native adapter's operational-error contract. Separate those responsibilities as specified in
section 6. This changes Program and Runtime association/payload contracts, not just authoring names.
The EVM implementation already retains signed wire in its authority and exposes a public prepared
descriptor. Reuse that custody boundary; do not introduce a second transaction store.

## 4. Concepts and ownership

| Concept | Meaning and owner |
| --- | --- |
| State | One meaningful deterministic executable step. Domain/framework authors implement Pure, Read, or Effect semantics. Adapters perform IO. |
| State selection | A typed definition of one State/capability occurrence and optional policy. Its public setup is resolved during compilation. |
| Semantic capability | The fixed typed intent/command and evidence interface used by a State, independent of the supported network implementation. |
| Capability implementation | Owns network-specific preparation, injected supporting States, native contracts/codecs, protocol validation, evidence projection, and operational errors; live adapters supply its explicit IO. Selected before Program publication. |
| `states` | An optional local name for a tuple of imported State selections. No collection type, constructor, or getter set is required. |
| Operation | A reusable functional grouping of States or child Operations, with supported configuration, validation, and scoped policy. |
| Typed tuple | The sequence representation connecting compatible selections or Operations. It adds no execution semantics or persistence boundary. |
| Program | The immutable, content-addressed expanded State sequence, input commitment, exact contracts, and resolved policy. |
| Runtime | Association of exact implementations and live bindings, execution, continuation, authorized recovery, and checked observation. |

Use `operation` for an authored functional grouping and `states` when naming its tuple is useful.
Concrete maintained names remain descriptive, such as ContractDeploymentLifecycle and
ConfigureAndObserve. No getter collection or original Operation instance is needed to import States.

All network-dependent behavior belongs to capability implementations and their supporting
components. Authored domain States consume fixed semantic capability contracts without inspecting
native representations. A capability implementation includes deterministic preparation and
validation as well as its live adapters; it is not merely a provider callback. Generic canonical
encoding, hashing, and schema admission remain framework responsibilities, as do persistence and
recovery scheduling. This ownership does not make injected supporting States network-independent.

An Operation does not execute as a hidden State. Its grouping affects construction, validation,
and policy scope; it adds no Journal frame and creates no atomic transaction around its children.

## 5. Rust type definitions and tuple construction

### 5.1 One representation

Use ordinary Rust types to define maintained Operations. The proposed representation is
`Operation<Body, Defaults>`, where Body is a typed tuple of State selections or child Operations:

```rust
pub type ContractDeploymentLifecycle = Operation<
    (
        Effect<Deploy, TransactionEffect>,
        Effect<Configure, TransactionEffect>,
        Read<Observe, ContractRead>,
        Pure<Validate>,
        Pure<Report>,
    ),
    LifecycleDefaults,
>;
```

These are production-owned State types. `Effect`, `Read`, and `Pure` are
framework selection types, not new State implementations or one-State Operations. The capability
parameter identifies a fixed semantic interface. Expansion resolves its supported implementation,
public binding, and typed injection from checked configuration. Deploy itself remains the same
non-generic executable State; neither an account nor a network type is a State type parameter.

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
    Effect::<Deploy, TransactionEffect>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    Effect::<Configure, TransactionEffect>::default(),
    Read::<Observe, ContractRead>::default(),
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
the supported defaults of a maintained Operation or define meaningful Operation defaults where
needed; it does not silently inherit a workflow's policy merely by importing its constituent States.
This RFC adds no arbitrary occurrence-modifier API.

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
| Selected capability binding/action agreement | Typed public setup resolution and capability-owned preparation/checked contracts at the applicable execution boundary |
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

## 6. Semantic capabilities and implementation-owned injection

### 6.1 The selected State remains executable and non-generic

Deploy is one concrete Effect State with fixed input, output, failure, and semantic implementation
identity. It implements EffectState<TransactionEffect>. TransactionEffect is a network-independent
semantic interface, not a Rust type parameter applied to Deploy and not a network switch in its code.
The selected native implementation supplies the protocol behind that interface.

```text
Effect<Deploy, TransactionEffect>
    -> resolve supported implementation and public binding during expansion
    -> inject that implementation's supporting States
    -> emit the same Deploy executable with the resolved implementation association
```

Do not introduce Deploy<C>, BindEffect, StateDefinition bodies, or a substitute ExecuteEvmTransaction
in place of Deploy. Those are superseded design alternatives. A separate PrepareContract State can
use a CompileContract or RequestContract capability when the product requires artifact preparation.
It is ordinary preceding work, not a hidden provider call inside Deploy. State implementations remain
deterministic and perform no ambient IO; capability injection does not combine several acknowledged
external actions into one callback or atomic State.

A semantic capability must specify supported actions and precise completion guarantees. A common
name does not make EVM deployment semantics valid on every network. Unsupported action/network
choices fail checked construction before admission. Network-dependent request fields, identifiers,
and evidence must have reviewed typed contracts; arbitrary JSON or an untyped context bag is not
an implementation of portability.

### 6.2 Two contracts, one execution path

Separate the State-facing interface from the selected implementation's native ABI. These signatures
show the required type ownership, not a complete compiled trait definition:

```rust
pub trait EffectCapabilityContract {
    type Command: MfmValue;
    type Evidence: MfmValue;

    // Stable semantic contract identity and command/evidence qualification.
}

pub trait EffectImplementation<C: EffectCapabilityContract> {
    type Binding: MfmValue;
    type NativeCommand: MfmValue;
    type NativeEvidence: MfmValue;
    type OperationalError: MfmValue;

    // Exact implementation identity and checked public binding contract.
    // Decode/extract NativeCommand at the already-associated native boundary, without IO.
    // Bind NativeEvidence to the admitted command, EffectId, and binding.
    // Deterministically project NativeEvidence into C::Evidence.
}
```

For TransactionEffect, the proposed fixed command is SubmitPreparedTransaction and the fixed
State-facing evidence is TransactionEvidence. These names describe their roles; their final fields
must follow the concrete consuming proof rather than a speculative universal transaction vocabulary. For EvmTransactionImplementation, native types reuse
PreparedEvmTransaction, EvmTransactionSettlement, and EvmTransactionOperationalError. Deploy's own
semantic failure remains its State::Failure; no aggregate capability or Operation error is required.

The implementation supplies deterministic native command extraction, evidence binding, and semantic
projection. The callbacks are associated before execution, not discovered or resolved by Runtime.
They preserve existing exact decoding/binding responsibilities; they do not introduce a generic
Runtime product-validation hook. Native preparation and its checked value constructors establish
product/native correspondence as described in section 6.3. Live callbacks remain downstream.

Keep OperationalError: MfmValue in the inward capability interface. ClassifyError currently belongs
to Program, which depends on capabilities; enforce I::OperationalError: ClassifyError at Program and
Runtime association instead of creating a reverse dependency or moving the classifier for convenience.
A State returns its original typed domain failure, an adapter returns its original typed operational
failure, and Runtime applies that original's classification for recovery. No second classifier,
product-error remapping layer, or extra classification method is added to the implementation.

Do not require an additional TransactionOutcome enum/result hierarchy. Use the existing distinction:

| Observation | Existing owner and outcome |
| --- | --- |
| Transaction remains unresolved | Adapter reports Pending; Runtime retains the acknowledged command |
| Accepted native evidence is available | The selected implementation projects the fixed typed evidence; the State interprets it |
| The State finds semantic success or failure | State returns ProposedStateOutcome with its exact output or domain failure |
| Provider/transport operation fails | Adapter returns its exact operational original; Runtime classifies that original |

A discriminator inside evidence is justified only when the consuming State needs to distinguish
specific authenticated observations. An authenticated settled revert can be evidence interpreted
as a domain failure; transport failure or uncertain submission cannot be relabeled as Rejected.
Evidence exposes checked transaction identity, relevant settlement facts, and available failure
meaning while retaining the exact native source. Do not discard rejection details behind a generic
reason string, duplicate receipts into another authoritative result, or add scalar-product variants
to a supposedly universal transaction capability. Product requests stay in State inputs. Common
identifier/completion contracts require a reviewed definition; a network tag or an opaque string
alone is not that definition.

The method signatures remain a cross-crate proof obligation. Projection must receive the original
canonical native evidence as well as any decoded value it needs, so retention clones the immutable
Object rather than re-encoding the original or requiring every native Rust type to implement Clone.
Semantic evidence binding must verify the carried native reference against that authoritative source.

Apply the same separation to Reads: ReadCapabilityContract retains fixed semantic Intent/Evidence;
ReadImplementation<C> owns NativeIntent, NativeEvidence, OperationalError, exact binding, and pure
qualification/projection. Preserve duplicate-safe Read restrictions and original request identity.
A Read has no new Effect-style settlement transition: retain its native evidence in the existing
Read outcome/failure representation and derive its semantic view within the existing execution path.
Do not claim evidence was acknowledged when projection or append failed before that boundary.

Native supporting States may use a narrow protocol whose semantic and native types are identical.
Use identity qualification/projection through this same association mechanism; do not preserve a
second native-only engine or require wrapper States. One concrete type may implement both contracts
where appropriate. The existing State, PureState, ReadState<C>, and EffectState<C> execution model
remains; operational-error association moves from C to its selected implementation I.

### 6.3 Exact preparation behind a fixed command

A common prepared command must bind to one already-prepared native transaction. Its proposed value
shape is:

```text
SubmitPreparedTransaction
  prepared:
    checked semantic intent
    exact selected implementation identity
    public binding identity
    inline exact native preparation descriptor
```

Use concrete typed values throughout authoring, native preparation, qualification, and interpretation.
Use existing Object erasure only where necessary to retain heterogeneous native data across a fixed
semantic/persistence boundary. The private native preparation field retains exact schema, canonical
bytes, and content identity. It is not the representation for ordinary product fields or configuration,
and exposes no arbitrary field lookup, unchecked casting, or caller-built context bag.

Prefer the existing Object inside a purpose-specific checked owner. Do not introduce a general
QualifiedValue framework, native cache, second serializer grammar, or type-witness ladder. A shared
wrapper is justified only by actual shared invariants, not the presence of several private Object
fields. Artifact/contract-locator identities are qualified by their own exact contracts and ledger;
they do not inherit the transaction implementation version merely because a receipt produced them.

Capability-owned preparation consumes the actual typed predecessor input, derives the effective
request, and constructs a checked prepared value. Its constructor establishes action, target,
current argument values, binding, and native command correspondence, then canonicalizes the native
descriptor once. Consumers supply requests, not native descriptors. The designated State uses the
resulting checked capability command without inspecting native fields or repeating correspondence
validation.

Carry one authoritative effective request through preparation and execution. Do not retain
independently editable copies of the effective value and an unrelated prepared semantic request.
Preserve required sibling domain context and the distinct original requested value through checked
contracts. The concrete prepared-input representation must establish these relationships on
construction and applicable decoding; adding a duplicate validator to Configure is not the solution.
Preserve concrete construction causes and the existing no-retry behavior when first encoding fails;
no opaque side copy is retained.

For example, the Configure preparation owner constructs a command from current effective value 84
and qualifies it before its owning reservation/submission boundary. A native descriptor encoding 42
cannot become a checked prepared value for that request. This check belongs to capability-owned
preparation and its checked contracts, not Configure or a new Runtime callback comparing S::Input
against I::NativeCommand. The same owning contracts enforce applicable invariants when stored
prepared values are decoded.

For EVM, retain the public PreparedEvmTransaction descriptor, including reserved command and
transaction hash. Signed transaction bytes remain under EvmTransactionAuthority's existing custody.
The adapter qualifies the descriptor and retrieves the exact retained wire through that explicit
boundary. A bare ContentRef is insufficient: Store has no arbitrary value lookup, and deterministic
States cannot fetch hidden preparation facts. No new blob store or implicit history lookup is added.

Private fields are API discipline, not authority. Structural Object decoding checks canonical bytes,
hash, grammar, and size; exact descriptor admission, typed decoding, and the owning checked value's
invariants remain necessary before native use. Runtime retains its existing schema/input admission
and command/evidence binding checks. The already-associated native component validates its protocol
facts, and the adapter checks actual retained authority, epoch, sender, signing purpose, and wire.

Cold reconstruction uses the retained exact implementation and the same checked contracts. It does
not select a network again or trust arbitrary decoded native bytes. The implementation proof must
show forged/mismatched prepared values are rejected at the owning boundary without weakening these
checks. Do not solve that proof by adding the rejected generic State/implementation Runtime hook.
The compiler cannot certify live custody; that remains the explicit authority boundary's check.

The native owner's checked extraction must receive the relevant semantic intent and binding carried
by the fixed command, not only the native descriptor. An otherwise-valid native descriptor for 42
paired with a semantic intent for 84 must fail this contextual check on cold/native use. A matching
label or unattested request hash alone proves no correspondence. The concrete prepared contract must
retain the facts needed for that owner-local check without an arbitrary payload bag or a Runtime
S::Input/I::NativeCommand comparison hook. Checked construction alone is insufficient for decoded data.

Cold checks preserve those same ownership boundaries. A decoded request/native mismatch is
rejected by the selected capability's checked extraction; Configure need not decode calldata or
repeat that check. This is local contract validation, not a new claim that Runtime authenticates
arbitrary coherent substitutions of the complete run history.

The semantic command commits the complete native descriptor. Its command identity and EffectId
must stay linked to the exact native command identity; neither can be silently substituted for the
other at a custody boundary. Pending/recovery attempts reuse that acknowledged command and native
authority. They cannot reprepare from changed config, select another network, or acquire a fresh
transaction merely because the semantic State type is unchanged.

### 6.4 Typed injection preserves Deploy as the designated executable

Replace mutable before/after emission with typed prefix and successful suffix definitions owned by
the resolved implementation for the selected State/capability pair:

```text
prefix:      ExpandedInput -> State::Input
designated:  State::Input  -> State::Output
suffix:      State::Output -> ExpandedOutput
```

The domain declares endpoints without an expansion callback; the implementation declares the
surrounding source types. Proposed signature shape:

```rust
pub trait EffectSelection<C>: EffectState<C>
where
    C: EffectCapabilityContract,
{
    type ExpandedInput: MfmValue;
    type ExpandedOutput: MfmValue;
}

pub trait InjectEffect<S, C>: EffectImplementation<C>
where
    C: EffectCapabilityContract,
    S: EffectSelection<C>,
{
    type Prefix: AuthoringSource<Input = S::ExpandedInput, Output = S::Input>;
    type Suffix: AuthoringSource<Input = S::Output, Output = S::ExpandedOutput>;

    fn surround(binding: &Self::Binding)
        -> Result<(Self::Prefix, Self::Suffix), ProgramError>;
}
```

The current EVM setup is its public binding. Reuse it instead of introducing another generic setup
layer without a concrete requirement. Native command options and product arguments remain checked
input data. `surround` constructs typed sources during expansion and performs no IO; construction
errors must preserve their causes. Read injection follows the same endpoint relationship. Prefix,
State, and suffix are emitted by the framework, so implementations cannot omit or replace the State.

The State/capability selection declares its public expanded endpoints independently of the selected
implementation. Every supported implementation must prove those endpoints and the same designated
State's fixed raw endpoints. Runtime configuration cannot change a Rust associated type. A profile
with incompatible alternatives is not a valid implementation of the same selection.

For EVM deployment, the intended sequence is:

```text
DeploymentRequest
    -> ReserveEvmNonce
    -> PrepareEvmTransaction
    -> Deploy
    -> ProjectEvmDeploymentOutcome
    -> DeployedContract
```

All supporting implementations are EVM-specific; Deploy is not. Adapt the preparation State's output
to the common prepared-input contract, or add an explicit native-to-semantic projection if a separate
step is necessary. Do not hide it in an adapter or pretend the current native context already has
the required type. Configure follows the same pattern with its own semantic action and projection.
Other implementations inject the sequence their protocol requires, not a universal nonce workflow.

For the modified lifecycle, the execution order is:

```text
Add produces effective value 84
    -> capability-owned preparation receives that current typed input
    -> required supporting States construct and retain checked preparation
    -> Configure executes using the checked capability command
    -> capability implementation projects accepted native evidence
    -> Configure interprets the semantic response
```

Expansion selects and inserts these implementations before execution; their executable supporting
States consume 84 during execution, after Add. Saying "Configure calls the capability with 84"
describes the authored selection. Its request must reach capability-owned preparation before the
designated Configure State executes, not first be created inside Configure after its prefix.
This requires no additional State hook. Every injected State retains its own existing execution,
persistence, and recovery boundaries.

The displayed suffix is subject to the semantic audit, not an obligation to retain redundant work.
If Deploy's interpretation already checks success/rejection and constructs the final checked output,
remove a native suffix that merely repeats those decisions and use typed identity. Preserve any
separate useful work explicitly. Audit the old ProjectEvmTransactionOutcome failure/context behavior
and migrate its tests and recovery semantics in the same breaking cutover before retiring its boundary.
No acknowledged history is rewritten. Other genuinely needed injected boundaries remain independent.

The framework inserts the selected executable State exactly once. Prefix and suffix retain separate
policy/checkpoint scopes without one-State Operation wrappers. Empty expansion is typed identity
only for equal endpoints. A suffix runs on success, not as a finally handler. Every emitted State
retains its exact ABI, original failure, persistence boundary, and recovery eligibility.

Preserve one injection protocol and one typed source compiler. A runtime adjacency check inside the
old mutable callback is insufficient. The proof must include incompatible prefix/suffix compile
failures and the same non-generic Deploy executed with two distinct native implementations, both
standalone and nested. Existing TransactionRecipe/slot machinery may remain inside native support
where useful; consumers neither provide it nor use it to specialize Deploy.

Move EvmTransaction wrapper validation into checked selection and its owning deterministic
boundaries before deleting it. Root public setup resolves under section 9.2; the implementation
supplies setup for its supporting stages. No consumer configures hidden nonce/preparation stages.
Local binding/action mismatch remains Internal with no provider call or append; it cannot become
an integrity-block event without authenticated external evidence.

### 6.5 Native settlement is authoritative; semantic evidence is a view

Retain only exact native settlement evidence as the authoritative settlement object. Use the
existing Effect settlement/interpretation transitions:

1. Decode and qualify I::NativeEvidence against the admitted exact command, EffectId, selected
   implementation, public binding, and semantic intent.
2. Append the native settlement using the existing atomic acknowledgement boundary.
3. In AwaitingInterpretation, decode the retained native evidence and deterministically project
   C::Evidence under that same implementation identity; qualify it and call Deploy::interpret.
4. Reconstruct the same view during cold qualification/interpretation without live configuration.

All checks required to admit a settlement remain before its append. If a check requires the semantic
projection, run the same pure projection there and discard that view; interpretation may reproduce
it later. Do not postpone integrity qualification merely to make the sequence look simpler.

Projection failure after acknowledgement preserves the native settlement and known head, reporting
an InvocationFailure. It does not authorize resubmission, command replacement, or a fabricated domain
failure. Projection before acknowledgement similarly makes no claim that evidence was recorded.

For Observe, the selected capability implementation validates native evidence against the requested
contract and observation requirements, then exposes a checked semantic value through ContractRead.
It owns native decoding, target/evidence binding, and protocol guarantees. Validate compares that
observed value with the retained effective request; it does not repeat network integrity checks.
The response is the capability's exact typed semantic contract, not a universal result bag. Native
originals and operational causes remain retained through normalization.

Native evidence and the derived view have different schema/reference identities. Failure-origin
metadata must identify the native original and the semantic view supplied to interpretation without
labeling either as the other. State-failure audit retains the source needed to reconstruct that view
and the exact selected implementation identity. A product requiring a separately committed semantic
view must justify that persistence contract explicitly; it is not a default second authoritative copy.

If a native suffix needs native evidence, the fixed State output carries the framework-owned
qualified native envelope alongside the semantic result. Deploy can inspect semantic fields and
preserve this envelope without interpreting native fields. The suffix decodes through the exact
implementation contract; it never performs hidden Store or historical lookup. Native retention,
projection, output, and failure-report capacity checks remain applicable to complete representations.

### 6.6 Resolve and bind everything needed before execution

A downstream production profile supplies supported capability-to-implementation associations once
per capability and explicit binding role. It does not repeat a configuration switch for every State
or maintain an executable-registration list. Semantic domain contracts do not import or enumerate
all native implementations. Reuse the closed Choice representation for alternatives; no second
alternative algebra or runtime plugin registry is added.

Separate structural support from checked selection. The proposed minimal relationship is:

```rust
pub trait CapabilityFamily<C, Role> {
    type Choices;
}

pub trait Resolve<Root: MfmValue, C, Role>: CapabilityFamily<C, Role> {
    fn resolve(root: &Root) -> Result<Self::Choices, ProgramError>;
}
```

A selected leaf names a concrete implementation and its checked public binding; native owners
provide any required supporting setup. Framework bounds require every declared alternative to
supply valid typed injection for the selected State. That is a static guarantee. A profile with
an implementation incapable of the advertised State is invalid; expose an intentionally narrower
family/role rather than silently falling back. Supported configuration values still need checked
resolution for account, network, action, and options before publication.

The binding role may default for the ordinary two-argument Effect<S, C> surface. Multiple accounts
or networks use explicit roles. Shared roles must agree across the selected workflow. A new State
using an existing family needs actual native support for its semantics, not another profile entry
or parallel registration tree. Supporting injected States have already-selected native bindings;
they do not trigger another product-profile lookup for hidden preparation stages.

Operation configuration and checked root input supply the supported values and defaults already
required by the product. Their agreement is checked before publication. No with_selection(),
with_policy(), generic scoped override wrapper, or extra builder configuration object is added.
Ordinary State data remains in its admitted input. Native preparation derives future commands from
current predecessor values, not from stale root values. A binding discovered through execution
requires discovery followed by a new Program, not a mutable or unresolved binding in an admitted run.

Compilation and assembly resolve implementations, public bindings, native/semantic codecs,
injection, and exact callbacks before execution. Runtime receives a fully associated executable
Program. It executes those callbacks; it does not consult configuration, choose a network, rebuild
an injection plan, or resolve an abstract capability during progression. Cold assembly associates
retained exact descriptors with available code and explicit live handles without changing the plan.

Depth, expanded-State limits, and atomic assembly staging remain. Missing implementations or live
bindings fail before admission. Construction alone does not prove future execution values: their
checks remain with the selected component that constructs or consumes them.

### 6.7 Minimal durable identity and provenance

Extend the existing Program execution descriptor rather than introduce another registry or record:

| Descriptor facts | Owner |
| --- | --- |
| State implementation, input/output/failure contracts | Existing State ABI |
| Semantic capability and command/intent/evidence contracts | Fixed State-facing interface |
| Concrete implementation and native command/intent/evidence contracts | Selected implementation ABI |
| Exact operational-error contract | Selected implementation original |
| Public binding and policy | Resolved existing occurrence fields |

The versioned implementation identity commits native extraction, evidence qualification, and
projection semantics. Changing those semantics changes that identity even if schemas stay the same.
Do not add separate projection/injection IDs: projection belongs to the implementation contract,
and the actual expanded declarations, bindings, and policies already commit injection into Program.
Replace the old combined capability/error identity scheme in the same cutover, rather than retaining
parallel schemes. Rust TypeId and allocation identity are never durable identities.

Keep the existing EffectCall/Settlement structure:

```text
EffectCall.command = semantic command including its inline native descriptor
Settlement.evidence = exact native original
EffectId = H(RunId, ProgramRef, ExecutionPosition, semantic command reference)
```

Do not persist another native-command field or a second semantic settlement. At the native adapter
boundary, distinguish the semantic command reference from the exact native reference obtained from
its canonical descriptor; do not silently substitute one for the other. Pending reconciliation
retains the same command and authority regardless of later configuration changes.

Identify a derived semantic evidence view by native source reference, exact implementation reference,
and semantic evidence contract. Do not add a mandatory persisted view hash and its encoding failure
point. If checked inspection requires that view's own ContentRef, produce it only after successful
projection/encoding and report it unavailable on failure. Never use the native evidence reference
as though it identified the semantic view.

Reuse existing StateCall and failure context to retain origin. Self-contained reports may expose
resolved descriptor facts, but internal records need not duplicate every schema and binding field.
Suffix custody uses those same retained facts in its State input, with no historical lookup.

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

### 7.2 Required default inheritance

Resolve supported policy declarations from outer scope to the occurrence:

```text
framework fallback
    -> enclosing Operation defaults
    -> nearer maintained child Operation or injection-scope policy
```

Every expanded State, including native reservation/preparation, inherits applicable Operation
defaults unless a maintained component already declares a required more-local policy. Importing
State types does not import every Operation policy that happens to use them. Reusing a child twice
creates distinct occurrence scopes.

Keep one typed policy representation for these required declarations. Unspecified differs from
explicit zero. Handler, parameters, and checkpoint targets form one replacement unit; allowances
inherit independently. Preserve existing explicit component policies and scoped recovery behavior;
do not introduce a new optional caller API for arbitrary State-occurrence overrides.

No with_policy(), with_selection(), or speculative Scoped wrapper is part of this refactor. Existing
Operation configuration/defaults meet the accepted consumer scenarios. Add other controls only when
a concrete demand justifies their semantics and owner.

Per-occurrence recovery allowances remain separate from the Program-wide admitted recovery budget.
Local defaults cannot enlarge that budget or authorize a forbidden restart. These are durable safety
policies, not execution deadlines or limits on the duration of a Runtime invocation.

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

These exports are proposed. Production supplies the non-generic semantic State implementations,
their checked public contracts, and native implementations of their capabilities. The EVM domain
entry may re-export shared State types; it must not create EVM specializations of Deploy or Configure.
Neither import nor selection requires an existing Operation instance.

The 42/84 acceptance case selects an EVM implementation and supported EVM artifact/ABI. It proves
that product behavior, not portable deployment on every network. Shared capability commands,
prepared envelopes, State endpoints, and semantic results must live inward of native implementations.
Native facts remain exact qualified values. Existing EVM-specific fixture contexts are migration
inputs, not an already-specified network-independent schema. Moving exports alone does not establish
that schema; section 18 requires a concrete shared-contract proof. EVM-specific checked report
accessors may project native facts without making the generic State inspect them. Public scalar
contracts also belong to the shared domain; they must not require EvmU256 in State implementations.
The examples use their canonical decimal Display representation for assertions and their is_zero()
predicate for semantic validation. These are requirements on the proposed checked scalar contract,
not claims that the existing EVM fixture already provides the shared type.

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
input and the maintained Operation configuration. There is no repeated concrete binding argument
on each State definition.

### 8.2 Public expanded contracts

| Selection | Input | Output | Responsibility |
| --- | --- | --- | --- |
| Deploy | DeploymentRequest | DeployedContract | Create the supported contract and retain checked deployment facts |
| CheckedAddConfigurationValue | DeployedContract | DeployedContract | Add the admitted increment to the effective scalar, with checked overflow and sibling preservation |
| Configure selection | DeployedContract | ConfiguredContract | Native preparation constructs the command from the current effective scalar and created address; Configure executes against the semantic capability and the suffix retains checked call facts |
| Observe | ConfiguredContract | ObservedConfiguration | Consume the capability's checked semantic value for the configured target at the required configuration receipt anchor |
| Validate | ObservedConfiguration | ValidatedConfiguration | Compare the checked observed value with the retained effective request; do not repeat native evidence validation |
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

The Deploy selection preserves effective 42. Addition returns a new checked context with effective
84 and the same requested value and deployment facts. The Configure selection's native preparation
prefix deterministically constructs calldata and the complete nonce-free command from that current
context before reservation or acknowledgement. This is native preparation behavior, not calldata
construction in the non-generic Configure State or in the compiler's root-input projection. That
root still contains the original 42. Native reservation preparation may perform the deterministic
construction, or an explicit preceding Pure State may supply it if a separate step is necessary.
Configure then executes against its prepared semantic input; Observe and Validate use the retained
facts and Report returns observed 84. The cross-crate proof must establish this data flow before any
reservation, including rejection at the appropriate original-failure boundary.

The complete acceptance flow is:

```text
Requested 42 -> Add 42 -> Effective 84
    -> capability-owned preparation constructs the native request from 84
    -> Configure executes configuration
    -> Observe receives semantic value 84 from its capability
    -> Validate compares observed 84 with retained effective 84
    -> Report
```

Configure contains no native-field inspection or duplicate request/native validation. Observe
consumes the capability's checked semantic response, preserving original evidence for audit.
The caller supplies neither encoded calldata nor prepared transaction facts.

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

    fn read<S, C, I>(&mut self) -> Result<(), Self::Error>
    where
        C: ReadCapabilityContract,
        I: ReadImplementation<C>,
        I::OperationalError: ClassifyError,
        S: ReadState<C>;

    fn effect<S, C, I>(&mut self) -> Result<(), Self::Error>
    where
        C: EffectCapabilityContract,
        I: EffectImplementation<C>,
        I::OperationalError: ClassifyError,
        S: EffectState<C>;

    fn handler<H: Handler>(&mut self) -> Result<(), Self::Error>;
}
```

There is no root-map callback. Requirements cover raw and expanded value contracts, original
failures, semantic and native intent/command/evidence, implementation-owned operational errors,
qualification/projection functions, and policy parameter codecs. The compiler
retains concrete types until the requirement and corresponding descriptor have been emitted.
Runtime implements the receiver by extending its existing exact ABI tables and conflict checks.
Association keys include the selected implementation and public binding, not just the shared C.
One Deploy State ABI may associate with several admitted implementation ABIs without collision or
ambiguous adapter selection. Do not add a parallel semantic registry beside the existing assembly.

Generic methods use static dispatch, not a `dyn` visitor. A Program-only compiler receives the same
production profile type and uses a no-op receiver. Compile-time expansion and cold type inventory
use the same framework-owned structural
definition with explicit traversal modes. Components do not maintain independent compilation and
registration trees or expose a second `register` method. The feasibility proof must demonstrate
that the shared structure is sufficient for both modes, including injection and policy choices.

Use one sealed structural traversal with mode-specific leaf bounds: compilation needs the profile's
Resolve<Root, C, Role>; inventory needs only CapabilityFamily<C, Role> and typed injection support.
Inventory never constructs fake root input or calls resolve/surround. Tuples, Operations, choices,
and repeated bodies share their structural walk; only the selected-versus-inventory leaf handling
differs. Keep these traversal details internal rather than exposing another visitor DSL.

This mechanism removes duplicate executable-registration knowledge. It does not remove Runtime's
canonical Object representation. Object retains canonical bytes and content identity across
heterogeneous continuation and persistence boundaries; exact descriptor admission and native
decoding still qualify values at execution and cold-load boundaries. Typed authoring and public
results do not imply a second native continuation representation or an unchecked Any bag.

Compilation and association errors retain concrete source information through the receiver's
associated error. Do not collapse them into InvalidContract or a formatted string.

### 9.2 Runtime-builder compilation

The proposed builder selects a production-owned set of supported implementations:

```rust
let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
```

EvmContractCapabilities is the proposed production profile for the EVM acceptance case. A product
supporting several networks supplies its own maintained profile of supported alternatives; checked
configuration chooses the implementation during expansion. Consumers select that profile, not an
executable-registration list. NoCapabilities is the framework profile for Pure-only sources.

The profile defines structural supported alternatives and pure checked selection separately so
cold inventory does not need configuration. Each alternative names a concrete implementation and
its typed injection for the same State/capability selection. Their expanded endpoints must agree.
The exact resolver/projection trait bounds are a cross-crate proof requirement; a profile must not
hide a second lowering routine or duplicate its alternatives in a registration method.

The proposed integrated method on the profile-typed builder is:

```rust
fn compile<S: AuthoringSource>(
    &mut self,
    entry_point: EntryPointId,
    source: &S,
    input: &S::Input,
    limits: ProgramLimits,
) -> Result<Program<S::Input, S::Output>, CompileError>;
```

During expansion, compilation resolves the implementation, public capability setup, and supported
policy values from the actual checked root input being committed and supported Operation
configuration under section 6.6. There is no required additional concrete binding argument
on the Operation definition. A selected leaf requires a typed projection from that root input,
not from an unavailable future State input. Production owns projections for its supported public
components; consumers do not write per-injected-stage plumbing.

The private traversal must carry root-type-aware bounds for these projections. AuthoringSource's
Input/Output types alone do not prove that every nested State's implementation/setup can be resolved.
The profile and typed traversal must prove the supported associations and equal expanded endpoints.
Injection derives its supporting setup from the designated implementation's resolved setup. If a binding
can only be learned through execution, use discovery and compile a subsequent Program; do not
leave an unresolved or mutable binding inside an admitted Program.

Exact projection signatures, profile selection, multi-binding typed roles, and input-dependent
repetition coupling must be proved together. Do not fill this gap with a generic
setup bag, an unqualified second configuration that can disagree with C0, or the removed arbitrary
input-check callback. Resolved public execution parameters are retained under section 6.6.

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

Expansion chooses implementation types and public bindings; the Runtime receiver associates them
with explicitly supplied live resources. Provider/signing/custody acquisition remains downstream.
Native adapter objects may be constructed during assembly from supplied handles, but Program/domain
compilation performs no provider calls, key acquisition, or connection discovery. Application's
configuration match for resource acquisition must not duplicate State expansion. Unused bound
resources grant no execution authority. A mismatched or missing selected binding fails qualification.

The transaction binder retains epoch, sender, signing-purpose, and binding checks. Secrets and
handles never enter selections, Program, admitted context, Journal, reports, or diagnostics.

### 9.4 Cold association without configuration custody

A fresh builder can install the executable types declared by a maintained definition:

```rust
builder.associate::<ContractDeploymentLifecycle>()?;
```

This is proposed type-inventory association using the builder's production profile. It requires
no initial input, scalar values, concrete
public setup, handler parameter values, or deleted run configuration. It installs exact State,
capability, handler, and value implementations. Public live bindings remain explicit builder
inputs; the retained Program supplies its acknowledged binding identities and parameter Objects.
Cold read/resume qualify those retained descriptors against the installed implementations and
available live bindings.

Inventory visits the profile's supported implementation/injection alternatives, all typed Choice
alternatives, semantic/native codecs and projection functions, and a repeated body's types once.
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
failures retain the selected implementation's separate exact contracts. The execution failure envelope
identifies which original occurred and retains its originating input, intent/command, evidence,
and execution position according to the current causal contract.

Native settlement evidence is the retained original; the semantic evidence supplied to a State is
a deterministic view qualified under the selected implementation. Update failure-origin metadata
and checked report access to distinguish their schemas/references. Do not relabel a native object
as C::Evidence or replace native operational errors with a common TransactionEffect error. Audit
existing evidence-identity commitments before removing any field; required source facts remain
available after cold reconstruction and after a projection failure.

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
) -> Result<ExecutionResult<O>, InvocationFailure>;
```

Program already carries its entry point, structural/capacity limits, resolved recovery policy,
expanded sequence, implementation associations, and input commitment. Runtime checks its exact
association and input before admission. Execution never invokes an Operation constructor, capability
resolver, or injection callback. It uses the already-selected code and existing engine transitions.

ExecutionResult has checked terminal success and terminal original failure only. `success()` borrows
an O for qualified success; `failure()` exposes the checked original report for terminal failure.
Its private representation excludes contradictory outcomes/observations and retains the exact RunId.
No result is manufactured from unchecked bytes. RecoveryStopped remains InvocationFailure with
unresolved Effect authority; it is not terminal domain failure or a new result category.

Do not add AttemptOptions, deadlines, wall-clock limits, a finite invocation progression budget,
configurable polling controls, or time-exhaustion/incomplete result variants. None is required by
the accepted consumer scenario. Program capacity limits and admitted recovery allowances remain
independent existing contracts. Future timing controls require a demonstrated demand and separate
design; they are not an implementation gate for this refactor.

`execute` progresses until terminal completion, RecoveryStopped, invocation failure, or caller
cancellation. It delegates to the existing engine:

| Observation | Action |
| --- | --- |
| Runnable, AwaitingRecovery, AwaitingInterpretation | Continue the eligible existing transition; Runtime alone authorizes recovery |
| Pending Effect without RecoveryStopped | Reconcile the same retained command through the selected async native adapter |
| Durable success or terminal failure | Return its checked result |
| RecoveryStopped | Return the existing unresolved-authority failure; later explicit resume may reconcile |
| Store/internal/invocation failure or ambiguous acknowledgement | Return its causal failure and exact recovery identity; do not invisibly retry admission/append |
| Caller cancellation or disconnection | Preserve cancellation safety; no response or fabricated cancellation record is promised |

Pending execution must not spin on an immediately-returning Pending adapter. The native IO owner
must suspend appropriately for readiness or reconciliation backoff within its existing async call.
This introduces no Runtime timer, deadline, new waiting trait, detached worker, or polling-options
surface. Current adapter behavior needs inspection and a non-spinning/cancellation proof; do not
claim it already waits appropriately. Adapter-owned waiting does not limit the invocation's lifetime.

Preserve existing InvocationFailure::Execution with RunId, original RuntimeError, and last_observed,
and RecoveryStopped with its checked pending observation. RuntimeError::Projection retains a known
acknowledged summary separately from a failed/older observation; RecordingFailure retains the exact
candidate and definite/indeterminate disposition. Reuse those facts instead of another status enum.
An error before any acknowledged view still identifies the supplied RunId. A historical observation
never claims to be the current head, and failure to append is not audited through the failed Store.

Retain direct start/progression, read, and resume for pending inspection and deliberate manual control.
Cold results use the same exact codecs and selected projections as hot execution. Do not introduce
RuntimeDriver, another scheduler, or consumer-owned drive_to_success loops for ordinary progression.

## 12. Caller examples

These are proposed authoring/execution bodies, not runnable current tests. Imports and surrounding
async signatures are omitted. Callers supply explicit RunId, entry_point, limits, and
Arc<dyn Store>. EVM bodies also receive explicit signer, authority, transaction-provider, and
read-provider handles where needed. These handles come from their IO/custody owners; no implicit
resource acquisition is hidden in a definition constructor.

The proposed production exports in section 8.1 supply every State, Operation, context, and codec
below except the new State in 12.5. All lifecycle examples load a checked request containing its
complete public transaction binding, requested value 42, and increment 42. The binding used for
live adapters is read from that same checked request. Compilation independently qualifies setup
projections and live association before admission.

TransactionEffect and ContractRead are shared semantic interfaces. EvmContractCapabilities comes
from downstream production composition and supplies the EVM implementations, their injection, and
structural inventory. NoCapabilities comes from the framework. These are proposed production exports,
not consumer-defined profiles or aliases. The existing native binders supply explicit live handles;
the compiler derives executable registration from the selected definitions and implementations.

Assertions are acceptance-test expectations. Application callers inspect the explicit outcome;
absence of success is not itself a decoded failure or proof of terminal completion.

### 12.1 One existing Pure State

Production supplies CheckedAddition, CheckedAdd, its checked scalar output, and overflow failure.
The lifecycle addition reuses the same checked arithmetic while preserving its context.

```rust
let input = CheckedAddition::new("42", "42")?;
let state = Pure::<CheckedAdd>::default();

let mut builder = Runtime::builder::<NoCapabilities>(store)?;
let program = builder.compile(entry_point, &state, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input).await?;

let value = result.success().expect("expected terminal success");
assert_eq!(value.to_string(), "84");
```

The definition has no binding or setup argument. Its input constructor parses checked scalars;
compilation supplies exact State/value requirements. There are no root maps or separately
maintained executable-State registration entries.

### 12.2 One existing injected Effect State

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let input = config.initial_input()?;
let binding = input.transaction_binding().clone();
let state = Effect::<Deploy, TransactionEffect>::default();

let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
register_evm_transaction_adapters(
    &mut builder,
    binding,
    signer,
    authority,
    transaction_provider,
)?;
let program = builder.compile(entry_point, &state, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input).await?;

let deployed = result.success().expect("expected terminal success");
assert_eq!(deployed.requested_value().to_string(), "42");
let created_address = deployed.deployment().created_address();
```

DeploymentRequest is the public input, not prepared facts. Compilation resolves the native
implementation and public setup and associates reservation, preparation, Deploy itself, and
projection. Deploy executes against TransactionEffect with its fixed input/output ABI. No Operation
wrapper, State getter object, or concrete binding in the State definition is required.

### 12.3 One maintained Operation definition

ContractDeploymentLifecycle is the reusable type definition in section 5.1. It owns its fixed
functional sequence and LifecycleDefaults; this caller does not redeclare either.

```rust
let config = ContractWorkflowConfig::decode(&config_bytes)?;
let input = config.initial_input()?;
let binding = input.transaction_binding().clone();
let operation = ContractDeploymentLifecycle::default();

let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
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
let result = runtime.execute(run_id, program, input).await?;

let report = result.success().expect("expected terminal success");
assert_eq!(report.requested_value().to_string(), "42");
assert_eq!(report.effective_value().to_string(), "42");
assert_eq!(report.observed_value().to_string(), "42");
```

Supported configuration values specialize compilation, not the static definition's account or
endpoint. Runtime receives Program, never the authoring definition as execution input.

### 12.4 Existing Operations and States in one tuple

Production exposes ConfigureAndObserve as another definition:

```rust
pub type ConfigureAndObserve = Operation<
    (
        Effect<Configure, TransactionEffect>,
        Read<Observe, ContractRead>,
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
    Effect::<Deploy, TransactionEffect>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    ConfigureAndObserve::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
));

let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
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
let result = runtime.execute(run_id, program, input).await?;

let report = result.success().expect("expected terminal success");
assert_eq!(report.requested_value().to_string(), "42");
assert_eq!(report.effective_value().to_string(), "84");
assert_eq!(report.observed_value().to_string(), "84");
```

Unspecified outer policy fields inherit framework fallback; explicit maintained child defaults
resolve by section 7.2. The all-State variant in section 5.2 has the same acceptance
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
        if input.effective_value().is_zero() {
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
    Effect::<Deploy, TransactionEffect>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    Pure::<RequireNonZeroConfiguration>::default(),
    ConfigureAndObserve::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
));

let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
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
let result = runtime.execute(run_id, program, input).await?;

let report = result.success().expect("expected terminal success");
assert_eq!(report.observed_value().to_string(), "84");
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
qualification. Runtime owns semantic/native executable association and progression. Shared domains
own semantic capability contracts and non-generic States. Native domain modules contain the
deterministic portion of capability implementations: native contracts, preparation, typed injected
States, protocol validation, and evidence projections. They depend on shared contracts; they are
not separate network-specific implementations of Deploy or Configure. Live adapters provide the
IO portion of those same capability implementations. This crate separation preserves inward
dependencies without splitting conceptual ownership. Shared domains must not depend on a global
enum of native implementations. Downstream composition owns profiles connecting supported
implementations. Domains also own Operations, checked
configuration, contexts, and product reports. Live crates bind reusable
platform providers/signers/custody. Journal remains the exact opaque append-only frame owner;
Store remains mechanical admission/latest/probe and atomic exact-head append.

Removing root failure fields and adding exact capability implementation associations changes Program
identity and Runtime payload/report schemas. Native command/evidence qualification, operational-error
association, and native-versus-semantic evidence provenance must cut over together. Update
all producers, decoders, validators, transports, fixtures, and docs together. Choose schema versions
from the actual changed contracts during implementation; do not invent a speculative parallel wire.
Reject superseded data under the repository's clean-cutover policy. Do not rewrite acknowledged
history, provide a migration reader, or claim old runs remain executable with unavailable ABIs.

Journal's opaque envelope and Store's physical schema need not change solely because Runtime's
payload changes. Reassess them only if their owned format or capacity contract actually changes.
PostgreSQL admission snapshots, locked append transactions, and ambiguous COMMIT semantics remain.

No new third-party dependency is required by this design. Place shared contracts inward of native
domains, moving modules or introducing a justified workspace crate if required to avoid dependency
cycles; the placement must not pull Runtime or live IO inward. No cryptographic, keystore Send/Sync,
transaction replacement, or mainnet finality redesign is authorized. The current EVM receipt policy remains
limited to the pinned non-reorging development environment. Maintained production library code
does not by itself make that policy suitable for shipping transaction composition.

## 15. Complete deletion and migration scope

| Existing machinery | Target treatment |
| --- | --- |
| Operation-only `expand_program` root | Replace with the neutral source compiler; standalone selections use it directly |
| Public mutable `OperationExpansion` DSL | Remove; retain useful private draft/relocation logic behind typed source traversal |
| Mutable capability prefix/suffix emission | Replace with typed prefix/suffix sources; keep capability ownership and designated insertion |
| State-facing capability coupled to native operational errors | Separate fixed semantic contracts from implementation-owned native commands/evidence/errors; migrate one association and execution path |
| Concrete EVM capability arguments in ordinary lifecycle selections | Select TransactionEffect/ContractRead; resolve exact implementation and binding at expansion |
| Proposed Deploy<C>, BindEffect, or StateDefinition workflow replacement | Do not implement; Deploy remains the same concrete executable State |
| EVM execution State standing in for Deploy | Move useful native mechanics to the implementation/supporting States; emit Deploy itself as the designated Effect |
| Adapter association by shared capability/binding alone | Qualify exact implementation identity and binding in the existing tables; no second registry |
| Evidence treated as both native original and semantic view | Retain native originals, project semantic views, and update audit provenance and suffix custody together |
| Generic Runtime State/implementation product-qualification hook | Do not add; native preparation and checked value owners establish correspondence, while existing admission/binding/authority checks remain |
| Universal TransactionOutcome or product-action enum in TransactionEffect | Do not add without a concrete evidence-contract need; retain existing Pending, State outcomes, and original operational failures |
| Proposed selection/policy modifier APIs and Scoped wrapper | Remove from this target; retain required maintained Operation configuration/defaults only |
| AttemptOptions, deadlines, wall-clock/progression limits, polling knobs, incomplete execution results | Remove from this target; ordinary execute returns terminal results or existing invocation failures and supports caller cancellation |
| Separate projection/injection IDs, native-command copy, or persisted semantic-view hash | Do not add; reuse exact implementation identity, expanded declarations, inline native descriptor, and reproducible view provenance |
| Native outcome suffix duplicating Deploy interpretation | Audit distinct semantics, retire redundant work/boundary and migrate coverage in the same cutover; retain useful suffix support |
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
repetition, cold type inventory, fixed-State semantic/native capability boundaries, and the product
failure-report migration. Do not start broad API or persistence deletion merely because the five
fixed lifecycle expressions look plausible.

1. **refactor typed authoring and executable association**: implement neutral sources, typed
   tuple selections, scoped Operation defaults, implementation selection and typed injection,
   input/plan qualification, and one requirement receiver. Cut over semantic/native capability
   contracts, implementation-owned errors, prepared envelopes, evidence projection/provenance,
   and exact implementation descriptors together with their Runtime consumers and cold decoders.
   Keep Deploy as the actual non-generic State. Migrate authoring and association consumers together.
   Delete the mutable DSL, wrapper-only Operations, and duplicate executable lists. Update current architecture/design
   and consuming tests in this commit. Include failure-contract removal here if the API and wire
   cannot coherently change separately; do not create a temporary mandatory failure algebra.
2. **remove aggregate root failure contracts**: only a separate commit if the preceding cutover can
   remain internally coherent. Remove root maps and mapped-root persistence, migrate actual product
   reporting requirements, and update every affected schema, transport, cold decoder, and capacity
   test together. Preserve exact originals and recovery authority.
3. **consolidate runtime execution and checked results**: accept compiled Program, progress through
   the existing engine to terminal results or existing invocation failures, and replace ordinary
   copied drivers/decoders. Add no timing/options API. Prove adapter-owned pending waiting does not
   spin and retains cancellation safety. Keep direct APIs and cancellation/ambiguity coverage. Merge with the first
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
| Fixed-State capability abstraction | Same non-generic executable State and semantic ABI with two distinct native implementations, exact native commands/evidence/errors, and different typed injection; a deterministic test implementation proves the DSL, not production support for another chain |
| Implementation selection | One family per capability/role, supported Operation configuration and shared-role agreement; unsupported choices rejected before admission; no network fallback or resolution during progression/resume |
| Prepared custody | Capability-owned preparation consumes actual predecessor output and retains one authoritative effective request; its checked construction/extraction rejects request/native mismatches, including cold decoding and wrong action/implementation/schema/binding; Configure contains no native inspection or duplicate correspondence validator; exact descriptor admission and semantic/native command identity linkage remain; no extra generic Runtime product hook |
| Evidence views | Native settlement retained before interpretation; hot/cold semantic projection equivalence; projection failure preserves known head; suffix receives native custody without hidden IO |
| Evidence provenance | Native and semantic schemas/references distinguished in State-failure reports; exact native operational originals and classification retained |
| Operation scopes | Nested precedence, explicit zero allowances, target replacement, inherited installed handler, foreign/absent/duplicate checkpoint rejection, and repeated source occurrence relocation |
| Root checks | Direct and nested injection equivalence; wrong binding/action mode rejected at the owning boundary before affected IO or append |
| Association | Injected States and custom handler included automatically; each supporting State retains independent persistence/recovery boundaries; exact generic ABI conflicts rejected; failed compilation leaves builder unchanged |
| Cold construction | Type inventory without C0/setup/parameter values after config deletion, with explicit live bindings; exact original/output decoding, supported alternatives, and nondefault handler association |
| Input-dependent planning | Portfolio repetition derives from the input being committed; count/routes mismatch cannot be admitted; empty repetition, nested same-type semantic constraints, and cumulative limits |
| Later compilation | Already-associated ABIs execute a later compiled Program; missing ABIs fail before admission; a fresh immutable assembly can add supported definitions without mutating an existing Runtime |
| Product success | Maintained 42 and composed 84, with separate fresh signer/nonce domains where required |
| Product evidence | Configuration targets created address; capability validates observation target/receipt anchor and exposes checked semantic 84 with native provenance; Validate compares it with retained effective 84; admitted input remains 42 and capability-owned preparation constructs the command from predecessor 84 |
| Product rejection | Addition overflow before configuration; observed-value mismatch; malformed/wrong ABI result; new-State zero failure retains exact rejected input and prior facts |
| Causal failures | Distinguishable nested domain/provider/authority/signer causes survive hot/cold access, with classification unchanged and no deliberately appended secrets |
| Report limits | Small-bound complete-report overflow preserves original, acknowledged head, and command authority; no production-maximum allocations |
| Execution and cancellation | Terminal execute results, stale last observation, cancellation, ambiguous admission/append, RecoveryStopped, and adapter-owned non-spinning retained-command reconciliation; no timing-limit API |
| Minimal evidence | No mandatory new outcome hierarchy; checked transaction identity and authenticated rejection detail remain available; transport uncertainty stays an original operational error |
| Suffix simplification | Any removed projection boundary has its distinct original/failure/recovery semantics audited and retained where required; Deploy owns its semantic result and errors |
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
| Fixed semantic deployment contracts can support more than one native implementation. | The product's common prepared-input, outcome, identifier, and completion guarantees are not specified by existing EVM fixture types. | Network independence could be nominal or hide incompatible guarantees. | Specify one concrete shared contract and execute the same State with two native implementations; state supported actions explicitly and reject unsupported combinations. |
| Semantic/native implementation association fits the one compiler and Runtime. | Complete implementation methods, resolver bounds, and public expanded endpoints are uncompiled. | Type erasure could bypass validation or require a second registry/engine. | Cross-crate proof of fixed State ABI, different native ABIs, typed injection, identity projection for native support, atomic assembly failure, and cold inventory without configuration or resolution during execution. |
| Prepared envelopes preserve exact command custody and authority. | Private canonical envelopes alone do not establish native qualification or custody references. | Resume could rebind or replace the acknowledged native command. | Define qualified constructor/admission APIs and identity linkage; test forged/decoded mismatches, native wire retention, cancellation, and ambiguous acknowledgement. |
| One authoritative effective request and capability-owned preparation preserve product/native correspondence. | The common prepared-input fields and cold-decoding APIs remain unproved; independently editable request/context copies could disagree. | An inconsistent prepared value could bypass the effective84 requirement or force duplicate validation into Configure. | Prove the complete Add-to-preparation data flow from actual predecessor output, required sibling-context preservation, and owner-local cold rejection of request/native mismatches (including native42 with request84); Configure must neither inspect native fields nor repeat correspondence checks, and Runtime gains no generic product-validation hook. |
| Existing evidence/outcome contracts need no universal transaction-result layer. | Checked transaction identifiers, settlement facts, and rejection observations are not yet fully specified. | A generic reason or success flag could hide facts needed by the State or audit. | Define the minimal concrete evidence consumed by Deploy/Configure; test authenticated rejection versus provider uncertainty, exact native retention, and common/native checked access. |
| Native evidence alone suffices as authoritative settlement. | Current audit metadata and product commitments may conflate native evidence with the State-facing view. | Projection failures or cold reports could lose evidence or misidentify its schema. | Audit durable evidence-reference obligations, prove hot/cold projection, known-head failure, exact original decoding, suffix envelope custody, and small-bound capacity behavior. |
| Public setup can be projected from the actual checked root input for every selected meaningful State. | Root-aware traversal bounds and multi-binding roles are not specified by AuthoringSource's endpoints alone. | A future input or ambient binding could be used, or callers could need hidden-stage setup implementations. | Prove root-input projections across crates, wrong/multiple bindings, and capability-derived reserve/prepare setup with no consumer scaffolding. |
| Input-dependent repetition can remain coupled to the committed input without arbitrary callbacks. | Portfolio's current expansion captures collection count/routes independently of C0. | Removing its validator could admit a mismatched plan, especially when nested. | Define bounded homogeneous repetition from the same checked input, preserve count/route checks before admission, and test empty/repeated scopes and nested semantic preconditions. |
| Nominal checkpoint markers preserve scope and reuse semantics. | Type-based markers have not been relocated across nested/repeated Operation occurrences. | Targets could alias, escape scope, or be rebound during inheritance. | Test duplicate/missing/foreign/terminal markers, repeated definitions, inherited already-bound handlers, typed context mismatch, and Effect barriers. |
| Cold type inventory can share the same structural traversal without setup or C0 values. | Current injection and policy construction receive values; inventory must instead cover their supported types. | Deleted config could block recovery or a second registration tree could appear. | Fresh-runtime association/read/resume with no C0 reconstruction, both Choice branches, a repeated body, injected alternatives, and a nondefault handler; unavailable exact ABIs fail explicitly. |
| Product reports can move from mapped roots to checked projections of retained originals. | Portfolio/App/transport consumers require a field and durable-identity audit. | Required facts or persistence guarantees could be lost. | Produce old-field-to-retained-source mappings and explicitly resolve any durable projection requirement before deleting its persisted payload. |
| Typed Program, staged assembly, and execution result types preserve exact acknowledgement distinctions. | Generic reconstruction, failure atomicity, and terminal result ownership remain uncompiled. | Partial assemblies, stale-head claims, or silent authority changes could result. | Test unchanged builder on failure, exact typed reconstruction, later compilation, known acknowledgement followed by projection failure, ambiguous append, cancellation, and stale observations. |
| Existing async native adapter calls can wait appropriately for pending reconciliation. | Current adapters may return Pending immediately and no Runtime deadline/polling surface is being added. | Automatic execute could busy-spin or acquire a second scheduling path. | Inspect the EVM pending path, implement needed adapter-owned readiness/backoff using its async IO boundary, and test non-spinning execution, retained-command reuse, and cancellation without timers/options in Runtime. |
| Some native outcome suffixes are redundant after State interpretation owns the semantic result. | Existing suffixes carry domain failures and may have distinct recovery behavior. | Blind deletion could lose facts, classification, or a necessary boundary. | Audit ProjectEvmTransactionOutcome and its tests; remove only duplicate work in a complete cutover and retain coverage for every remaining guarantee. |
| The lifecycle's supported ABI/artifact and reports can reuse existing EVM primitives. | The fixture is evidence, not a completed production schema. | Product configuration could claim unsupported behavior or duplicate integrity checks. | Specify checked input/intermediate/report constructors and supported ABI choices; prove 42/84, malformed return, overflow, and independent evidence assertions. |

These gates do not reopen the agreed design: Rust types define Operations, typed tuples construct
them without lambdas, public bindings resolve during compilation, defaults inherit by scope,
classification remains intrinsic, Program is the execution input, and Runtime retains exact Objects
and original failures. Deploy stays non-generic and executable; capability implementation selection
occurs during expansion, native evidence remains authoritative, and semantic projection runs within
the existing execution path. No getter collection, public arbitrary root predicate, parallel DSL, or
mandatory aggregate failure type is required. No optional selection/policy modifier, new generic
Runtime product-qualification hook, deadline, wall-clock limit, or invocation-control surface is part
of this target.
