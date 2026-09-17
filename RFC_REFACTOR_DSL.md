# RFC: typed authoring and capability-owned execution

## Status and authority

This RFC specifies the replacement design for MFM's public authoring API. The concrete contracts,
capability inventory, supporting sequences, codec interfaces, and workflows below are the target.
Construction follows DSL -> complete executable Program -> Runtime. Program retains selected
code and already-bound adapter handles in memory; its canonical document contains only public
execution facts. Runtime never completes missing associations.

Rust sketches omit routine derives and implementations where the surrounding contract specifies
behavior; they have not been compiled. Implementation must prove the complete contracts together,
not invent a different ownership model behind the examples.

The fixed-sequence target excludes public branching/repetition combinators. Section 5.3 records
the resulting unresolved Portfolio migration; the complete compiler cutover is not ready until
that existing consumer is accounted for.

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
| Program construction | Typed traversal, capability selection, injection, policy relocation, exact descriptors, public bindings, executable code and bound live adapters |
| Runtime | Execute completed Programs; continuation, local run admission, durable transitions and authorized recovery |
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

Preserve the existing async callback/lifetime behavior in the inward adapter interface. The Effect
callback receives EffectId, semantic and native command references, and typed I::NativeCommand,
and returns:

```rust
Result<EffectAdapterOutcome<I::NativeEvidence>, AdapterError<I::OperationalError>>
```

A Read callback receives the semantic intent reference, native intent reference, and typed
I::NativeIntent, returning typed native evidence or AdapterError<I::OperationalError>. Explicit
binding resources are captured during Program construction (section 6.4). No new executor or waiting trait is needed.

| Moment | Required behavior |
| --- | --- |
| Program construction | Resolve implementation, exact ABI/binding, typed injection, codecs and already-bound live callback |
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
Operation and injected nesting; reject excess before returning Program. Runtime limits
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
supporting selections, typed identity, and checkpoints. New-State authors enter
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
type. Distinct occurrences receive distinct scopes. Relocate to final expanded positions and retain
Runtime's irreversible Effect barrier checks.

### 5.3 Planning checks and excluded composition features

This refactor defines fixed typed sequences. It adds no public `Choice`, `Repeat`, `ItemsFrom`,
conditional composition, or configuration-driven repetition API. Capability implementation
selection remains required and is owned by the native family as described in section 6.1;
it does not expose a general-purpose branching combinator to composition authors.

Portfolio currently expands one child sequence for each configured collection. Removing the
proposed repetition API leaves that existing consumer's migration unresolved; it does not authorize
removing collection support, replacing it with a fixed count, or retaining a parallel legacy DSL.
Before the compiler cutover can replace all current consumers, resolve that migration explicitly
against actual Portfolio requirements. Preserve collection count/order/routes, the immutable plan,
and each collection's State boundaries. Do not claim that `checked_collections` or its agreement
checks can be deleted until an equivalent replacement is designed and proved. This is a handoff
blocker for the complete cutover, not permission to introduce repetition under a different name.

Remove arbitrary with_input_check/root predicates, not their guarantees. Checked constructors and
decoders own local invariants; capability setup owns binding/action agreement; planning derives from
the committed input; semantic prerequisites belong to their checked contracts/owning States.
Compilation cannot simulate preceding States to validate a future 84. Native preparation consumes
that future value during execution. Retain Program validation for decoded/untrusted documents,
exact schemas, content identity, and capacity even where Rust proves authored connections.

## 6. Resolution, compilation, and executable association

### 6.1 Profiles and one structural walk

```rust
pub trait CapabilityFamily<C> {
    type Selection;
}

pub trait Resolve<Config, C>: CapabilityFamily<C> {
    fn resolve(config: &Config) -> Result<Self::Selection, ProgramError>;
}
```

A production profile declares supported typed implementations per capability family.
A transaction family can implement these generically over supported R, sharing the
configuration match; adding Configure does not require another handwritten native registry.
Each selected leaf supplies a concrete I and checked I::Binding. All offered alternatives must
support the selected State's typed injection. Use a narrower admitted family if necessary;
never silently fall back to a network incapable of the advertised operation.

