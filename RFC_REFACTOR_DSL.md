# RFC: typed authoring and capability-owned execution

## Status and authority

This RFC specifies the replacement design for MFM's public authoring API. The concrete contracts,
capability inventory, supporting sequences, codec interfaces, and workflows below are the target.
Rust sketches omit routine derives and implementations where the surrounding contract specifies
behavior; they have not been compiled. Implementation must prove the complete contracts together,
not invent a different ownership model behind the examples.

[Design](docs/design.md) and [architecture](docs/architecture.md) remain authoritative for the current
implementation. Update their affected contracts, code, and tests together during the implementation
cutover. This RFC does not change persisted schemas or enable shipping transaction execution.

This document supersedes conflicting authoring sketches in the
[public interfaces and tests RFC](RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md),
[composition design](docs/e2e-composition-design.md), and
[extension design](docs/e2e-extension-design.md). Their independent scenarios, external oracles,
and recovery requirements remain. The [README diagram](README.md#target-workflow-for-the-dsl-refactor)
is an overview of this target, not a second specification or a claim of implemented behavior.

## 1. Consumer contract and ownership

| Activity | Caller supplies | Production/framework supplies |
| --- | --- | --- |
| Select existing behavior | Checked inputs and supported options | Public contracts, implementation selection, construction, execution, checked results |
| Compose Operations and States | Order, compatible connections, intentional maintained policy | Typed endpoints, injection, exact executable requirements, original failure propagation |
| Implement a new State | New semantics, typed contracts and implementation | The same authoring/compiler/Runtime path |

An existing-component consumer defines no State, context struct, type alias, failure mapper,
codec, or executable-registration list. Authors introducing new semantics legitimately define
contracts and implementations. Use `operation` for an Operation value and `states` for a tuple.
There is no State getter collection and no prerequisite Operation instance for selecting a State.

The acceptance product is a maintained scalar-contract lifecycle:

```text
maintained: Deploy -> Configure -> Observe -> Validate -> Report
            requested42 -> effective42 -> observed42

composed:   Deploy -> CheckedAddConfigurationValue -> Configure -> Observe -> Validate -> Report
            requested42 + increment42 -> effective84 -> observed84
```

Deploy and Configure are concrete Effect States. Observe is a concrete Read State. Addition,
Validate, and Report are Pure States. Configuration selects supported values and implementations;
production Rust types own the contracts. A supported EVM artifact and a second distinct native test
ABI prove the abstraction, not deployment support on every network.

| Owner | Responsibility |
| --- | --- |
| Shared domain States | Meaningful deterministic steps and product semantics using fixed typed capability interfaces |
| Capability implementation | All network-specific preparation, supporting States, native contracts/codecs, protocol validation, evidence projection, and operational errors |
| Native live adapter | Explicit provider, signer, and authority IO for that implementation |
| Operation | Reusable typed sequence and maintained defaults/policy interpretation |
| Program compiler | Typed traversal, capability selection, injection, policy relocation, exact descriptors and input commitment |
| Runtime | Exact executable association, continuation, local admission, durable transitions and authorized recovery |
| Values / IDs | Canonical encoding, hashing, checked identities, exact schemas and heterogeneous Object custody |
| Journal / Store | Opaque frame wire / mechanical load and atomic exact-head append |

Network-specific deterministic code is part of the capability implementation, not another Deploy
or Configure implementation. Crate separation between deterministic contracts and live IO preserves
inward dependencies without dividing that conceptual ownership. Native supporting States can be
network-specific; they still execute through the same Pure/Read/Effect machinery.

## 2. Concrete semantic contracts

### 2.1 A request-typed transaction capability

One reusable transaction capability contract is specialized by exact production request types:

```rust
pub trait TransactionRequest: MfmValue {
    type Applied: MfmValue;
}

pub struct TransactionEffect<R: TransactionRequest>(PhantomData<R>);

pub struct PreparedTransaction<R: TransactionRequest> {
    request: R,
    implementation_ref: ContentRef,
    binding_ref: ContentRef,
    native: Object,
}

impl<R: TransactionRequest> EffectCapabilityContract for TransactionEffect<R> {
    type Command = PreparedTransaction<R>;
    type Evidence = TransactionEvidence<R::Applied>;
    // Identity and semantic evidence binding follow sections 2.4 and 3.
}
```

`TransactionRequest` associates a meaningful request with the domain result of successful
transaction execution. Implement it per request contract, not per State: multiple States can use
the same request type. The request declares the relationship; the selected native capability
implementation translates and checks native evidence to produce that result. The generic `R`
specializes the request/result contract, not the network. Deploy remains a concrete State.

Keep four contracts distinct:

| Contract | Deployment example | Responsibility |
| --- | --- | --- |
| Semantic request `R` | `DeploymentRequest` | Describe the requested work in domain terms |
| Prepared command | `PreparedTransaction<DeploymentRequest>` | Retain that request together with capability-owned native preparation |
| Capability evidence | `TransactionEvidence<ContractLocator>` | Describe settlement and its checked applied result, preserving native evidence |
| State output | `DeployedContract` | Carry the domain context produced when Deploy interprets the evidence |

The deployment flow is:

```text
DeploymentRequest
  -> injected reservation and preparation States
  -> PreparedTransaction<DeploymentRequest>
  -> Deploy uses TransactionEffect<DeploymentRequest>
  -> TransactionEvidence<ContractLocator>
  -> Deploy interprets evidence and returns DeployedContract or its declared failure
```

`PreparedTransaction<R>` does not introduce another request/result relationship. It preserves the
typed request and exact native preparation across separate persisted execution boundaries, so
continuation does not reconstruct acknowledged preparation from configuration or hidden memory.
The domain State does not inspect native fields.

The applied result contains domain facts beyond the common transaction evidence. Deployment needs
a `ContractLocator`; transaction identity is already carried by `TransactionEvidence.transaction`.
An illustrative transfer request could therefore have a unit `TransferApplied` result if settlement
and the common evidence provide everything its domain needs. It need not return or duplicate a
transaction identifier as its applied result. This illustration does not add transfer functionality
to the implementation scope.

This structure is not a template required for every capability. Every Effect capability declares
its typed command/evidence contract through `EffectCapabilityContract`; every Read capability uses
`ReadCapabilityContract`. A capability with fixed input/output contracts needs no request trait or
generic parameter: `ContractRead` uses `ReadContractValue` and `ContractValueEvidence` directly.
Introduce a prepared value when intermediate preparation must cross execution boundaries, and
supporting States when that work needs its own persistence/recovery boundary. A codec alone does
not require a supporting State. Existing-component consumers reuse these production contracts;
they do not implement request traits merely to select or compose States.

The prepared value's fields are private. Its native Object is the exact public preparation
descriptor, with schema, canonical bytes, and content identity. It is not signed wire, a blob-store
pointer, or an arbitrary semantic context. The retained `R` is concrete and authoritative.

This replaces the former monomorphic TransactionEffect / unspecified SubmitPreparedTransaction.
It does not introduce `Deploy<C>`, a universal product-action enum, separate DeployContract and
ConfigureContract frameworks, or an erased semantic request. The capability is generic; the
meaningful executable States are not.

Generic value identities include the exact type-argument contracts. Distinct requests/applied
contracts produce distinct capability ABIs even when they share native implementation code.
No blanket Clone bound is imposed on native values. The concrete lifecycle requests support
cloning where their State's prepare method returns the existing checked command.

### 2.2 Scalar, identifiers, artifact, and public configuration

Preserve the existing unsigned-256 range, 0 through 2^256-1. Values owns the shared checked decimal
primitive `Unsigned256`; move reusable range/decimal logic inward and add checked addition without
a new third-party dependency. Shared-domain `ConfigurationValue(Unsigned256)` provides equality,
checked addition, decimal Display, and is_zero(). Native EvmU256 reuses the primitive with its
appropriate native contract; shared State code does not import EvmU256. Do not narrow to u64.

Purpose-specific values retain the unavoidable native heterogeneity:

```rust
pub struct LedgerIdentity { native: Object }
pub struct ContractArtifact { ledger: LedgerIdentity, native: Object }
pub struct ContractLocator { ledger: LedgerIdentity, native: Object }
pub struct TransactionIdentity { ledger: LedgerIdentity, native: Object }
pub struct ObservationPoint { ledger: LedgerIdentity, native: Object }

pub struct ContractExecutionConfig {
    transaction_implementation: StableId,
    read_implementation: StableId,
    native: Object,
}
```

All fields are private. There is no generic QualifiedValue framework, arbitrary field lookup,
unchecked cast, or native-value cache. Structural decoding checks the envelope and Object identity;
it does not prove native semantics. The selected owner decodes the exact native contract and checks
ledger/protocol agreement before use. Public constructors must distinguish these guarantees.

| Shared value | Exact EVM payload |
| --- | --- |
| LedgerIdentity | Existing EvmChainInstance, including chain ID and expected genesis identity |
| ContractArtifact | Checked EvmScalarContractArtifact containing bounded canonical initcode |
| ContractLocator | EvmAddress |
| TransactionIdentity | EvmHash |
| ObservationPoint | Checked EvmBlockPoint containing block number and hash |
| ContractExecutionConfig | EvmContractExecutionConfig below |

Artifact/locator/point identities belong to their own exact contracts and ledger, not the version
of the transaction implementation that produced them. The maintained EVM artifact owner admits
only the supported scalar-contract artifact/ABI. Parsing arbitrary initcode does not certify its
semantics. Bind supported artifact identity and ABI selectors to the maintained first-party
artifact definition; compiler output remains a temporary artifact under the existing build policy.
Native construction rejects unsupported artifact/schema/ledger combinations before admission.

```rust
pub struct EvmContractExecutionConfig {
    binding: EvmTransactionBinding,
    deployment: Eip1559Options,
    configuration: Eip1559Options,
}

pub struct Eip1559Options {
    gas_limit: NonZeroU64,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
}
```

Keep the existing fee widths and require priority fee <= max fee. This scalar workflow transfers
zero native value; broader transaction recipes retain their separately supported behavior.
EvmTransactionBinding retains its full public authority epoch, route/chain instance, endpoint
reference, and sender. The observation route derives from it. An endpoint reference names an
explicitly supplied resource, not a hidden provider lookup or credential.

Configuration StableIds select supported implementation families/code versions. They are not the
exact descriptor references for both TransactionEffect request specializations. The compiler derives
the exact per-capability ABI references after checked selection. One native config Object contains
only the closed native binding/options structure; shared product values are not erased into it.
No signer/provider/authority handle, URL credential, or secret enters these values.

### 2.3 Authoritative workflow context

```rust
pub struct DeploymentRequest {
    artifact: ContractArtifact,
    execution: ContractExecutionConfig,
    requested: ConfigurationValue,
    increment: ConfigurationValue,
    retry_allowance: Option<u32>,
    restart_allowance: Option<u32>,
}

pub struct DeployedContract {
    request: DeploymentRequest,
    effective: ConfigurationValue,
    deployment: TransactionEvidence<ContractLocator>,
}

pub struct ConfigurationApplied;

pub struct ConfiguredContract {
    configured: DeployedContract,
    configuration: TransactionEvidence<ConfigurationApplied>,
}

pub struct ObservedConfiguration {
    configured: ConfiguredContract,
    observation: ContractValueEvidence,
}

pub struct ValidatedConfiguration {
    observed: ObservedConfiguration,
}

impl TransactionRequest for DeploymentRequest {
    type Applied = ContractLocator;
}
impl TransactionRequest for DeployedContract {
    type Applied = ConfigurationApplied;
}
```

ConfigurationApplied is an explicit unit MfmValue, not another execution step or receipt wrapper.
The original requested value and current effective value are intentionally different facts.
Deployment initializes effective from requested. Addition changes effective by checked addition of
the admitted increment, preserving siblings. Overflow is its exact typed domain failure before
configuration reservation or preparation.

DeployedContract and ConfiguredContract have checked constructors/decoders requiring their
corresponding transaction evidence to be Applied. ValidatedConfiguration requires observed value
equal to effective value. There is no second observed scalar beside ContractValueEvidence.value.
Public accessors expose checked semantic facts; native inspection is explicitly implementation-owned.
`DeployedContract::contract(&self) -> Result<&ContractLocator, InvocationDiagnostic>` matches
applied evidence and returns its locator. A rejected variant produces a reviewed invariant
diagnostic defensively, without panic. This is a checked semantic projection, not native validation
or a new domain failure; checked construction and decoding still reject that variant.

Configure consumes DeployedContract itself as its request. Its prepared input retains that request
once rather than adding a ConfigurationRequest wrapper or independently mutable effective/request
copies. Native encoding of 84 is a checked representation of that request, not another source of
semantic truth. Retaining complete typed requests increases some command payloads; measure this
against existing Object/report bounds rather than dropping context or introducing hidden lookup.

Report builds a useful checked ContractDeploymentReport from ValidatedConfiguration. It flattens
the retained request, effective value, deployment evidence, configuration evidence, and observation
evidence into one product report, preserving each once. Its requested_value(), effective_value(),
and observed_value() accessors derive from those fields. Construction/decoding checks the declared
semantic relationships. Report is not an identity State added merely to name a step.

The concrete State ABIs are fixed:

```rust
impl State for Deploy {
    type Input = PreparedTransaction<DeploymentRequest>;
    type Output = DeployedContract;
    type Failure = DeploymentFailure;
    // state_id identifies this concrete semantic implementation.
}
impl EffectState<TransactionEffect<DeploymentRequest>> for Deploy {
    // prepare returns the checked input command; interpret handles Applied/Rejected evidence.
}
impl State for Configure {
    type Input = PreparedTransaction<DeployedContract>;
    type Output = ConfiguredContract;
    type Failure = ConfigurationFailure;
    // state_id identifies this concrete semantic implementation.
}
impl EffectState<TransactionEffect<DeployedContract>> for Configure {
    // prepare returns the checked input command; interpret handles Applied/Rejected evidence.
}
```

These ABI sketches omit method bodies, not their responsibilities. prepare clones the concrete
checked prepared input. On Applied, interpretation moves its retained request into the checked
success context with the semantic evidence. On Rejected, it returns the State's exact domain
failure; Runtime retains the full original call/settlement. Neither method selects a network,
constructs native calldata, or decodes native receipts.

### 2.4 Evidence and original failures

```rust
pub enum TransactionResult<T> {
    Applied { output: T },
    Rejected { reason: Option<String> },
}

pub struct TransactionEvidence<T: MfmValue> {
    effect_id: EffectId,
    command_ref: ContentRef,
    implementation_ref: ContentRef,
    transaction: TransactionIdentity,
    observed_at: ObservationPoint,
    original: Object,
    result: TransactionResult<T>,
}

pub struct ReadContractValue {
    target: ContractLocator,
    at: ObservationPoint,
}

pub struct ContractValueEvidence {
    intent_ref: ContentRef,
    implementation_ref: ContentRef,
    observed_at: ObservationPoint,
    value: ConfigurationValue,
    original: Object,
}

pub struct ContractRead;
impl ReadCapabilityContract for ContractRead {
    type Intent = ReadContractValue;
    type Evidence = ContractValueEvidence;
    // Identity and semantic binding follow section 3.
}
```

Fields are private and owners construct checked evidence. TransactionResult is the necessary
applied/rejected discriminator inside evidence. It does not contain Pending, operational errors,
or State outcomes. The capability returns evidence; the State returns ProposedStateOutcome.

Rejected means an authenticated unsuccessful settlement under the supported protocol. A reason is
Some only when an actual reviewed explanation is available; None explicitly means unavailable.
Current EVM reverted receipts supply no revert reason and map to None. Do not invent one or add
tracing IO. Optional reason text is a projection; exact native codes/details remain in original.
Transport uncertainty cannot be normalized into Rejected.

Observe requests the deployed locator at the configuration evidence's exact observed_at point.
The native implementation checks target, request binding, block/ledger facts, and ABI decoding,
then exposes the checked scalar. Validate compares that semantic value with retained effective;
it does not repeat native integrity validation. This refactor adds no finality-policy enum or
configurable observation modes. The EVM guarantee remains the pinned non-reorging fixture policy.

The original Object is cloned immutably, never re-encoded as an allegedly identical native original.
Its value_ref identifies native evidence, not the semantic view. Effect/intent ref, exact selected
implementation, semantic evidence contract, and native original establish view provenance. No
mandatory semantic-view hash or second authoritative settlement is stored.

DeploymentFailure and ConfigurationFailure are exact State-declared semantic rejection contracts.
Their enclosing StateCall retains the request, command, native evidence, and rejection meaning;
do not duplicate the whole executed context inside each failure. Their intrinsic classification
preserves the existing Permanent classification for authenticated transaction reversion. Other
semantic failures, such as overflow and observation mismatch, belong to their owning Pure States.

## 3. Native implementation and codec interfaces

### 3.1 Inward capability interfaces

Semantic capability contracts retain typed commands/intents, evidence, identity, and semantic
binding. OperationalError moves to the selected native implementation. Capability traits do not
depend on State outcomes or Program's classifier.

```rust
pub trait EffectCapabilityContract: Send + Sync + 'static {
    type Command: MfmValue;
    type Evidence: MfmValue;
    fn contract_id() -> Result<StableId, CapabilityError>;
    fn bind_evidence(
        effect_id: &EffectId,
        command_ref: &ContentRef,
        command: &Self::Command,
        native_evidence_ref: &ContentRef,
        evidence: &Self::Evidence,
    ) -> Result<(), InvocationDiagnostic>;
}

pub trait ReadCapabilityContract: Send + Sync + 'static {
    type Intent: MfmValue;
    type Evidence: MfmValue;
    fn contract_id() -> Result<StableId, CapabilityError>;
    fn bind_evidence(
        intent_ref: &ContentRef,
        intent: &Self::Intent,
        native_evidence_ref: &ContentRef,
        evidence: &Self::Evidence,
    ) -> Result<(), InvocationDiagnostic>;
}
```

Semantic binding checks exact request identity and original-evidence provenance, including that the
carried original matches the authoritative native reference. It does not decode native receipts
again. Native implementation code owns protocol interpretation.

### 3.2 Two pure translation methods per implementation

```rust
pub trait EffectImplementation<C: EffectCapabilityContract>: Send + Sync + 'static {
    type Binding: MfmValue;
    type NativeCommand: MfmValue;
    type NativeEvidence: MfmValue;
    type OperationalError: MfmValue;
    fn implementation_id() -> Result<StableId, CapabilityError>;

    fn decode_command(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &Self::Binding,
        command_ref: &ContentRef,
        command: &C::Command,
    ) -> Result<(ContentRef, Self::NativeCommand), InvocationDiagnostic>;

    fn project_evidence(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &Self::Binding,
        effect_id: &EffectId,
        command_ref: &ContentRef,
        command: &C::Command,
        native_command: &Self::NativeCommand,
        native_evidence: &Self::NativeEvidence,
        original: &Object,
    ) -> Result<C::Evidence, InvocationDiagnostic>;
}

pub trait ReadImplementation<C: ReadCapabilityContract>: Send + Sync + 'static {
    type Binding: MfmValue;
    type NativeIntent: MfmValue;
    type NativeEvidence: MfmValue;
    type OperationalError: MfmValue;
    fn implementation_id() -> Result<StableId, CapabilityError>;

    fn encode_intent(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &Self::Binding,
        intent: &C::Intent,
    ) -> Result<Self::NativeIntent, InvocationDiagnostic>;

    fn project_evidence(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &Self::Binding,
        intent_ref: &ContentRef,
        intent: &C::Intent,
        native_intent_ref: &ContentRef,
        native_intent: &Self::NativeIntent,
        native_evidence: &Self::NativeEvidence,
        original: &Object,
    ) -> Result<C::Evidence, InvocationDiagnostic>;
}
```

These explicit inputs come from the already-associated descriptor, retained request, and exact
native evidence. No implementation queries Runtime or configuration to discover them. Do not add
another generic execution-context wrapper solely to abbreviate these signatures.

Effect decode_command selects the private native descriptor, admits its exact schema, decodes it,
and checks implementation/binding/request correspondence. Its returned ContentRef is the native
command identity, distinct from the semantic command identity. Identity implementations return the
supplied command_ref without re-encoding the command; prepared transaction implementations return
their inline descriptor's reference. For Configure it checks the retained
effective84 against native84. Configure itself never decodes calldata or repeats this check.

Prepared constructors canonicalize native descriptors once and use the same owning validation
rules as checked extraction. Applicable checks also run on decoded values. Private fields and
structural Object decoding alone cannot establish native correspondence. Preserve current
State::prepare(input) versus retained-command consistency without adding a generic Runtime callback
comparing arbitrary State input with native commands. Local checks do not claim to authenticate an
arbitrary coherent substitution of all run history.

project_evidence combines native binding and semantic projection in one checked pure function.
Framework encoding creates the authoritative native Object once and supplies its decoded native
value alongside it. Projection may clone that Object but may not substitute or re-encode its source.
Read encode_intent translates the retained semantic request; the framework canonicalizes that native
intent once per required derivation and passes both semantic and native intent references explicitly
to the adapter. Cold derivation uses the same selected code. The references are never interchangeable.

Native supporting protocols use these same traits even when semantic/native types are identical.
Identity translation is the base case, not another native-only engine or callback registry.

### 3.3 Adapters, timing, and error routes

Preserve the existing async callback/lifetime representation in Runtime assembly. The Effect
callback receives EffectId, native command reference, and typed I::NativeCommand, and returns:

```rust
Result<EffectAdapterOutcome<I::NativeEvidence>, AdapterError<I::OperationalError>>
```

A Read callback receives the semantic intent reference, native intent reference, and typed
I::NativeIntent, returning typed native evidence or AdapterError<I::OperationalError>. Explicit
binding resources are captured at assembly. No new executor or waiting trait is needed.

| Moment | Required behavior |
| --- | --- |
| Expansion/assembly | Select implementation, exact ABI/binding, typed injection, codecs and live callback |
| Before command acknowledgement | Selected checked native extraction rejects local mismatches; preserve existing exact command admission |
| Before provider call | Native adapter checks actual authority, epoch, sender, signing purpose, and retained wire |
| Native evidence returned | Canonically encode once; first encoding failure preserves cause/context without retry or opaque side custody |
| Before Effect settlement append | Decode native evidence, run checked project_evidence and semantic binding; discard temporary semantic view |
| AwaitingInterpretation | Run the same pure projection/binding, then State interpretation; native settlement stays authoritative |
| Read execution | Translate, observe, encode/bind/project, interpret, then append the existing Read conclusion |
| Cold reconstruction | Repeat exact checked decoding/translation under retained implementation; no resolution, source config, or native cache |

Always checking the same projection before settlement append removes the previous optional
"projection needed for admission" path. A subsequent projection/interpretation failure preserves
the acknowledged native settlement and known head; it does not authorize resubmission. A failed
Read projection before conclusion makes no claim of acknowledged evidence.

| Situation | Owner and result |
| --- | --- |
| Authenticated unsuccessful settlement | Capability projects Rejected evidence; State returns its exact semantic domain failure |
| Provider, transport, signer, authority failure | Native owner adapts once into exact I::OperationalError; Runtime durably retains and classifies that original |
| Local schema, codec, binding or invariant failure | Existing invocation-failure route with concrete originating causes and available diagnostic facts |

There is no native-error -> generic-capability-error -> State-error conversion chain. Error codecs
serialize/decode exact native originals and perform reviewed source-preserving adaptation. Neither
classification nor public rendering replaces the cause. Preserve source ancestry, execution context,
reviewed fields, parser category/location/reason, and applicable size facts. Do not reduce errors to
InvalidContract, an enum label, or a formatted string. Extend existing construction error contracts
with source-preserving variants as needed; no new generic error bag or classifier is introduced.

I::OperationalError: ClassifyError is enforced in Program/Runtime association. ClassifyError stays
in Program; moving it inward solely to satisfy a bound would invert current dependencies. Operation
defaults cannot override intrinsic classification. Changing an original error's classification
semantics changes its exact error-contract identity, including State domain errors. A local mismatch has no affected provider call or
append and cannot become a durable integrity block without authenticated external evidence.

## 4. Supporting States and typed injection

### 4.1 Required EVM sequence

```text
DeploymentRequest
 -> ReserveEvmNonce<deployment recipe>
 -> PrepareEvmTransaction<deployment recipe>
 -> Deploy
 -> DeployedContract
 -> CheckedAddConfigurationValue
 -> DeployedContract(effective84)
 -> ReserveEvmNonce<configuration recipe>
 -> PrepareEvmTransaction<configuration recipe>
 -> Configure
 -> ConfiguredContract
 -> Observe
 -> ObservedConfiguration
 -> Validate
 -> ValidatedConfiguration
 -> Report
```

The two recipes and supporting State specializations belong to the EVM implementation. Consumers
neither supply recipes/slots nor name intermediate contexts. Reserve's deterministic prepare
constructs/checks the nonce-free command from actual predecessor input before reservation command
acknowledgement. Its adapter reserves authority. Prepare signs/retains exact wire through the
explicit authority and its interpretation constructs `PreparedTransaction<R>`.

Native intermediate context is a concrete `ReservedRequest<R> { request: R, reserved:
ReservedEvmTransaction }`, retaining the exact request through both supporting States. The next
output is common `PreparedTransaction<R>`, containing the public PreparedEvmTransaction descriptor.
Signed bytes remain exclusively in EvmTransactionAuthority. Store gains no arbitrary value lookup.

No additional ConstructNativeRequest or codec State is required. Observe has no injected support
for the current anchored scalar read; its pure native translation/projection are callbacks in the
existing Read path. Future genuinely independent work uses ordinary explicit supporting States.

Delete ExecuteEvmTransaction as the designated product step and ProjectEvmTransactionOutcome as a
mandatory suffix. Native action checks move to capability preparation/projection; semantic rejection
and successful output construction move to Deploy/Configure. The new failure origin is the designated
Effect interpretation, with the original settlement and input preserved. This is an explicit new
ABI/boundary, not a reinterpretation of old histories. No useful suffix work remains in this case.

### 4.2 Endpoint contracts and recursive expansion

```rust
pub trait EffectSelection<C>: EffectState<C>
where C: EffectCapabilityContract {
    type ExpandedInput: MfmValue;
    type ExpandedOutput: MfmValue;
}

pub trait InjectEffect<S, C>: EffectImplementation<C>
where C: EffectCapabilityContract, S: EffectSelection<C> {
    type Prefix: AuthoringSource<Input = S::ExpandedInput, Output = S::Input>;
    type Suffix: AuthoringSource<Input = S::Output, Output = S::ExpandedOutput>;
    fn surround(binding: &Self::Binding)
        -> Result<(Self::Prefix, Self::Suffix), ProgramError>;
}
```

ReadSelection and InjectRead have the same endpoint relationships with ReadState/ReadImplementation.
Pure selections use State's raw endpoints. The domain fixes expanded endpoints independently of the
native implementation; configuration cannot change a Rust associated type.

| Selection | Expanded input | Raw State input | Expanded/State output |
| --- | --- | --- | --- |
| Deploy | DeploymentRequest | `PreparedTransaction<DeploymentRequest>` | DeployedContract |
| Configure | DeployedContract | `PreparedTransaction<DeployedContract>` | ConfiguredContract |
| Observe | ConfiguredContract | ConfiguredContract | ObservedConfiguration |
| Add | DeployedContract | DeployedContract | DeployedContract |
| Validate | ObservedConfiguration | ObservedConfiguration | ValidatedConfiguration |
| Report | ValidatedConfiguration | ValidatedConfiguration | ContractDeploymentReport |

Use typed identity suffixes for Deploy/Configure. Supporting reservation/preparation protocols
have identity prefixes/suffixes unless a concrete further requirement exists. Empty identity is
valid only for equal endpoints; a suffix runs on success, not as a finally handler.

An implementation-owned `ResolvedEffect<S, C, I>` holds I::Binding and type markers. Its endpoints
are S's expanded endpoints, like Effect<S, C>. ResolvedRead is analogous. Ordinary selection asks
the profile for I; a resolved supporting selection already supplies I. Both call the same typed
injection routine. These supporting leaves are not an everyday second DSL or native-only emitter.

Expansion walks the prefix recursively, emits the designated executable exactly once, and walks
the successful suffix recursively. Designated emission is a private action; it does not reinject
itself. Each emitted State retains its own ABI, policy scope, original failure, persistence and
recovery boundary. Injection never makes the whole sequence atomic.

Supporting definitions form finite typed structures. Apply depth and total-State limits across
Operation and injected nesting; reject excess before publishing Program/assembly. Runtime limits
do not make infinitely recursive Rust types valid. Native contracts/codecs are associated
requirements, not additional persisted States merely because they were selected.

## 5. One authoring representation and maintained defaults

### 5.1 Tuples and Operations

```rust
pub trait AuthoringSource: sealed::Sealed {
    type Input: MfmValue;
    type Output: MfmValue;
}

pub struct Operation<Body, Defaults = Inherit> {
    body: Body,
    defaults: PhantomData<Defaults>,
}

pub type ContractDeploymentLifecycle = Operation<
    (
        Effect<Deploy, TransactionEffect<DeploymentRequest>>,
        Effect<Configure, TransactionEffect<DeployedContract>>,
        Read<Observe, ContractRead>,
        Pure<Validate>,
        Pure<Report>,
    ),
    LifecycleDefaults,
>;

pub type ConfigureAndObserve = Operation<
    (
        Effect<Configure, TransactionEffect<DeployedContract>>,
        Read<Observe, ContractRead>,
    ),
    Inherit,
>;
```

Operation::new(body) is infallible on Operation<Body, Inherit>, preserving inference. Maintained
Operation::default constructs the same structure with its Defaults marker. Selection defaults
construct markers, not executable States; they require no S: Default. Neither construction performs
IO, binding resolution, input validation, or State execution. A maintained definition need not be a
literal Rust static; it is reusable with different checked inputs and resulting Programs.

Tuple adjacency requires exact equality of expanded endpoints. Operations are ordinary tuple
elements and retain scopes. No aggregate Failure associated type, construction closure, fluent
parallel DSL, getter collection, selection constants, or caller-defined aliases are required.

Framework-controlled sources comprise tuples, Operations, Pure/Read/Effect selections, resolved
supporting selections, typed identity, checkpoints, Choice, and Repeat. New-State authors enter
through a State selection, not an arbitrary mutable emitter. Exact supported tuple arities and
nested tuples must be covered in consuming tests, without inventing a dynamic heterogeneous list.

AuthoringSource's endpoints are not all compiler bounds: selection resolution and defaults also
depend on the current borrowed compile configuration. Section 6 defines that traversal. Keep its
mode traits internal instead of exposing a generic visitor DSL.

### 5.2 Configuration and policy defaults

Checked input is the sole per-run configuration source. Operation definitions own structure and
interpretation of supported policy values. There is no independent Operation-instance config or
binding payload, with_selection(), with_policy(), or speculative Scoped wrapper.

```rust
pub trait OperationDefaults {
    type Handler: Handler;
    type Targets: CheckpointTargets;
}

pub trait ResolveDefaults<Config>: OperationDefaults {
    fn resolve(config: &Config)
        -> Result<PolicyValues<Self::Handler>, ProgramError>;
}

pub struct PolicyValues<H: Handler> {
    pub handler: Option<H::Params>,
    pub retries: Option<u32>,
    pub restarts: Option<u32>,
}
```

CheckpointTargets is the sealed typed marker-tuple protocol used for permitted recovery targets.
Some(params) installs H, params, and Defaults::Targets as one unit. None inherits that entire unit.
Retries/restarts inherit independently; Some(0) explicitly means zero. Inherit uses Stop and empty
targets, resolving all fields to None. Cold association visits handler/target types without resolving
values. One handler type per maintained defaults definition is sufficient; no configurable handler
alternatives or string-based handler lookup are added.

Framework fallback and the baseline lifecycle default are Stop with zero allowances. LifecycleDefaults
identifies Stop/empty targets and resolves the root's supported allowance options with zero fallback.
An allowance is permission, not a command to retry; Stop still requests Stop. A maintained defaults
definition that selects StandardRecovery can interpret those same supported inputs without altering
intrinsic classification or adding a new caller modifier API.

Inheritance is framework -> enclosing Operation -> nearer maintained child/injection scope.
Supporting States inherit applicable defaults. Leaving a child restores its parent's scope.
Private existing RecoveryDefaults/HandlerBinding lowering implements this merge; remove superseded
public untyped Occurrence modifiers rather than keeping two policy declaration surfaces.
Local allowances never enlarge the Program-wide admitted recovery budget or bypass Effect barriers.

A concrete scoped example is a maintained observation child containing
`(Checkpoint<AfterConfigure>, Read<Observe, ContractRead>)`. Its defaults may select StandardRecovery,
Some(0) retries, Some(1) restart, and Targets=(AfterConfigure,). The marker has ConfiguredContract
endpoints and precedes the child's first executable. The handler, parameters, and target replace as
one unit; Validate/Report outside that child retain parent defaults. This is a policy example,
not a change to the baseline lifecycle definition above.

Checkpoint markers emit no State/frame. Resolve a newly installed target only in its owning scope.
Reject missing, duplicate, foreign, forward, terminal-without-following-State, and context-mismatched
targets. An inherited already-bound parent target is never rebound to a child marker of the same
type. Repeated occurrences receive distinct scopes. Relocate to final expanded positions and retain
Runtime's irreversible Effect barrier checks.

### 5.3 Choice, repetition, and semantic planning checks

Choice<A, B> requires equal endpoints and expands the selected construction-time branch. It is not
a failure sum or Runtime branch. Choice also supplies the closed alternative structure for native
implementation families; do not introduce another alternative algebra or registry.

```rust
pub trait ItemsFrom<Config> {
    type Item;
    fn items(config: &Config) -> &[Self::Item];
}

pub struct Repeat<Body, Items> {
    body: Body,
    items: PhantomData<Items>,
}
```

Body's output must equal its input. Compile each body under the corresponding borrowed Item as
its configuration; derive count/order from the actual checked slice. Empty repeat is typed identity.
A bare empty tuple cannot establish an arbitrary input contract; provide explicit typed identity.
Each occurrence gets a fresh policy/checkpoint scope and contributes to cumulative expansion limits.

Portfolio's production selector borrows the root's collections. Its repeated body is
EnterPortfolioCollection -> CollectEvmBalances -> ResumePortfolioCollection. Resolve each occurrence's
route from that actual item. Delete the separately stored checked_collections vector and root-only
validator once this coupling is proved. The continuation preserves the immutable collection plan;
execution changes cursor/results, not collection count/order/routes. A composition that changes
planning fields cannot claim compatibility merely by sharing a broad context type.

Remove arbitrary with_input_check/root predicates, not their guarantees. Checked constructors and
decoders own local invariants; capability setup owns binding/action agreement; planning derives from
the committed input; semantic prerequisites belong to their checked contracts/owning States.
Compilation cannot simulate preceding States to validate a future 84. Native preparation consumes
that future value during execution. Retain Program validation for decoded/untrusted documents,
exact schemas, content identity, and capacity even where Rust proves authored connections.

## 6. Resolution, compilation, and executable association

### 6.1 Profiles and one structural walk

```rust
pub trait CapabilityFamily<C, Role> {
    type Choices;
}

pub trait Resolve<Config, C, Role>: CapabilityFamily<C, Role> {
    fn resolve(config: &Config) -> Result<Self::Choices, ProgramError>;
}
```

A production profile declares supported typed implementation choices per capability family and
binding role. A transaction family can implement these generically over supported R, sharing the
configuration match; adding Configure does not require another handwritten native registry.
Each selected leaf supplies a concrete I and checked I::Binding. All offered alternatives must
support the selected State's typed injection. Use a narrower admitted family/role if necessary;
never silently fall back to a network incapable of the advertised operation.

The default role keeps ordinary selections two-argument. Multiple accounts/networks use explicit
typed roles. Shared roles must agree across a workflow. At entry, Config is the actual checked root
input. Nested tuples/Operations inherit it; Repeat borrows an Item. Changing compile configuration
for a repeated body does not change the root input whose identity is committed.

The private traversal is one sealed structural walk with two modes:

| Source | Compile mode | Inventory mode |
| --- | --- | --- |
| Tuple | Walk children with unchanged borrowed config | Walk child types |
| Operation | Enter scope, resolve typed defaults, walk body | Visit declared handler/target and body types |
| Choice/family | Walk selected supported branch | Walk all supported branch types |
| Repeat | Borrow each Item and walk Body under it | Walk Body type once |
| Unresolved selection | Resolve profile choice/binding then use typed injection | Visit family alternatives and their injection types |
| Resolved supporting selection | Use supplied I/binding through the same injection | Visit I/prefix/designated/suffix types without values |

Compile leaf bounds require Resolve<Config, C, Role>; inventory requires only structural family and
injection support. Inventory never calls resolve/surround/default resolution, fabricates C0, or loads
source configuration. Runtime does not perform this authoring traversal during progression.

### 6.2 Exact requirement emission and atomic construction

Program owns the receiver contract; Runtime implements it through its existing exact ABI tables:

```rust
trait ExecutableRequirements {
    type Error: From<ProgramError>;
    fn value<V: MfmValue>(&mut self) -> Result<(), Self::Error>;
    fn pure<S: PureState>(&mut self) -> Result<(), Self::Error>;
    fn read<S, C, I>(&mut self) -> Result<(), Self::Error>
    where C: ReadCapabilityContract, I: ReadImplementation<C>,
          I::OperationalError: ClassifyError, S: ReadState<C>;
    fn effect<S, C, I>(&mut self) -> Result<(), Self::Error>
    where C: EffectCapabilityContract, I: EffectImplementation<C>,
          I::OperationalError: ClassifyError, S: EffectState<C>;
    fn handler<H: Handler>(&mut self) -> Result<(), Self::Error>;
}
```

These generic methods are statically dispatched, not a dyn visitor. Emission retains concrete types
until both descriptor and executable requirements are supplied: raw/expanded values, original
failures, semantic/native requests/evidence, operational errors, exact translation functions,
handler parameters, and support. There is no root-map receiver method. Same exact registration is
idempotent; conflicting ABIs fail. Include I and binding in association, not just shared C or State ID.

```rust
fn compile<S: AuthoringSource>(
    &mut self,
    entry_point: EntryPointId,
    source: &S,
    input: &S::Input,
    limits: ProgramLimits,
) -> Result<Program<S::Input, S::Output>, CompileError>;
```

The builder is profile-typed (`Runtime::builder::<EvmContractCapabilities>(store)?`); built Runtime
is not. The shown compile signature has additional private walk/profile bounds described above.
Stage the complete Program and association delta privately. Check input/config agreement, roles,
contracts, policies, limits, and exact live bindings before publication. Any failure leaves the
builder unchanged. Constructor/codec/assembly errors retain their concrete causes.

Program-only compilation uses this same compiler/profile with a no-op receiver, not a second
lowering implementation. Program<I,O> wraps one immutable content-addressed representation; checked
reconstruction verifies exact endpoints and initial commitment, not merely PhantomData.

The existing adapter table holds native callbacks under an exact key derived from mode, native
implementation StableId, native request/evidence/operational-error/binding contract references, and
binding value reference. The full <S,C,I> executable association selects its monomorphized pure
translators and matches that native entry. This key is not a new adapter identity and never replaces
the full semantic/native Program ABI. No wildcard, fallback, generic callback factory, or second
registry is introduced.

This allows one native PreparedEvmTransaction callback to serve deployment and configuration's
distinct `TransactionEffect<R>` ABIs. register_evm_transaction_adapters remains independent of R and
binds its fixed reserve/prepare/submit IO protocols; the compiler discovers selected executable
requirements. Callers maintain neither per-request registrations nor lists of executable States.
Conflicting callbacks or bindings under one exact native key are rejected.

Each native entry retains the checked canonical public binding Object. Installation verifies its
schema/value reference; association decodes I::Binding and captures it in immutable translator
closures alongside the selected code. This is assembly state, not a typed continuation cache.
The Program retains its binding reference; a reference alone does not reconstruct the binding value.
Cold callers supply matching explicit public bindings/resources without needing the source config.

### 6.3 Durable identities and cold association

Extend existing Program Execution descriptors with separate native implementation facts:

| Descriptor | Retained facts |
| --- | --- |
| State ABI | State implementation, input/output/failure contracts |
| Semantic capability | Exact capability and command/intent/evidence contracts |
| Native implementation | Exact implementation, native command/intent/evidence, operational-error and public-binding contracts |
| Occurrence | Public binding, handler/parameters, allowances, permitted targets |

Implementation identity commits extraction, binding, and projection semantics. Changing those
semantics changes the versioned identity even with unchanged schemas. No separate projection or
injection ID is needed: the actual expanded sequence already commits injection.

```text
EffectCall.command = exact C::Command
  (PreparedTransaction<R> with native descriptor for designated transaction selections)
Settlement.evidence = authoritative native original
EffectId = H(RunId, ProgramRef, ExecutionPosition, semantic_command_ref)
```

Keep these existing operation records. Do not persist a second native command beside the inline
descriptor or a duplicate semantic settlement/view hash. Native command refs cross custody explicitly
and never replace semantic refs in EffectId. Read intent/evidence provenance makes the corresponding
native/semantic distinction without adding an Effect-style settlement transition.

A fresh builder associates source/profile types, including all supported injected alternatives and
handlers, using `builder.associate::<ContractDeploymentLifecycle>()?`. Then bind explicit live
resources and build. Read/resume checks retained exact descriptors against those implementations.
No original config, fake input, network re-selection, native cache, or missing-ABI fallback occurs.
Unavailable exact implementations fail explicitly.

A later Program can execute on an existing immutable Runtime only if its exact requirements are
already associated. Otherwise build a new immutable assembly sharing Store and explicitly supplied
resources. Do not mutate the existing Runtime or secretly install code at admission.

## 7. Runtime execution and recovery

```rust
async fn execute<I: MfmValue, O: MfmValue>(
    &self,
    run_id: RunId,
    program: Program<I, O>,
    input: I,
) -> Result<ExecutionResult<O>, InvocationFailure>;
```

Execute accepts the resulting Program, not an Operation. It checks exact input/association and uses
the existing engine until terminal success/failure, RecoveryStopped, invocation failure, or caller
cancellation. ExecutionResult has checked terminal success or terminal original failure; accessors
success()/failure() borrow the applicable value/report and retain exact RunId. RecoveryStopped is
InvocationFailure with unresolved command authority, never terminal domain failure.

| Recorded phase | Progression |
| --- | --- |
| Runnable | Enter the next expanded State through its existing execution mode |
| EffectPending | Reconcile the exact retained command through the selected adapter |
| AwaitingInterpretation | Project retained native settlement and interpret; do not resubmit |
| AwaitingRecovery | Classify recorded original, invoke handler, authorize and record decision |
| Terminal | Return checked result |

State conclusions, original failures, commands, settlements, and recovery decisions retain their
separate durable boundaries. RecoveryStopped follows a recorded decision to stop pending-Effect
recovery; it does not imply that the external transaction failed. Later explicit resume may reconcile.

Pending must not busy-spin. Native async callbacks suspend for readiness/reconciliation backoff
within the existing IO boundary. No new Runtime timer, waiting trait, detached driver, deadlines,
wall-clock limits, invocation progression budget, configurable polling, or incomplete-result variant
is added. Preserve direct progression/read/resume for deliberate manual use. Adapter waiting cannot
silently consume operational failures in an invisible retry/recovery loop.

Preserve InvocationFailure::Execution's RunId, original RuntimeError and last_observed, plus the
checked observation on RecoveryStopped. RuntimeError::Projection retains a known acknowledged summary
separately from older/failed observation. RecordingFailure retains the exact candidate and definite
or indeterminate disposition. No implicit append retry, fabricated cancellation record, or claim
that a failed Store audited its own failure is allowed. Cancellation promises no response; resume
uses acknowledged history without changing command authority. Even before a view exists, failures
identify the caller-supplied RunId.

Discovery remains ordinary Program execution. Checked output/head/RunId linkage can construct a new
input and new immutable Program under a new RunId. Preserve configuration-deletion and dependent-start
semantics. Discovery neither mutates an admitted Program nor bypasses unresolved Effect authority,
replaces a transaction, or becomes a new inter-Program scheduler.

## 8. Complete caller examples

Production exports State types, requests, capabilities, and maintained Operation definitions from
`mfm_transactions::contract_lifecycle` and the shared transaction module. The EVM domain exports
checked `EvmContractWorkflowConfig`; live composition supplies EvmContractCapabilities and explicit
adapter binders. No State getters or selection constants are needed.

The config type implements checked Deserialize. Its fields are the native artifact, native execution
config, shared requested/increment scalars, and optional retry/restart allowances described above.
`initial_input()` constructs DeploymentRequest, encoding native envelopes once and assigning the
supported implementation selectors. `transaction_binding()` borrows that config's exact public
EVM binding. The shared DeploymentRequest cannot expose an EVM-typed binding accessor.

Examples assume the caller supplies config_text through its own IO boundary, explicit Store/live
resources, entry_point, limits, and one RunId. They use the existing workspace TOML parser in the
consuming application/example, not a new domain parsing dependency; another Serde format can consume
the same checked config type. TOML parsing is configuration loading, not an author-supplied codec.
All named production components here are target API, not presently compiled symbols.

### 8.1 One existing Pure State

```rust
let input = CheckedAddition::new("42", "42")?;
let state = Pure::<CheckedAdd>::default();
let mut builder = Runtime::builder::<NoCapabilities>(store)?;
let program = builder.compile(entry_point, &state, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input).await?;
assert_eq!(result.success().expect("terminal success").to_string(), "84");
```

Production supplies checked scalar construction/addition and its exact overflow failure. No binding,
root failure conversion, or registration list exists for this case.

### 8.2 One existing injected Effect State

```rust
let config: EvmContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let state = Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default();
let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
register_evm_transaction_adapters(
    &mut builder, config.transaction_binding().clone(),
    signer, authority, transaction_provider,
)?;
let program = builder.compile(entry_point, &state, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input).await?;
let deployed = result.success().expect("terminal success");
assert_eq!(deployed.effective_value().to_string(), "42");
let locator = deployed.contract()?;
```

DeploymentRequest is the public expanded input. Compile associates native support and Deploy itself;
caller input contains no reserved/prepared facts. The locator accessor exposes the applied semantic
ContractLocator, not an EVM address. Native inspection remains with a checked native accessor when
specifically required. Adapter binding validates epoch, sender, signing purpose and exact resources.

### 8.3 One maintained Operation

```rust
let config: EvmContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let binding = config.transaction_binding().clone();
let operation = ContractDeploymentLifecycle::default();
let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
register_evm_transaction_adapters(
    &mut builder, binding.clone(), signer, authority, transaction_provider,
)?;
register_evm_anchored_contract_calls(&mut builder, binding.route.clone(), read_provider)?;
let program = builder.compile(entry_point, &operation, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input).await?;
let report = result.success().expect("terminal success");
assert_eq!(report.requested_value().to_string(), "42");
assert_eq!(report.effective_value().to_string(), "42");
assert_eq!(report.observed_value().to_string(), "42");
```

Supported config chooses native values/implementations; the maintained Rust definition owns order
and defaults. Merely admitting increment42 does not execute addition.

### 8.4 A mixed composition of existing Operations and States

```rust
let config: EvmContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let binding = config.transaction_binding().clone();
let states = (
    Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    ConfigureAndObserve::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
);
let operation = Operation::new(states);
let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
register_evm_transaction_adapters(
    &mut builder, binding.clone(), signer, authority, transaction_provider,
)?;
register_evm_anchored_contract_calls(&mut builder, binding.route.clone(), read_provider)?;
let program = builder.compile(entry_point, &operation, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input).await?;
let report = result.success().expect("terminal success");
assert_eq!(report.requested_value().to_string(), "42");
assert_eq!(report.effective_value().to_string(), "84");
assert_eq!(report.observed_value().to_string(), "84");
```

The all-State form replaces ConfigureAndObserve with
`Effect::<Configure, TransactionEffect<DeployedContract>>::default()` and
`Read::<Observe, ContractRead>::default()` in the same tuple. The tuple itself is also a compilable
source when no Operation scope is needed. No lambda, context, alias, mapper, codec, or registry is
introduced by the caller. Configuration calldata is constructed from effective84 during native
preparation, not pre-encoded in config or edited after acknowledgement.

### 8.5 New semantics composed with existing components

Only the new State and its original failure are authored:

```rust
#[derive(Debug, serde::Serialize, serde::Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
pub struct ZeroConfiguration {}

impl ClassifyError for ZeroConfiguration {
    fn classify(&self) -> Classification { Classification::Permanent }
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
    fn evaluate(input: DeployedContract) -> Result<
        ProposedStateOutcome<DeployedContract, ZeroConfiguration>, InvocationDiagnostic,
    > {
        if input.effective_value().is_zero() {
            Ok(ProposedStateOutcome::Failure { failure: ZeroConfiguration {} })
        } else {
            Ok(ProposedStateOutcome::Success { output: input })
        }
    }
}

let config: EvmContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let binding = config.transaction_binding().clone();
let operation = Operation::new((
    Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    Pure::<RequireNonZeroConfiguration>::default(),
    ConfigureAndObserve::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
));
let mut builder = Runtime::builder::<EvmContractCapabilities>(store)?;
register_evm_transaction_adapters(
    &mut builder, binding.clone(), signer, authority, transaction_provider,
)?;
register_evm_anchored_contract_calls(&mut builder, binding.route.clone(), read_provider)?;
let program = builder.compile(entry_point, &operation, &input, limits)?;
let runtime = builder.build()?;
let result = runtime.execute(run_id, program, input).await?;
assert_eq!(result.success().expect("terminal success").observed_value().to_string(), "84");
```

ProgramError's identity-construction conversion must retain the source. The empty failure does not
discard its rejected input: Runtime's complete Failure retains the originating call and prior facts.
A companion zero-input/zero-increment case cold-decodes exactly ZeroConfiguration and proves no
configuration command was prepared. No enclosing failure conversion or registration entry is added.

## 9. Original-failure reporting and migration

Delete mandatory aggregate/root failures, not originals. Runtime commits the exact declared domain
or native operational original before classification, handler invocation, and recovery decision.
A failed handler, codec, report, or Store operation cannot replace that acknowledged original.
Wrong-contract checked access fails explicitly; it is not unchecked JSON or successful absence.

The current canonical Domain FailureReport serializes original/root but omits StateCall retained
in memory. Removing root alone would lose exported execution context. Replace its versioned wire:

```rust
struct Report<'a> {
    domain: &'static str,
    run_id: &'a RunId,
    program_ref: &'a ContentRef,
    state_implementation_ref: &'a ContentRef,
    execution: &'a Execution,
    failure: &'a Failure,
    reason: StopReason,
    usage: RecoveryUsage,
}
```

Runtime copies provenance from the retained, associated Program/current run. Execution is the
extended descriptor from section 6.3. Failure already owns position/input, Effect command/settlement
or Read intent/evidence, and exact original. Keep position there once. Objects own their exact
contracts/value refs. No whole Program/handler copy, new incident tree, or projection ID is needed.

This bounded diagnostic artifact supports exact decoding/projection with installed implementations
and matching explicitly associated public bindings, without source config. Installed code alone
cannot supply a missing binding value. Diagnostic projection uses pure translators, not provider IO.
It does not prove its copied descriptor belongs to ProgramRef or grant execution
or recovery authority. An API using an untrusted report authoritatively must check it against the
exact Program declaration at its position. Stopped pending authority remains RecoveryStopped.

Portfolio's existing mapped collection ordinal/code are a checked domain projection of the retained
EvmBalanceFailure. App decodes the exact original and calls the domain projection; domains do not
depend on Runtime FailureReport. Preserve native Portfolio originals directly. An ordinal conversion
failure becomes a projection error, not a fabricated ConsolidationFailed incident. Product projection
never replaces the original or becomes a prerequisite for ordinary composition.

Version Program, changed Runtime operation/recovery payloads, and reports together. Remove root
contract fields, root-map chains, map-only callbacks, mapped-root Stop payloads and root() accessors.
Retain Never as the exact uninhabited State failure. Preserve source facts, not duplicated-root size
thresholds or claims of unchanged artifact identity. Complete-report overflow retains the known
acknowledged original/recovery head and authority; use small-bound tests, no serializer retries.

## 10. Crate placement and dependency perimeter

Use one new inward domain crate, `crates/domains/transactions` (`mfm-transactions`), for shared
transaction contracts, purpose-specific semantic values, and its `contract_lifecycle` module.
This avoids copying those contracts into each native domain or placing product semantics in Runtime.
EVM depends inward on it; it never depends back on EVM. Values owns reusable Unsigned256 mechanics.

| Location | Target responsibility |
| --- | --- |
| kernel/capabilities | Generic semantic/native Read/Effect interfaces; no State outcome or classifier dependency |
| kernel/program | Typed source/injection/defaults, exact descriptors, compiler and requirement receiver |
| domains/transactions | `TransactionEffect<R>`, `PreparedTransaction<R>`, evidence, shared values and concrete lifecycle States/contracts |
| domains/evm | Native config/artifact contracts, request recipes, supporting States, native translation/projection and native operational originals |
| live/evm and downstream composition | Explicit IO binding and supported EvmContractCapabilities profile; no handwritten executable inventory |
| app | Supported wire/config use cases, checked product projections, inspection consuming structural inventory |
| kernel/runtime | Existing association tables/engine, continuation, recovery and checked results |
| journal/store/backends | Existing owned physical/wire contracts; separate native authority implementations remain separate ports |

EvmContractWorkflowConfig has checked Deserialize and the concrete native fields described in section
8. It lives with native deterministic domain contracts and needs no TOML dependency. A consuming
application owns format parsing/IO. Native wire consensus/signing qualification remains with its
current appropriate live/platform owner; moving shared semantics does not move provider IO inward.

Generic Store semantics do not acquire EVM knowledge because a backend separately implements
EvmTransactionAuthority. Reusable signing remains a platform primitive. No cryptographic or keystore
Send/Sync redesign, transaction replacement, mainnet finality redesign, new third-party dependency,
or shipping transaction enablement is authorized by this refactor.

Journal's opaque envelope and Store's physical schema need not change merely because Runtime
payloads change. Preserve PostgreSQL snapshot/locked-append/ambiguous-COMMIT semantics. Reject
superseded schemas under the clean cutover policy; do not rewrite acknowledged histories, retain a
migration reader, or claim old Programs run with unavailable ABIs.

## 11. Complete replacement and deletion scope

| Existing or superseded machinery | Replacement / retained owner |
| --- | --- |
| Operation-only expand_program and public mutable OperationExpansion | Neutral typed source compiler; useful draft/relocation logic stays private |
| Mutable before/after injection | Typed prefix/designated/suffix, including recursively resolved native support |
| Monomorphic TransactionEffect / unspecified SubmitPreparedTransaction | `TransactionEffect<R>` and `PreparedTransaction<R>` |
| State-facing native operational errors | Exact I::OperationalError in the one association/engine path |
| ExecuteEvmTransaction as product step, ProjectEvmTransactionOutcome, wrapper EvmTransaction | Concrete Deploy/Configure with retained reservation/preparation and checked semantic interpretation |
| Fixture-owned contexts, slots/aliases, ABI decoding, report reconstruction | Maintained shared contracts and capability-owned native code; internal recipes/slot mechanics may remain useful |
| Root Failure/ExpandedFailure/FailureMap, root maps and map-only machinery | Exact originals and complete versioned Failure reports; domain/App checked projections where required |
| register_fixture_states and native State-registration helper | Compiler-emitted exact requirements, including support/codecs/handlers |
| Application handwritten compiled State registration inventory | Inspection fed from the same structural source/type inventory |
| Separate ordinary public assembly construction and copied progress loops | Runtime builder and execute over the existing engine |
| Independent Portfolio checked_collections / root-only validator | Root-derived typed Repeat plus immutable execution plan |
| Public untyped occurrence modifiers / ambiguous Operation instance config | Typed maintained defaults interpreting one checked input |
| Duplicate decimal/range implementation | Shared Unsigned256 mechanics with exact owning schemas preserved or deliberately versioned |

Current implementation references for this cutover are
[authoring](crates/kernel/program/src/authoring.rs),
[capability contracts](crates/kernel/capabilities/src/lib.rs),
[assembly](crates/kernel/runtime/src/assembly.rs),
[execution](crates/kernel/runtime/src/engine.rs),
[native transaction stages](crates/domains/evm/src/transaction/stages.rs),
[fixture composition](crates/live/evm/tests/support/contract_workflow.rs), and
[application inventory](crates/app/src/inspection.rs). These describe migration inputs, not parallel
public designs to preserve.

Do not implement recompose, author_operation, State::expand, `Deploy<C>`, replacement StateDefinition
bodies, config.contracts(), arbitrary input predicates, with_selection/with_policy, a new driver,
generic Runtime product validator, native cache/blob store, error Either, or parallel registries.
Do not turn every codec into a State or keep superseded implementations behind convenience wrappers.
Retain useful native custody/provider primitives, exact causal diagnostics, validation, safety
barriers, and independent E2E oracles.

## 12. Implementation sequence

First compile the specified contracts in a bounded cross-crate slice: fixed States, two distinct
native ABIs, exact prepared/evidence values, defaults, recursive injection, Repeat, and cold
inventory. This proves the design; it does not delegate product vocabulary or ownership to an
engineer. Adjust private Rust bounds as necessary without weakening the specified guarantees.

Use coherent logical commits, merging inseparable cuts:

1. Establish shared scalar/domain contracts and native implementation interfaces with their typed
   constructors, schemas, and consuming proof. Do not expose a second maintained runtime path.
2. Cut over authoring, typed injection/defaults/resolution, exact Program descriptors, association,
   native codecs, cold decoding, and their consumers together. Remove mutable DSL/wrapper/registration
   paths in this cutover. Include root-map removal here when required for one coherent API/wire.
3. Complete original-failure/report and product-projection migration if independently coherent;
   otherwise keep it with step 2. Remove the native outcome suffix and migrate its exact semantics.
4. Consolidate execute/checked results on the existing engine and prove native pending waiting,
   cancellation, ambiguity, and exact RunId. Keep direct access; no new timing API.
5. Complete maintained lifecycle callers and managed acceptance, replacing fixture equivalents while
   retaining fault infrastructure, external oracles, Portfolio and transport/discovery coverage.

Every implementation commit updates current design/architecture and relevant transport/rustdoc
contracts. Delete superseded APIs/tests/docs with executable replacements. Report removed complexity,
necessary additions, and actual production-code LOC separately from test/docs changes; do not claim
reduction before measuring. A staged proof must not become a permanent alternate DSL or engine.

As part of the capability/authoring cutover, add `docs/capability-authoring.md` and link it from the
README and relevant public rustdoc. This is a required implementation deliverable, not a second
specification to publish against unimplemented APIs. Explain when to reuse an existing capability,
when new semantics require a new capability contract, and when request specialization, prepared
values, or supporting States are necessary. Contrast the fixed `ContractRead` contract with the
request-specialized transaction contract and trace request, prepared command, capability evidence,
and State output through the maintained deployment example. Explain native codec ownership,
common transaction identity versus applied result, and the different responsibilities of component
consumers, State authors, and capability implementation authors. Link to authoritative contracts
and consuming compiled examples rather than maintaining a duplicate set of signatures. Do not
turn the illustrative transfer into an additional implementation requirement.

## 13. Verification contract

| Boundary | Required evidence |
| --- | --- |
| Public construction | All five complete callers; no consumer context/alias/map/codec/registration list; no executable-State Default bound |
| Capability authoring documentation | New guide linked from README/rustdoc; fixed and request-specialized examples match compiled production contracts; decision criteria distinguish codecs, prepared values and persisted supporting States |
| Static typing | Incompatible adjacent/expanded/nested endpoints fail compilation; supported tuple nesting and typed empty identity |
| Generic native reuse | Preserve a second supported context and an ordinary call to an existing address through maintained typed components, without fixture-specific Runtime machinery |
| Shared scalar | Full unsigned-256 range, canonical decoding, zero, maximum, overflow, native schema equivalence where retained |
| Capability family | Same non-generic State with distinct native command/evidence/error ABIs; exact request-type schemas; supported roles and no fallback |
| Native request custody | Actual predecessor84 produces native84 before reservation; forged request84/native42, wrong schema/binding/implementation/action rejected at owning hot/cold boundary; no duplicate Configure validation |
| Recursive injection | Nested supporting selection uses same walk/bindings; no product re-resolution; finite types, depth/State-count rejection, atomic builder failure |
| Defaults/checkpoints | Parent/child inheritance, explicit zero, handler/parameter/target unit, distinct repeated scopes, duplicate/missing/foreign/forward/terminal/context mismatch, inherited-parent-target non-rebinding and Effect barriers |
| Portfolio planning | Actual root-derived collection count/order/routes, immutable continuation plan, empty Repeat, nested use and cumulative limits |
| Assembly/cold | Automatic support/codecs/handlers, one native callback serving distinct request-specialized ABIs, explicit matching public-binding custody, exact conflicts, unchanged builder on failure, no config/C0/setup fabrication, supported alternatives and typed Program reconstruction |
| Evidence | Checked projection before settlement and repeated hot/cold interpretation; native/semantic ref separation including Reads; exact original bytes; no re-encoding or hidden lookup |
| Product | Maintained42/composed84, target/configuration-point agreement, overflow, malformed ABI, mismatched observed value, custom State zero rejection |
| Original reports | Complete original/call/native evidence and implementation provenance after root removal; exact-type access; Portfolio field projection; report is not admission authority |
| Capacity | Small-bound complete-report and prepared-request overflow preserve acknowledged head/original/authority; no production-maximum allocations |
| Execution | Non-spinning pending, direct resume/read, stopped pending authority, stale observation versus known acknowledgement, exact RunId, cancellation and ambiguous append |
| Failure boundaries | Declared domain/native operational originals retain causes and classification; codec/invariant failures stay invocation errors; no failed-Store audit claim |
| Discovery/later Program | Source RunId/output/head linkage, config deletion, new Program/RunId, unavailable ABI rejection, no replacement of unresolved command |

Before removing an assertion, record its executable replacement and managed owner. Retain reservation
acknowledgement loss, prepared-wire recovery with rejecting signer, transaction-boundary cancellation
and ambiguity, external nonce advancement, cold terminal history/output/nonce, actual SQL causes,
retained-epoch rejection without append, and closed signer owner failures. Preserve client
transport/configuration-deletion/enrichment coverage. Second native test ABI is not production support
for another chain or a mainnet settlement guarantee.

Managed Effect E2E Runtime reconstruction retains the same keystore owner; it is not host-process key
recovery. Unchanged terminal history/output/nonce does not prove absence of provider calls. Use the
pinned Reth/PostgreSQL/solc managed tasks and independent observations; compiled first-party artifacts
remain temporary.

Follow [build and verification](docs/build-and-verification.md): affected focused checks in the
Nix shell, relevant managed cases, and one final CI on the exact implementation candidate. Do not
stack broad gates. For this documentation rewrite, review local links, signatures/workflows and
`git diff --check`; no Rust/managed E2E/CI gate is selected. Production-code LOC change is zero.

## 14. Material uncertainties and handoff gates

The core design is specified. Remaining uncertainties require implementation evidence, not an
alternative capability vocabulary or a new framework layer.

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Exact generic contracts compose on pinned Rust | Derive/coherence/private traversal bounds are uncompiled | Extra erasure or a second path could be introduced | Compile fixed States, typed requests, two native ABIs, recursive support, defaults, Repeat and cold inventory across crates |
| Complete prepared requests fit capacity behavior | Requests retain explicit context/artifact/evidence | Some current workloads or report thresholds may overflow | Measure maintained artifacts and small-bound failure cases without dropping facts or authority |
| Shared scalar extraction preserves native behavior | Range/decimal mechanics move inward | Accepted values or schema/serialization could drift | Boundary/overflow/native equivalence tests and explicit versioning for changed contracts |
| Native artifact/ABI binding is exact | Actual identifiers/selectors must come from the maintained artifact definition | Arbitrary bytecode could be treated as supported semantics | Wrong artifact/schema/ledger and malformed-return tests against independent oracle |
| Native adapters implement the required waiting and identities | Current Pending may return immediately; Read currently shares native/semantic identity | Busy looping or wrong evidence binding | Async cancellation/non-spinning proof and distinct native/semantic Read refs hot/cold |
| Report cutover preserves every consumer fact | Current canonical reports omit in-memory call facts and include mapped roots | Exported diagnostics or product fields could be lost | Field/source comparison, exact original cold decoding, descriptor checks, and product projection tests |

Do not claim sketches are compiled or broaden Runtime's historical validation/custody authority.
Unavailable exact implementations fail explicitly. No deadline, wall-clock limit, optional modifier
API, second registry, or native-only engine is an implicit solution to a failed proof.