Selection is a concrete resolved leaf for a single implementation, or a family-owned Rust enum
whose variants hold the supported concrete implementations and checked bindings. The family's
expert integration interface must expose selected construction and exact cold discovery through
checked Program-owned leaf constructors. A private/sealed trait cannot inspect an enum defined in
another crate; the cross-crate interface is a proof gate in section 14, not an implicit Rust feature.
Public tuple/Operation authoring remains sealed. Selected construction and cold loading reuse the
same executable constructors. It introduces no public generic choice algebra, untyped executable
selection, or additional registration list.

Resolve exact bindings from checked configuration; do not add a public binding-role type parameter
or multi-account composition API for hypothetical use. Binding identity and cross-State agreement
remain checked. At entry, Config is the actual checked root input. Nested tuples/Operations inherit
it; compilation commits that exact root input.

The private traversal is one sealed structural walk with two modes:

| Source | Compile mode | Inventory mode |
| --- | --- | --- |
| Tuple | Walk children with unchanged borrowed config | Walk child types |
| Operation | Enter scope, resolve typed defaults, walk body | Visit declared handler/target and body types |
| Unresolved selection | Resolve profile choice/binding then use typed injection | Visit family alternatives and their injection types |
| Resolved supporting selection | Use supplied I/binding through the same injection | Visit I/prefix/designated/suffix types without values |

Compile leaf bounds require Resolve<Config, C>; inventory requires only structural family and
injection support. Inventory never calls resolve/surround/default resolution, fabricates C0, or loads
source configuration. Runtime does not perform this authoring traversal during progression.

### 6.2 Complete immutable executable Program

Program is the complete in-process result of construction. Runtime neither participates in
compilation nor supplies missing executable code or live resource bindings afterward.

```rust
pub struct Program<I: MfmValue, O: MfmValue> {
    inner: Arc<ProgramInner>,
    endpoints: PhantomData<fn(I) -> O>,
}

struct ProgramInner {
    document: ProgramDocument,
    executables: Box<[ExecutableState]>,
}

pub fn compile<P, S, R>(
    entry_point: EntryPointId,
    states: &S,
    input: &S::Input,
    resources: &R,
    limits: ProgramLimits,
) -> Result<Program<S::Input, S::Output>, ProgramError>
where
    S: AuthoringSource;
```

The signature additionally requires private source/profile/resource traversal bounds. P is the
maintained set of supported implementations, such as ContractCapabilities; checked input selects
the network, implementation and public binding. R is a concrete environment of explicitly supplied
resource handles. Selecting P must not require callers to select the network a second time.
A Pure-only source uses NoCapabilities and `&()` and requires no adapter resources.

ProgramDocument and ExecutableState are kernel implementation details, not new consumer layers.
Each executable entry corresponds to exactly one expanded declaration. Both fields are mandatory;
there is no optional executable companion or public half-constructed Program.

| Persistent, content-addressed document | Nonserialized executable realization |
| --- | --- |
| Expanded State sequence and exact implementation/value contracts | State evaluate/prepare/interpret functions and exact codecs |
| Complete checked public bindings and their references | Native translation/projection functions and already-bound adapter callbacks |
| Recovery policies, handler identities/parameters and permitted targets | Original-error classifiers and deterministic handler functions |
| Initial input commitment and limits | Exact code necessary to interpret the declared contracts |

Compilation owns its private draft. It resolves, injects, constructs and binds each designated and
supporting executable, checks contracts/policies/capacity, then freezes both parts together. Failure
returns no Program and publishes no registrations. Construction invokes no State, provider, signer
operation, authority mutation, or Store append. Caller-supplied handles are already constructed.
Local binding checks may inspect their declared public identity. Constructor and binding errors
retain their causal information through Program-owned errors, without importing RuntimeError.

Store each canonical nonsecret public binding Object once in the document, in deterministic order;
exact declaration references must resolve to those values and expected contracts. Reject missing,
conflicting, or wrong-contract bindings. Include this table in Program identity and size limits.
A binding hash alone cannot support cold reconstruction. Credentials, private keys, provider
connections, signer handles and authority handles never enter that document or admitted values.

The executable realization captures explicit adapter handles, never serializes/hashes them, and
never prints them through derived Debug. Clone can share immutable Program storage. Program identity
is document identity, not pointer identity: matching replacement handles can realize the same
persisted Program. Immutability fixes code and resource associations; it does not freeze remote
services. Capture the existing signer handle, not Keystore; preserve its thread-affine ownership.

### 6.3 Durable identities and cold reconstruction

Extend existing Program Execution descriptors with separate native implementation facts:

| Descriptor | Retained facts |
| --- | --- |
| State ABI | State implementation, input/output/failure contracts |
| Semantic capability | Exact capability and command/intent/evidence contracts |
| Native implementation | Exact implementation, native command/intent/evidence, operational-error and public-binding contracts |
| Occurrence | Public binding reference, handler/parameters, allowances, permitted targets |

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

Program owns cold reconstruction:

```rust
pub fn load<P, S, R>(
    canonical_program: &[u8],
    resources: &R,
) -> Result<Program<S::Input, S::Output>, ProgramError>
where
    S: AuthoringSource;
```

As with compile, additional private inventory/binding bounds apply. S and P supply installed code
inventory; they do not reconstruct the authored plan. Load validates the canonical document and
exact endpoints, matches each declaration to installed exact code, decodes its recorded public
binding, and uses the same typed binding and executable constructors as fresh compilation. It
returns the same complete Program. Bind only implementations selected in the stored declarations,
not every alternative discovered during inventory.

Load never calls configuration resolution, surround, or defaults resolution; it never reinjects
States, fabricates initial input, or replans from current configuration. Missing exact implementations
or mismatched resources fail before execution. No semantic-identity fallback is permitted.
Ordinary Deserialize must not return an executable Program. Canonical decoding is a private step;
document-only inspection remains non-executable and need not acquire live resources.

Application retrieves canonical Program bytes through existing Journal/Store boundaries and calls
load before Runtime receives the Program. Program never depends on Store or reconstructs run
history. Runtime read/resume receive the completed Program and check it against the retained
ProgramRef and current facts. A later complete Program can use the same Runtime/Store without
installing more code into Runtime or mutating an earlier Program.

### 6.4 Typed live binding and the execution boundary

Replace Runtime-mutating native registration helpers with maintained resource environments and
typed binding implementations. These interfaces are owned inward, with concrete implementations in
live/composition crates. They bind an already-selected native implementation; they neither resolve
configuration nor register arbitrary States.

```rust
pub trait BindEffect<C, I>
where
    C: EffectCapabilityContract,
    I: EffectImplementation<C>,
{
    type Adapter: EffectAdapter<
        I::NativeCommand, I::NativeEvidence, I::OperationalError,
    >;
    fn bind_effect(
        &self,
        binding: &I::Binding,
    ) -> Result<Self::Adapter, InvocationDiagnostic>;
}

pub trait BindRead<C, I>
where
    C: ReadCapabilityContract,
    I: ReadImplementation<C>,
{
    type Adapter: ReadAdapter<
        I::NativeIntent, I::NativeEvidence, I::OperationalError,
    >;
    fn bind_read(
        &self,
        binding: &I::Binding,
    ) -> Result<Self::Adapter, InvocationDiagnostic>;
}
```

EffectAdapter and ReadAdapter are typed async invocation interfaces in Capabilities. Preserve the
existing boxed-future lifetime and Send/Sync requirements; introduce no executor or waiting trait.
Their invocation shapes are:

```rust
// EffectAdapter<Command, Evidence, Failure>::invoke
fn invoke<'a>(
    &'a self,
    effect_id: &'a EffectId,
    semantic_command_ref: &'a ContentRef,
    native_command_ref: &'a ContentRef,
    command: &'a Command,
) -> Pin<Box<dyn Future<Output = Result<
    EffectAdapterOutcome<Evidence>, AdapterError<Failure>,
>> + Send + 'a>>;

// ReadAdapter<Intent, Evidence, Failure>::invoke
fn invoke<'a>(
    &'a self,
    semantic_intent_ref: &'a ContentRef,
    native_intent_ref: &'a ContentRef,
    intent: &'a Intent,
) -> Pin<Box<dyn Future<Output = Result<Evidence, AdapterError<Failure>>>
    + Send + 'a>>;
```

Move EffectAdapterOutcome (Pending/Settled) from Runtime to Capabilities beside AdapterError; do
not duplicate it. Native custody uses native references; EffectId continues to commit the semantic
command reference. Capabilities has no Program classifier dependency. Program's constructor enforces
I::OperationalError: ClassifyError and captures the corresponding exact classifier.

A maintained native resource environment can implement the transaction binder generically over
supported request types sharing its native ABI. Deployment and configuration reuse native adapter
code and supplied handles; neither requires a per-request registration list. Each supporting
reservation/preparation State uses this same binding path with its already-selected implementation.

Binding validates local correspondence, including sender, signing purpose and authority epoch.
It is not a provider health check or a guarantee of future external success. Actual predecessor
values, signed facts, external chain evidence and current authority retain their execution-time
checks at the owning native boundary. Such checks do not finish an incomplete Program.

Internally, each mode-specific executable contains only its valid deterministic callbacks, exact
codecs, bound native adapter and recovery functions. Use the existing exact Object boundary for the
heterogeneous sequence, with typed construction and exact admission before decoding. No Any context,
untyped semantic request, or opaque original-error custody is added. Native callbacks return their
exact originals through the existing encoding/audit contract; classification remains after durable
failure acknowledgement.

Do not move existing StateStart/ReadStart/EffectPendingStart callbacks wholesale into Program: they
accept Runtime DriverContext and return DriverDisposition. Split deterministic State invocation and
bound capability invocation from the one Runtime mode dispatcher. Only Runtime owns continuation,
Store/Journal transitions, EffectId scheduling, acknowledgement and recovery authorization. Handler
functions already belong to Program and return proposals; moving their association does not move
authorization. Program kernel accessors expose the narrow execution boundary to Runtime without
letting consumers forge executable entries.

## 7. Runtime execution and recovery

```rust
async fn execute<I: MfmValue, O: MfmValue>(
    &self,
    run_id: RunId,
    program: &Program<I, O>,
    input: &I,
) -> Result<ExecutionResult<O>, InvocationFailure>;
```

Runtime::new(store) receives only the execution Store. Execute accepts the completed Program, not an
Operation. It checks exact input commitment and run facts, performs no executable/resource assembly,
and uses the existing engine until terminal success/failure, RecoveryStopped, invocation failure, or caller
cancellation. ExecutionResult has checked terminal success or terminal original failure; accessors
success()/failure() borrow the applicable value/report and retain exact RunId. RecoveryStopped is
InvocationFailure with unresolved command authority, never terminal domain failure.

Direct progression, read and resume likewise receive `&Program<I, O>` with the explicit RunId.
They verify the retained Program identity before using its code; read invokes no State or adapter.
Runtime does not secretly load or compile a Program. Program construction has no Store dependency.
The same immutable Program can serve separate runs; each continuation and history belongs to its
own caller-supplied RunId. Do not add a Program cache or another execution wrapper to hide this handoff.

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
checked EvmContractWorkflowConfig. Downstream composition supplies network-independent
ContractCapabilities and ContractWorkflowConfig covering supported native implementations. The
configuration selects the implementation; the profile names its installed support set. The current
production example supports EVM; another native ABI in proof tests does not claim another shipping
network. No State getters or selection constants are needed.

ContractWorkflowConfig has checked Deserialize and a native configuration variant selected by the
configuration's network field. The EVM variant contains EvmContractWorkflowConfig: native artifact,
execution config, shared requested/increment scalars, and the supported retry/restart allowances.
`initial_input()` delegates to the selected native config to construct DeploymentRequest, encoding
native envelopes once and assigning supported implementation selectors. It performs no IO. There
is no native binding accessor on the shared DeploymentRequest and callers do not supply a second
binding value to override checked configuration.

Maintained live resource types have explicit fields, not a type-erased resource registry:

```rust
pub struct EvmTransactionResources {
    pub route: EvmTransactionRoute,
    pub signer: Arc<dyn Secp256k1Signer>,
    pub authority: Arc<dyn EvmTransactionAuthority>,
    pub provider: Arc<dyn EvmTransactionProvider>,
}

pub struct ContractResources {
    pub transactions: EvmTransactionResources,
    pub read_route: EvmTransactionRoute,
    pub read_provider: Arc<dyn EvmReadProvider>,
}
```

These names are target production types. Their BindEffect/BindRead implementations use existing
native routines and validate configured binding against supplied public routes, sender, purpose,
and authority epoch. The transaction-only environment suffices for a lone Deploy; ContractResources
also supplies Observe's provider. This avoids requiring a Read handle for an Effect-only source.
Route identities describe the caller's association of a provider handle with its public endpoint;
matching them does not attest the remote chain. Native evidence checks remain mandatory.

Examples assume caller-owned IO supplies config_text, explicit Store/live resources and their
public routes, entry_point, limits, and one RunId. TOML is the existing workspace parser in the
consuming application/example, not a new domain dependency. A different Serde format can consume
the same checked config. All named production components below are target APIs, not presently
compiled symbols. Binding occurs inside compile, not in Runtime::new.

### 8.1 One existing Pure State

```rust
let input = CheckedAddition::new("42", "42")?;
let state = Pure::<CheckedAdd>::default();
let program = compile::<NoCapabilities, _, _>(entry_point, &state, &input, &(), limits)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
assert_eq!(result.success().expect("terminal success").to_string(), "84");
```

Production supplies checked scalar construction/addition and its exact overflow failure. No binding,
root failure conversion, or registration list exists for this case.

### 8.2 One existing injected Effect State

```rust
let config: ContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let state = Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default();
let resources = EvmTransactionResources {
    route: transaction_route, signer, authority, provider: transaction_provider,
};
let program = compile::<ContractCapabilities, _, _>(
    entry_point, &state, &input, &resources, limits,
)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
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
let config: ContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let operation = ContractDeploymentLifecycle::default();
let resources = ContractResources {
    transactions: EvmTransactionResources {
        route: transaction_route, signer, authority, provider: transaction_provider,
    },
    read_route, read_provider,
};
let program = compile::<ContractCapabilities, _, _>(
    entry_point, &operation, &input, &resources, limits,
)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
let report = result.success().expect("terminal success");
assert_eq!(report.requested_value().to_string(), "42");
assert_eq!(report.effective_value().to_string(), "42");
assert_eq!(report.observed_value().to_string(), "42");
```

Supported config chooses native values/implementations; the maintained Rust definition owns order
and defaults. Merely admitting increment42 does not execute addition.

### 8.4 A mixed composition of existing Operations and States

```rust
let config: ContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let states = (
    Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    ConfigureAndObserve::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
);
let operation = Operation::new(states);
let resources = ContractResources {
    transactions: EvmTransactionResources {
        route: transaction_route, signer, authority, provider: transaction_provider,
    },
    read_route, read_provider,
};
let program = compile::<ContractCapabilities, _, _>(
    entry_point, &operation, &input, &resources, limits,
)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
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

let config: ContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let operation = Operation::new((
    Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default(),
    Pure::<CheckedAddConfigurationValue>::default(),
    Pure::<RequireNonZeroConfiguration>::default(),
    ConfigureAndObserve::default(),
    Pure::<Validate>::default(),
    Pure::<Report>::default(),
));
let resources = ContractResources {
    transactions: EvmTransactionResources {
        route: transaction_route, signer, authority, provider: transaction_provider,
    },
    read_route, read_provider,
};
let program = compile::<ContractCapabilities, _, _>(
    entry_point, &operation, &input, &resources, limits,
)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
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
and matching public bindings from the retained Program document, without source config. A standalone
report still needs that document (or its exact binding values) for projection; a reference alone
cannot supply them. Diagnostic projection uses pure translators, not provider IO.
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
| kernel/capabilities | Semantic/native Read/Effect, typed adapter/binding interfaces and EffectAdapterOutcome; no State outcome or classifier dependency |
| kernel/program | Typed construction, immutable document/executable sequence, automatic code inventory, live binding and cold load; no Store/Journal or Runtime dependency |
| domains/transactions | `TransactionEffect<R>`, `PreparedTransaction<R>`, evidence, shared values and concrete lifecycle States/contracts |
| domains/evm | Native config/artifact contracts, request recipes, supporting States, native translation/projection and native operational originals |
| live/evm and downstream composition | Typed resource environments/binders, native adapters, supported ContractCapabilities/configuration; no handwritten executable inventory |
| app | Supported wire/config use cases, checked product projections, inspection consuming structural inventory |
| kernel/runtime | One mode dispatcher/transition engine over complete Program, continuation, recovery authorization and checked results |
| journal/store/backends | Existing owned physical/wire contracts; separate native authority implementations remain separate ports |

EvmContractWorkflowConfig has checked Deserialize and the concrete native fields described in section
8. It lives with native deterministic domain contracts and needs no TOML dependency. A consuming
application owns format parsing/IO. Native wire consensus/signing qualification remains with its
current appropriate live/platform owner; the compiler binds explicit callbacks but never invokes provider IO. Runtime invokes them only at
its authorized execution boundaries. Program may retain bound callbacks; deterministic State
functions and Program construction remain free of IO.

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
| register_fixture_states and native State-registration helper | Program-owned exact executable construction, including support/codecs/handlers |
| Application handwritten compiled State registration inventory | Inspection fed from the same structural source/type inventory |
| RuntimeAssembly/Builder, Runtime-owned ExecutableProgram and native register_*_adapters helpers | Complete Program plus typed binders; Runtime::new(store) and execute/read/resume over the existing engine |
| Proposed ExecutableRequirements receiver, RuntimeBuilder::compile and no-op receiver path | Program-owned compile/load with mandatory executable entries; no Runtime construction callbacks |
| Independent Portfolio checked_collections / root-only validator | Replacement unresolved after removing repetition; preserve guarantees and resolve section 5.3 before complete cutover |
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
native ABIs, exact prepared/evidence values, defaults, recursive injection, and cold
inventory. This proves the design; it does not delegate product vocabulary or ownership to an
engineer. Adjust private Rust bounds as necessary without weakening the specified guarantees.

Resolve the Portfolio migration gate in section 5.3 before attempting the complete compiler cutover.
Do not hand the unresolved construction model to an engineer as an implementation detail.

Use coherent logical commits, merging inseparable cuts:

1. Establish shared scalar/domain contracts and native implementation interfaces with their typed
   constructors, schemas, and consuming proof. Do not expose a second maintained runtime path.
2. Cut over authoring, typed injection/defaults/resolution, complete Program/document/binding schema,
   adapter interfaces, Runtime callback split, native codecs, cold load and explicit read/resume
   handoff with their consumers together. Move EffectAdapterOutcome inward. Remove mutable
   DSL/wrapper/registration/Runtime assembly paths in this cutover. Include root-map removal here when required for one coherent API/wire.
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
and State output through the maintained deployment example. Document complete Program construction,
public binding persistence versus live handle custody, and configuration-free cold load before
Runtime read/resume. Explain native codec ownership,
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
| Capability family | Same non-generic State with distinct native command/evidence/error ABIs; exact request-type schemas; checked binding agreement and no fallback |
| Native request custody | Actual predecessor84 produces native84 before reservation; forged request84/native42, wrong schema/binding/implementation/action rejected at owning hot/cold boundary; no duplicate Configure validation |
| Recursive injection | Nested supporting selection uses same walk/bindings; no product re-resolution; finite types, depth/State-count rejection, atomic construction failure |
| Defaults/checkpoints | Parent/child inheritance, explicit zero, handler/parameter/target unit, distinct occurrence scopes, duplicate/missing/foreign/forward/terminal/context mismatch, inherited-parent-target non-rebinding and Effect barriers |
| Portfolio migration gate | Resolve section 5.3 before replacing its authoring path; preserve configured collection count/order/routes, empty collections, immutable continuation plan, State boundaries and cumulative limits |
| Construction/cold | Mandatory executable entries; automatically bound support/codecs/handlers; native adapter reuse across request ABIs; persisted public bindings; missing/mismatched resources rejected before return with no IO; no config/C0/setup fabrication; same exact Program identity after load |
| Runtime handoff | Explicit Program for execute/read/resume; retained ProgramRef mismatch rejected before execution; unchanged current-state validation, no late binding or hidden load hook |
| Resource custody | No handles/secrets in canonical bytes/debug/context/history; existing Keystore affinity preserved; public endpoint identity is not provider authentication |
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

The fixed-sequence lifecycle design is specified. Portfolio migration remains an explicit design
gap after removing optional composition constructs. Other uncertainties require implementation
evidence, not an alternative capability vocabulary or a new framework layer.

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Existing Portfolio can migrate without the removed repetition API | Its current expansion loops over configured collections; no replacement is specified | Complete compiler deletion/cutover is blocked | Review actual Portfolio requirements and agree its migration before implementation; preserve behavior or explicitly approve a separate product scope change |
| Exact generic contracts compose on pinned Rust | Derive/coherence/private traversal bounds are uncompiled | Extra erasure or a second path could be introduced | Compile fixed States, typed requests, two native ABIs, recursive support, defaults and cold inventory across crates |
| External native families can participate in typed construction | The compiler cannot inspect arbitrary external enums or accept implementations of private traits | Cross-crate selection/inventory could require duplicated lists or an unsafe escape hatch | Prove a narrow family interface with two native alternatives and checked leaf constructors; keep public source composition sealed |
| Complete Program callbacks preserve Runtime phase ownership | Current registered runners include Runtime driver context and typed encoding | Moving whole runners would move transitions inward or alter original custody | Separate prepare/check/invoke/project/interpret/classify and prove encode-once and acknowledgement ordering |
| Public bindings can be persisted completely | Current Program wire keeps references; document table and capacity are new | Cold load could need hidden config or exceed bounds | Audit fields and measure checked Object/document encoding, wrong-binding rejection, and small-bound failures |
| Explicit Program loading supports existing read/resume | Current Runtime hides admission decode and executable association | Application could regain a hidden compiler or lose current-state checks | Retrieve the stored document through existing boundaries, load before Runtime, and test ProgramRef mismatch/config deletion |
| Complete prepared requests fit capacity behavior | Requests retain explicit context/artifact/evidence | Some current workloads or report thresholds may overflow | Measure maintained artifacts and small-bound failure cases without dropping facts or authority |
| Shared scalar extraction preserves native behavior | Range/decimal mechanics move inward | Accepted values or schema/serialization could drift | Boundary/overflow/native equivalence tests and explicit versioning for changed contracts |
| Native artifact/ABI binding is exact | Actual identifiers/selectors must come from the maintained artifact definition | Arbitrary bytecode could be treated as supported semantics | Wrong artifact/schema/ledger and malformed-return tests against independent oracle |
| Native adapters implement the required waiting and identities | Current Pending may return immediately; Read currently shares native/semantic identity | Busy looping or wrong evidence binding | Async cancellation/non-spinning proof and distinct native/semantic Read refs hot/cold |
| Report cutover preserves every consumer fact | Current canonical reports omit in-memory call facts and include mapped roots | Exported diagnostics or product fields could be lost | Field/source comparison, exact original cold decoding, descriptor checks, and product projection tests |

Do not claim sketches are compiled or broaden Runtime's historical validation/custody authority.
Unavailable exact implementations fail explicitly. No deadline, wall-clock limit, optional modifier
API, second registry, or native-only engine is an implicit solution to a failed proof.
