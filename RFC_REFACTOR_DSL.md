# RFC: typed authoring and capability-owned execution

## Status and authority

This RFC specifies the replacement design for MFM's public authoring API. The concrete contracts,
capability inventory, supporting sequences, codec interfaces, and workflows below are the target.
Construction follows DSL -> complete executable Program -> Runtime. Program retains selected
code and already-bound adapter handles in memory; its canonical document contains only public
execution facts. Runtime never completes missing associations.

Rust sketches omit routine derives and implementations where the surrounding contract specifies
behavior; the complete sketches have not been compiled. Section 15 distinguishes a bounded
scratch compilation experiment from the still-unproved production contracts. Implementation must
prove the complete contracts together, not invent a different ownership model behind the examples.

The target uses typed tuples and Operation planning, including homogeneous collections required by
Portfolio. It excludes a separate branching/repetition DSL. Section 5.3 specifies that construction
model; sections 2.5, 4.3 and 9.1 specify the balance contracts and their migration. Complete
cutover still requires the consuming proofs and verification in sections 13–15.

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
contracts and implementations. For now, their integration must also publish each newly supported
source once in its installed component set so a cold process can find the executable code. This
accepted limitation applies to new executable semantics, not rearranging existing components;
automatic discovery of arbitrary downstream implementations is deferred. Derive State, codec,
handler and injected-support requirements from that publication, without separate lists for each.
Use `operation` for an Operation value and `states` for a tuple.
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

### 2.5 Reusable semantic balance contracts

Balance collection uses the same capability and injection machinery as the transaction lifecycle.
Shared balance contracts contain no EVM types and introduce no universal asset framework:

```rust
pub struct BalanceTarget {
    ledger: LedgerIdentity,
    native: Object,
}

pub struct BalanceSource {
    source_id: String,
    target: BalanceTarget,
}

pub struct DecimalScale(u8); // checked 0..=30

pub struct BalanceRequest {
    sources: Vec<BalanceSource>,
    decimals: DecimalScale,
}

pub struct BalanceContext<K: MfmValue> {
    request: BalanceRequest,
    caller: K,
    metadata: BalanceCollectionMetadata,
    completed: Vec<ConfirmedBalance>,
}

pub struct PreparedBalance<K: MfmValue> {
    context: BalanceContext<K>,
    observed_at: ObservationPoint,
    source_decimals: DecimalScale,
}

pub struct CandidateBalance<K: MfmValue> {
    prepared: PreparedBalance<K>,
    raw_units: Unsigned256,
}

pub struct ConfirmedBalance {
    source: BalanceSource,
    observed_at: ObservationPoint,
    source_decimals: DecimalScale,
    raw_units: Unsigned256,
}

pub struct ReadBalanceAt {
    target: BalanceTarget,
    observed_at: ObservationPoint,
}

pub struct BalanceEvidence {
    intent_ref: ContentRef,
    implementation_ref: ContentRef,
    original: Object,
    outcome: BalanceOutcome,
}

pub enum BalanceOutcome {
    Observed { observed_at: ObservationPoint, raw_units: Unsigned256 },
    Rejected,
    SafeFailure,
    IntegrityBlocked,
}

pub struct BalanceRead;
impl ReadCapabilityContract for BalanceRead {
    type Intent = ReadBalanceAt;
    type Evidence = BalanceEvidence;
    // Exact identity and evidence binding follow section 3.
}

pub struct ObserveBalance<K>(PhantomData<fn() -> K>);
impl<K: MfmValue> State for ObserveBalance<K> {
    type Input = PreparedBalance<K>;
    type Output = CandidateBalance<K>;
    type Failure = ObserveBalanceFailure;
    // Read prepare/interpret construct intent and interpret checked BalanceOutcome.
}
```

All value fields are private; constructors and decoders enforce their owning invariants. Source IDs
retain the current nonempty, bounded public-text checks. BalanceRequest preserves nonempty sources,
at most 64, unique source IDs, declaration order and one ledger per collection. The native owner
additionally qualifies the exact ledger/route/target contracts. BalanceCollectionMetadata retains
checked collection ordinal, correlation and public route reference; native failures do not carry
Portfolio-specific metadata. Derive the active source from completed.len(), without a second mutable
index. Completed values must match the corresponding request prefix and common observation point.

K is the exactly typed caller continuation, not a network/capability parameter. Preserve current
snapshot and enrichment reuse through their distinct continuations. Consumers reuse maintained
contracts and definitions; they do not construct context aliases. This introduces neither a generic
carry/lift mechanism nor an erased caller context. Deploy remains a concrete non-generic State.

BalanceTarget is one purpose-specific native boundary like ContractLocator. Its EVM payload contains
the account and native/token asset descriptor; only the capability implementation decodes addresses
or token ABI facts. ObservationPoint already carries the exact native number/hash needed for anchor
confirmation. Do not add another opaque protocol-context Object. Native originals remain in their
acknowledged Read calls. BalanceEvidence uses the existing immutable-original projection protocol;
CandidateBalance stores only prepared facts and units, not a copied evidence history.

ObserveBalance returns a candidate only for checked Observed evidence at the requested point.
Rejected/SafeFailure and authenticated IntegrityBlocked are semantic failure interpretations
(section 9.1), not local codec errors. Only the native confirmation suffix may advance completed
results. Its shared semantic append helper checks request-prefix agreement and amount semantics;
context mismatches remain invocation diagnostics, not fabricated external failures.

Raw units retain the full unsigned-256 range. Scaled values and totals retain the existing 80-digit
limit: a private checked DecimalUnits80 in the balance domain owns canonical nonnegative decimal
arithmetic. Do not narrow totals to Unsigned256 or add a general arbitrary-precision framework.
Reuse/move the existing mechanics; decimal scale and the 80-digit product bound stay explicit.

Preserve the current scaling policy: native source scale equals the configured collection scale
(not an implicit 18); token scale is read at the anchor and lies in 0..=30. Scale-down returns zero
for amounts below one target unit; otherwise discarded nonzero digits are rejected. Scale-up and
summation reject values exceeding 80 digits. No rounding configuration is introduced.

Explicitly fix zero upscaling: scale_units("0", 0, 2) currently constructs noncanonical "000", which
subsequent summation rejects. The new arithmetic returns canonical "0" for zero at every scale.
This is a deliberate correctness change; retain the other dust/exact-remainder behavior and add
boundary regressions. Semantic arithmetic failure contracts are specified in section 9.1.

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
the maintained environment for I; a resolved supporting selection already supplies I. Both call the same typed
injection routine. These supporting leaves are not an everyday second DSL or native-only emitter.

Expansion walks the prefix recursively, emits the designated executable exactly once, and walks
the successful suffix recursively. Designated emission is a private action; it does not reinject
itself. Each emitted State retains its own ABI, policy scope, original failure, persistence and
recovery boundary. Injection never makes the whole sequence atomic.

Supporting definitions form finite typed structures. Apply depth and total-State limits across
Operation and injected nesting; reject excess before returning Program. Runtime limits
do not make infinitely recursive Rust types valid. Native contracts/codecs are associated
requirements, not additional persisted States merely because they were selected.

### 4.3 Balance preparation, observation and confirmation

EvmNativeBalance and EvmTokenBalance are two concrete implementations of BalanceRead. Both supply
ReadSelection endpoints `BalanceContext<K> -> BalanceContext<K>` around the same designated
ObserveBalance<K>, whose raw endpoints are `PreparedBalance<K> -> CandidateBalance<K>`.

| Expanded Read State | Input | Output |
| --- | --- | --- |
| CheckEvmBalanceChain<K> | BalanceContext<K> | EvmChainChecked<K> |
| ReadInitialEvmBalanceAnchor<K, Native> | EvmChainChecked<K> | PreparedBalance<K> |
| ReadInitialEvmBalanceAnchor<K, Token> | EvmChainChecked<K> | EvmTokenAnchored<K> |
| ReadEvmTokenDecimals<K>, token only | EvmTokenAnchored<K> | PreparedBalance<K> |
| ObserveBalance<K> | PreparedBalance<K> | CandidateBalance<K> |
| ConfirmEvmBalanceAnchor<K> | CandidateBalance<K> | BalanceContext<K> |

The two initial-anchor rows are alternatives selected by the native implementation, not consecutive
States. Native/Token are private preparation specializations sharing anchor-reading logic. Their
associated output types are exact; do not hide an output conversion after State execution or use
optional decimals to represent an incomplete PreparedBalance. EvmChainChecked retains the typed
context and checked native chain facts; EvmTokenAnchored additionally retains the native anchor.
Only native supporting State implementations interpret those EVM facts. The shared point envelope
must preserve the current native anchor number range and hash; normalizing it must not narrow
EvmBlockAnchor's EvmU256 number representation.

Native prefix: chain check, native anchor specialization. Token prefix: chain check, token anchor
specialization, token decimals. Both share the confirmation suffix. Use existing InjectRead,
ResolvedRead and identity native translation for supporting protocols. No extra conversion State,
conditional DSL, native-only engine or custom evidence registry is needed.

Each production source definition owns checked local planning facts derived from the collection:
source ordinal/target, collection scale and public route. Native resolution selects its fixed
implementation shape and derives exact per-source binding. Before affected provider IO, the selected
implementation checks actual active source against planned ordinal, target and route. This is native
request/binding admission, not a generic Runtime product validator. A typed vector of source
Operations with equal BalanceContext<K> endpoints preserves collection order and connectivity.

Preserve the actual observation protocol:

1. Check chain identity for every source.
2. Read the initial anchor at latest for every source. After the first confirmed source, require
   equality with that first source's number/hash; reject differing anchors before continuing.
3. Read token decimals when required and the raw balance at the initial anchor's block number.
4. Re-read that committed block number for confirmation, never latest, and compare number/hash.
5. Only after successful comparison, run shared semantic scaling/append validation and return the
   next BalanceContext. Consolidate after all sources have confirmed.

This preserves current number-based observation/reorg detection; it does not claim finality or
protection against every transient reorg. Retain four Read boundaries per native source and five
per token source, plus the existing collection consolidation and outer handoffs. No successful
intermediate candidate is exposed as a completed collection result.

State interpretation constructs proposed outputs before append; Runtime acknowledgement governs
advancement. Cancellation or failed append cannot claim a prefix/candidate/confirmation was recorded.
Cold resume uses retained typed stage values, exact implementation and binding without replanning.
Each Read's original evidence remains in its own acknowledged frame. A confirmation failure report
contains its candidate input and confirming evidence, not the preceding balance Read's entire
original; historical evidence remains in the run history. No evidence bag is carried through stages.

## 5. One authoring representation and maintained defaults

### 5.1 Tuples and Operations

```rust
pub trait AuthoringSource: sealed::Sealed {
    type Input: MfmValue;
    type Output: MfmValue;
}

pub trait OperationDefinition {
    type Body: AuthoringSource;
}

pub trait Plan<Parent: ?Sized>: OperationDefinition {
    type Config: ?Sized;
    fn plan<'a>(
        &'a self,
        parent: &'a Parent,
    ) -> Result<(&'a Self::Config, Self::Body), ProgramError>;
}

pub struct Operation<Definition, Defaults = Inherit> {
    definition: Definition,
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

Operation::new(definition) is infallible on Operation<Definition, Inherit>, preserving inference. Maintained
Operation::default constructs the same structure with its Defaults marker. Selection defaults
construct markers, not executable States; they require no S: Default. Neither construction performs
IO, binding resolution, input validation, or State execution. A maintained definition need not be a
literal Rust static; it is reusable with different checked inputs and resulting Programs.

Supported tuples implement OperationDefinition with Body=Self and identity Plan: borrow the parent
configuration and clone their construction values. Clone/Default requirements apply only to those
values and selection markers, never to executable State types. The fixed aliases above therefore
keep their current caller syntax. Authors only implement a named definition when introducing new
planning semantics; arranging existing components requires no trait implementation.

At an Operation node, the compiler invokes plan once, resolves that Operation's defaults from its
returned local configuration, then walks the returned Body through the same sealed source traversal.
An ordinary tuple walk does not invoke plan, so identity definitions do not recursively replan.
Leaving the Operation restores its parent's configuration and policy scope. Planning performs no
State execution, provider/signer IO or Store activity. It returns a typed structure, never an
unrestricted mutable emitter. Operation endpoints are its Definition::Body endpoints and cannot
change with configuration values. Bodies and supporting types must form finite structures.

Cold discovery visits Definition::Body and declared policy types without invoking plan, constructing
an Operation value or requiring Plan<RootConfig>. Separate OperationDefinition from Plan for this
reason. The number of occurrences is read from the retained expanded document, not regenerated.

Tuple adjacency requires exact equality of expanded endpoints. Operations are ordinary tuple
elements and retain scopes. No aggregate Failure associated type, construction closure, fluent
parallel DSL, getter collection, selection constants, or caller-defined aliases are required.

Program has no input/output type parameters. For `A -> B -> C -> D`, its executable States retain
the distinct A/B, B/C and C/D contracts. Rust proves each authored adjacent connection through the
typed source before compilation returns one complete `Program`. The compiler retains those exact
contracts and their corresponding typed callbacks; removing endpoint parameters does not introduce
untyped composition, a context bag, or additional erasure within State/capability implementations.
Cold loading verifies the stored connections and associates exact installed implementations before
returning the same `Program` type. It cannot use a compile-time proof for untrusted stored bytes.

Do not add endpoint marker types, a typed Program wrapper, or a `try_typed` conversion. Input is
checked and committed during compilation. Checked result decoding uses the actual recorded output
contract (section 7), without requiring callers to restate the complete composition or its internal
capability requirements. The public result boundary deliberately checks the requested result type
when it is decoded; it does not promise compile-time inference of that type from `Program`.

Framework-controlled sources comprise tuples, Operations, Pure/Read/Effect selections, resolved
supporting selections, homogeneous vectors with equal body endpoints, typed identity, and checkpoints. New-State authors enter
through a State selection, not an arbitrary mutable emitter. Exact supported tuple arities and
nested tuples must be covered in consuming tests, without inventing a dynamic heterogeneous list.

AuthoringSource's endpoints are not all compiler bounds: selection resolution and defaults also
depend on the current borrowed compile configuration. Section 6 defines that traversal. Keep its
mode traits internal instead of exposing a generic visitor DSL.

### 5.2 Configuration and policy defaults

Checked root input is the authoritative per-run configuration source. A maintained Operation may
project a borrowed local configuration or derive child definition values from that checked input.
Those values are planning facts, not a second independently supplied override or future State input.
Operation definitions own structure and interpretation of supported policy values. No with_selection(),
with_policy(), arbitrary root predicate, or generic Scoped wrapper is introduced.

Production supplies a typed view of the lifecycle's retained planning facts:

```rust
pub trait LifecyclePlanning {
    fn deployment_request(&self) -> &DeploymentRequest;
}
```

DeploymentRequest returns itself; DeployedContract returns its retained request. Implement the same
view on later maintained contexts where standalone selections require it. Lifecycle defaults and
native resolution use `Config: LifecyclePlanning + ?Sized`, not a single root Rust type. Thus the
fixed ConfigureAndObserve tuple works alone with DeployedContract or nested under DeploymentRequest,
and ordinary mixed tuples need no consumer conversion. This domain-owned view is not an erased
context or a generic config bag. It exposes already-known network/binding/options/policy facts;
Configure still obtains the actual effective value (84 after addition) from its execution input.

A configuration-dependent child can instead own the exact checked demand derived by its parent's
plan. Its Plan implementation returns a reference to that demand as its local Config. Section 5.3
uses this for Portfolio; child native resolution then needs no knowledge of the parent's root type.
No compiler step fabricates a DeployedContract or evaluates Add to obtain planning information.

```rust
pub trait OperationDefaults {
    type Handler: Handler;
    type Targets: CheckpointTargets;
}

pub trait ResolveDefaults<Config: ?Sized>: OperationDefaults {
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

### 5.3 Configuration-dependent typed bodies and Portfolio

Operation planning may use ordinary Rust iteration over checked configuration to return homogeneous
collections of typed bodies. The sealed source implementation has the following endpoint rule:

```rust
impl<T, B> AuthoringSource for Vec<B>
where
    T: MfmValue,
    B: AuthoringSource<Input = T, Output = T>,
{
    type Input = T;
    type Output = T;
}
```

This is bounded repetition required by existing behavior, not a second compiler or runtime loop.
Each body's internals can have different exact contracts; equality of its outer endpoints proves
that any number of occurrences connects. The compiler expands each element in order, retains each
State boundary and ordinary Operation scope, and applies cumulative depth/State/capacity limits.
Planning must check product bounds before materializing oversized collections; compiler limits still
cover the complete injected sequence. Rust proves connectivity, not the configured count/order/routes;
production planning tests must establish those value-level guarantees.

Portfolio's maintained definition returns this structure (names below are target definitions):

```rust
type PortfolioBody = (
    Pure<InitializePortfolio>,
    Vec<Operation<CollectionDefinition>>,
    Pure<ConsolidatePortfolio>,
);

struct CollectionDefinition {
    demand: CollectionDemand,
}

impl<Parent: ?Sized> Plan<Parent> for CollectionDefinition {
    type Config = CollectionDemand;
    fn plan<'a>(
        &'a self,
        _parent: &'a Parent,
    ) -> Result<(&'a CollectionDemand, Self::Body), ProgramError> {
        Ok((&self.demand, Self::Body::default()))
    }
}
```

CollectionDefinition's associated Body wraps collection entry, balance collection and resumption;
its endpoints are PortfolioContinuation. The parent derives every demand directly from the committed
PortfolioSnapshotInput in declaration order. The balance collection definition likewise derives its
per-source body from its checked source list. No separately maintained checked_collections plan or
consumer-built intermediate context is retained. Cold discovery visits vector element/body types
once and reconstructs the stored occurrences without calling plan or reading configuration.

An empty vector is mechanically an identity. Current PortfolioConfig, PortfolioSnapshotInput and
EvmBalanceRequest reject empty collections/sources: preserve those product rejections rather than
mistaking structural identity support for permission to accept empty requests.

The per-source collection body uses ObserveBalance<K>/BalanceRead with the two fixed native
injection shapes specified in section 4.3. Native/token choice belongs to capability resolution,
not a domain conditional-source algebra. Sections 2.5 and 9.1 specify normalized values, arithmetic
and original failures. Delete old agreement checks only with their typed/construction replacements
and consuming tests; no parallel legacy compiler remains.

No public Choice, Repeat, ItemsFrom, generic conditional-source algebra, or heterogeneous dynamic
emitter is added. Execution-dependent discovery produces a subsequent Program; planning cannot
change an admitted Program based on a future State result.

Remove arbitrary with_input_check/root predicates, not their guarantees. Checked constructors and
decoders own local invariants; capability setup owns binding/action agreement; planning derives from
the committed input; semantic prerequisites belong to their checked contracts/owning States.
Compilation cannot simulate preceding States to validate a future 84. Native preparation consumes
that future value during execution. Retain Program validation for decoded/untrusted documents,
exact schemas, content identity, and capacity even where Rust proves authored connections.

## 6. Resolution, compilation, and executable association

### 6.1 One supported implementation set for construction and cold discovery

```rust
pub trait CapabilityFamily<C> {
    type Implementations;
}

pub trait Resolve<Config: ?Sized, C>: CapabilityFamily<C> {
    fn implementation(config: &Config) -> Result<StableId, ProgramError>;
}

pub trait ResolveEffectBinding<Config: ?Sized, C>: EffectImplementation<C>
where
    C: EffectCapabilityContract,
{
    fn binding(config: &Config) -> Result<Self::Binding, ProgramError>;
}

pub trait ResolveReadBinding<Config: ?Sized, C>: ReadImplementation<C>
where
    C: ReadCapabilityContract,
{
    fn binding(config: &Config) -> Result<Self::Binding, ProgramError>;
}
```

Program owns the support-environment contract; maintained outward integrations implement it:

```rust
pub trait ProgramEnvironment {
    type Sources;
}

impl ProgramEnvironment for ContractResources {
    type Sources = (
        ContractDeploymentLifecycle,
        Pure<CheckedAdd>,
        Pure<CheckedAddConfigurationValue>,
    );
}
```

Sources is a tuple of independent installed source roots, not an executable sequence. Its entries
have no adjacency requirement. The same structural machinery visits their State, handler and
capability types in discovery mode. Actual compiled sequences retain all adjacency requirements.
The environment also implements CapabilityFamily/Resolve and the typed native binders. Remove the
separate public profile parameter/marker: configuration chooses the network from installed support,
not from another caller-selected profile. Resources are explicit handles, not a network override.

Each Pure selection supplies its callbacks and value/failure codecs. Each Read/Effect selection
supplies its meaningful State and semantic contracts. Its selected native implementation supplies
its native codecs, adapter, binding, errors/projections and recursively injected sources. Operation
defaults supply handlers and parameter codecs. Derive these requirements during the same traversal;
no independent registration/emission tree or Runtime receiver is introduced.

A genuinely new State or handler must be reachable from one published source root for cold loading.
Fresh compilation knows its supplied source; it does not automatically install code for future
processes. Cold discovery can only find the environment's installed support. Existing-component
recomposition changes no publication list. Identical exact descriptors discovered through several
roots deduplicate; conflicting or ambiguous implementation claims fail without ordered fallback.
Keep the implementation-site comment and known-gaps entry required in section 12.

Implementations is one tuple of concrete native implementation types, interpreted by sealed
framework traversal. For a family supporting two implementations, the shape is:

```rust
impl CapabilityFamily<TransactionEffect<DeploymentRequest>> for ContractResources {
    type Implementations = (EvmTransactionImplementation, TestTransactionImplementation);
}
```

TestTransactionImplementation illustrates a second supported implementation in proof tests, not
another shipping network. Production lists only its actual support. Request-generic native families
can share this declaration and configuration selection across supported R; adding Configure must
not require another native registration list. This tuple declares supported native code, not an
executable-State list or a branching/repetition source. The typed tuple/Operation representation is
shared with Operation planning and vector traversal. Remove the previously proposed family-owned Selection enum and its implicit external
visitor interface rather than maintaining both representations.

Resolve selects a supported implementation StableId from checked configuration. The compiler
requires a unique matching entry; duplicate/ambiguous family identities or unsupported choices are
errors, never ordered fallback. The selected implementation's ResolveEffectBinding or
ResolveReadBinding constructs its exact typed public binding from that same configuration. These
methods perform no IO and do not obtain live handles. BindEffect/BindRead (section 6.4) subsequently
attach the supplied handles to that already-selected typed binding.

All family entries must support the selected State's injection and the resource environment's
typed binder. Sealed tuple implementations state those bounds for each concrete entry; an external
enum does not need to implement a private trait and no generic visitor with impossible per-entry
bounds is introduced. Use a narrower supported family if a native implementation cannot implement
the advertised action. No public Choice, derive macro, role generic or registry is needed.

The private tuple dispatch handles two binding sources:

```rust
enum BindingSource<'a, Config: ?Sized> {
    Configuration(&'a Config),
    Stored(&'a Object),
}
```

Fresh construction resolves the configured identity and calls the matching native typed binding
constructor. Cold construction matches the complete retained ABI and decodes the corresponding
I::Binding from its recorded Object. Both then call the same checked leaf constructor with that
typed binding and explicit resources. The leaf itself requires no configuration-resolution trait;
resolved supporting selections already supply their binding. Never rerun parent configuration
selection for injected nonce/preparation States.

StableId selects installed code during fresh construction; it is not sufficient cold admission.
Before resource binding, compare the complete stored State, semantic capability, native value/error
and binding contracts with the installed exact descriptor. No fallback to a matching name/version
with different schemas. Detect multiple incompatible installed matches before publishing Program.

The compiler uses one structural traversal for public sources and injected source types:

| Source | Compile mode | Cold discovery mode |
| --- | --- | --- |
| Tuple | Walk children with unchanged borrowed config | Walk child types |
| Operation | Plan once, enter local config/scope, resolve defaults, walk typed body | Visit Definition::Body and declared handler/target types without Plan bounds |
| Homogeneous vector | Walk every occurrence in order, with cumulative limits | Visit its element type, not configured occurrences |
| Unresolved selection | Match family tuple, construct typed binding, inject | Inspect the same family tuple and its injection types |
| Resolved supporting selection | Use supplied I/binding through the same injection | Visit I/prefix/designated/suffix types without values |

At entry Config is the actual checked root input. Tuples inherit the current borrowed configuration;
Operation planning may establish a local view or derived demand. Cold construction receives no
configuration value and never calls Plan, Resolve, typed binding construction, surround or defaults
resolution. Cold traversal must not require a fresh Plan<Root> implementation. Resolved supporting
selections already carry their selected binding and do not rerun root resolution. Program construction
commits the exact root input and checks binding agreement across the workflow. Runtime performs none
of this traversal.

### 6.2 Complete immutable executable Program

Program is the complete in-process result of construction. Runtime neither participates in
compilation nor supplies missing executable code or live resource bindings afterward.

```rust
pub struct Program {
    inner: Arc<ProgramInner>,
}

struct ProgramInner {
    document: ProgramDocument,
    executables: Box<[ExecutableState]>,
}

pub fn compile<S, R>(
    entry_point: EntryPointId,
    states: &S,
    input: &S::Input,
    resources: &R,
    limits: ProgramLimits,
) -> Result<Program, ProgramError>
where
    S: AuthoringSource,
    R: ProgramEnvironment;
```

Additional sealed traversal/planning/binding bounds apply. S and R are inferred from the supplied
source and maintained environment. Checked configuration selects the implementation and public
binding. A Pure-only caller uses that same maintained environment with no attached live handles;
its installed source types still support cold discovery. There is no NoCapabilities marker or
per-call capability-profile argument.

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
pub fn load<R>(
    canonical_program: &[u8],
    resources: &R,
) -> Result<Program, ProgramError>
where
    R: ProgramEnvironment;
```

Additional sealed discovery/binding bounds apply. R::Sources and its supported native families
supply installed code; callers do not name the original source or capability profile. They do not
reconstruct the authored plan. Load validates the canonical document and exact endpoints, matches each declaration to installed exact code, decodes its recorded public
binding, and uses the same typed binding and executable constructors as fresh compilation. It
returns the same complete Program. Bind only implementations selected in the stored declarations,
not every alternative discovered during inventory.

Load never calls Operation planning, configuration resolution, surround, or defaults resolution.
It never reinjects States, fabricates initial input, or replans from current configuration. Missing exact implementations
or mismatched resources fail before execution. No semantic-identity fallback is permitted.
Ordinary Deserialize must not return an executable Program. Canonical decoding is a private step;
document-only inspection remains non-executable and need not acquire live resources.

Runtime owns the private admission-record wire, so Application must not duplicate that parser to
obtain the stored Program. Add this narrow read-only bootstrap method:

```rust
impl Runtime {
    pub async fn program_document(
        &self,
        run_id: &RunId,
    ) -> Result<Object, InvocationFailure>;
}
```

It loads through the existing Store admission/latest snapshot, checks the Journal admission
envelope, RunId, admission sequence/variant, available metadata and Program reference linkage, and
returns the retained Program Object without re-encoding. It neither resolves code/resources nor
claims a checked RunView or validates the current continuation. Errors retain the requested RunId
and no fabricated last observation. Canonical Program validation remains in its owning decoder.

The cold caller workflow is explicit:

```rust
let document = runtime.program_document(&run_id).await?;
let program = load(
    document.canonical_bytes(), &resources,
)?;
let result = runtime.resume(&run_id, &program).await?;
```

Program load has no Store dependency. Runtime read/resume take a fresh existing snapshot and compare the
supplied Program identity with its admitted document before current-state validation. Progress
between extraction and resume is allowed: admission is immutable, and no old head grants append
authority. Preserve all existing snapshot/head and retained-state checks. Runtime never calls load
or compiles a Program internally. A later complete Program can use the same Runtime/Store without
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

### 6.5 Callback failure phase and original custody

Program's internal executable boundary distinguishes invocation failure phase without importing
RuntimeError or duplicating Runtime's execution-operation enum:

```rust
pub enum CallbackFailure {
    Decode(InvocationDiagnostic),
    Execute(InvocationDiagnostic),
    Encode(InvocationDiagnostic),
}
```

This kernel interface is invocation-only, not a persisted domain/operational failure contract.
Runtime knows the operation it called (prepare, native adapter, evidence binding, interpretation,
classification or handler) and wraps the returned phase/cause at that call site. Keep compound
callbacks separated where their operation provenance differs. Runtime alone adds RunId, retained
StateCall, known acknowledgement and last observation. No generic original-error bag is introduced.

Each executable captures its exact declared original-error contract; Runtime supplies the current
execution position for diagnostic context. Encode a concrete State/native original exactly once
before returning its Object across the heterogeneous boundary. Failure retains position, expected
contract, encoding cause/size facts and explicitly unavailable original detail/identity; it never
retries the serializer or invokes classification. Runtime forms the complete existing Failure from
its retained call/command facts and the encoded original. Classification is a separate callback,
invoked only after Runtime has acknowledged the original failure.

Move the existing encode_failure implementation and adapter callback/poll panic containment inward
with those typed wrappers; delete the Runtime-local copies in the same cutover. Preserve immediately
awaited pure blocking encoding with the existing workspace Tokio dependency in Program. This adds
an existing direct dependency for necessary async wrapper work, not a new executor abstraction or
third-party package. Never move provider IO, Store access or mutation authority into that job.
Runtime retains frame encoding, sealing, append and Store-error handling.

Report decode failures as Decode, callback/invariant failures as Execute, and encoding failures or
encoding-job panics as Encode. This deliberately corrects current adapter decode/normal-encoding
panic paths that sometimes report Execute. Preserve their causes and test the new phase explicitly;
it changes diagnostic precision, not recovery classification or acknowledgement semantics.
Panic containment must not include panic payloads in returned diagnostics. It does not by itself
control process panic-hook logging; do not claim that this proof changes the process logging contract.

## 7. Runtime execution and recovery

```rust
async fn execute<I: MfmValue>(
    &self,
    run_id: RunId,
    program: &Program,
    input: &I,
) -> Result<ExecutionResult, InvocationFailure>;
```

Runtime::new(store) receives only the execution Store. Execute accepts the completed Program, not an
Operation. It checks the input's exact recorded contract and value commitment before provider IO or
append, validates retained run facts, and performs no executable/resource assembly. It uses the
existing engine until terminal success/failure, RecoveryStopped, invocation failure, or caller
cancellation. ExecutionResult has checked terminal success or terminal original failure; accessors
success()/failure() borrow the applicable value/report and retain exact RunId. RecoveryStopped is
InvocationFailure with unresolved command authority, never terminal domain failure.

ExecutionResult is not generic. Its accessors expose the existing checked persistence boundary:

```rust
impl ExecutionResult {
    pub fn success(&self) -> Option<&Object>;
    pub fn failure(&self) -> Option<&FailureReport>;
    pub fn run_id(&self) -> &RunId;
}

// Existing Values API; reuse it rather than adding a result conversion layer.
impl Object {
    pub fn decode<T: MfmValue>(&self) -> Result<T, InvocationDiagnostic>;
}
```

Runtime admits the successful Object against the terminal State's exact output contract before
exposing it. `success()` returns None only for terminal failure. Decoding checks T's exact schema
identity before invoking its codec; a different schema is an error even if its JSON shape matches.
Codec failures retain the existing InvocationDiagnostic cause contract. A failed inspection neither
changes the acknowledged outcome nor invokes recovery, IO, or a Store append. Failure inspection
continues to expose the original report and its exact checked originals. Fresh execution and cold
resume use these same accessors. Consumers select a production result type only when inspecting it;
this does not restrict intermediate State contracts or require converting the Program.

Direct progression, read and resume likewise receive `&Program` with the explicit RunId.
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
`mfm_chain::transaction::contract_lifecycle` and the shared transaction module. The EVM domain exports
checked EvmContractWorkflowConfig. Downstream composition supplies network-independent
ContractResources and ContractWorkflowConfig covering supported native implementations. The
configuration selects the implementation; the resource environment identifies installed support. The current
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

pub struct EvmReadResources {
    pub route: EvmTransactionRoute,
    pub provider: Arc<dyn EvmReadProvider>,
}

#[derive(Default)]
pub struct ContractResources {
    pub transactions: Vec<EvmTransactionResources>,
    pub reads: Vec<EvmReadResources>,
}
```

These names are target production types. Their BindEffect/BindRead implementations use existing
native routines and validate configured binding against supplied public routes, sender, purpose,
and authority epoch. A selected binder requires a unique corresponding typed resource; missing or
ambiguous matches reject construction. These vectors supply handles for multiple public routes,
not erased executable registrations. Pure callers supply empty vectors; Deploy needs only a
transaction entry; the full lifecycle also needs a Read entry. Unselected native implementations
require no handles. Add fields for another native implementation only when that support ships.
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
let resources = ContractResources::default();
let program = compile(entry_point, &state, &input, &resources, limits)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
let sum = result.success().expect("terminal success").decode::<Unsigned256>()?;
assert_eq!(sum.to_string(), "84");
```

Production supplies CheckedAddition input, CheckedAdd with Unsigned256 output, checked scalar
construction/addition and its exact overflow failure. No binding, root failure conversion, or
registration list exists for this case.

### 8.2 One existing injected Effect State

```rust
let config: ContractWorkflowConfig = toml::from_str(&config_text)?;
let input = config.initial_input()?;
let state = Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default();
let resources = ContractResources {
    transactions: vec![EvmTransactionResources {
        route: transaction_route, signer, authority, provider: transaction_provider,
    }],
    reads: vec![],
};
let program = compile(
    entry_point, &state, &input, &resources, limits,
)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
let deployed = result.success().expect("terminal success").decode::<DeployedContract>()?;
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
    transactions: vec![EvmTransactionResources {
        route: transaction_route, signer, authority, provider: transaction_provider,
    }],
    reads: vec![EvmReadResources { route: read_route, provider: read_provider }],
};
let program = compile(
    entry_point, &operation, &input, &resources, limits,
)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
let report = result.success().expect("terminal success").decode::<ContractDeploymentReport>()?;
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
    transactions: vec![EvmTransactionResources {
        route: transaction_route, signer, authority, provider: transaction_provider,
    }],
    reads: vec![EvmReadResources { route: read_route, provider: read_provider }],
};
let program = compile(
    entry_point, &operation, &input, &resources, limits,
)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
let report = result.success().expect("terminal success").decode::<ContractDeploymentReport>()?;
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

The new State and its original failure are authored below. Its integration also publishes this
new source once for cold executable discovery, under the accepted limitation in section 1; ordinary
composition does not maintain an executable-registration list.

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
    transactions: vec![EvmTransactionResources {
        route: transaction_route, signer, authority, provider: transaction_provider,
    }],
    reads: vec![EvmReadResources { route: read_route, provider: read_provider }],
};
let program = compile(
    entry_point, &operation, &input, &resources, limits,
)?;
let runtime = Runtime::new(store);
let result = runtime.execute(run_id, &program, &input).await?;
let report = result.success().expect("terminal success").decode::<ContractDeploymentReport>()?;
assert_eq!(report.observed_value().to_string(), "84");
```

ProgramError's identity-construction conversion must retain the source. The empty failure does not
discard its rejected input: Runtime's complete Failure retains the originating call and prior facts.
A companion zero-input/zero-increment case cold-decodes exactly ZeroConfiguration and proves no
configuration command was prepared. No enclosing failure conversion or separate per-State,
codec, handler or adapter registration list is added. Publishing the new source in the integration's
installed component set remains necessary until automatic downstream discovery is designed.

For this extension example, the composing integration owns ContractResources. It replaces its one
section 6.1 publication declaration with the following; these are alternative complete versions of
one implementation, not two concurrent impls or registries:

```rust
impl ProgramEnvironment for ContractResources {
    type Sources = (
        ContractDeploymentLifecycle,
        Pure<CheckedAdd>,
        Pure<CheckedAddConfigurationValue>,
        Pure<RequireNonZeroConfiguration>,
    );
}
```

The new failure codec and classifier follow from the State declaration. Cold load now discovers
the new State through that source entry without naming the original composition. A downstream
extension must publish through its owning integration; it cannot add an external trait implementation
for an externally owned resource type. This installation responsibility applies to new semantics,
not to callers rearranging the already installed lifecycle components.

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

Portfolio's collection ordinal/code become a checked projection of the exact original and retained
typed call input (section 9.1). Domains do not depend on Runtime FailureReport. Preserve native Portfolio originals directly. An ordinal conversion
failure becomes a projection error, not a fabricated ConsolidationFailed incident. Product projection
never replaces the original or becomes a prerequisite for ordinary composition.

Version Program, changed Runtime operation/recovery payloads, and reports together. Remove root
contract fields, root-map chains, map-only callbacks, mapped-root Stop payloads and root() accessors.
Retain Never as the exact uninhabited State failure. Preserve source facts, not duplicated-root size
thresholds or claims of unchanged artifact identity. Complete-report overflow retains the known
acknowledged original/recovery head and authority; use small-bound tests, no serializer retries.

### 9.1 Balance originals and Portfolio presentation

Keep domain, native operational and local invocation routes distinct:

```rust
pub enum ObserveBalanceFailure {
    ObservationUnavailable,
    IntegrityBlocked,
}

pub enum BalanceArithmetic { Scale, Sum }

pub enum BalanceCollectionFailure {
    InexactScale {
        raw_units: Unsigned256,
        source_decimals: DecimalScale,
        target_decimals: DecimalScale,
    },
    DecimalCapacityExceeded {
        operation: BalanceArithmetic,
        // Reuse reviewed size facts: measured digits at rejection and the fixed limit 80.
        size: SizeLimitExceeded,
    },
}

pub enum EvmObservationRejection { Rejected, SafeFailure, ChainMismatch }

pub enum EvmBalanceFailure {
    AnchorChanged { previous: EvmBlockAnchor, observed: EvmBlockAnchor },
    ObservationRejected { reason: EvmObservationRejection },
    IntegrityBlocked,
    Collection { source: BalanceCollectionFailure },
}
```

Preserve exact native evidence with the Read call, including external chain-mismatch evidence.
Native failures contain no Portfolio ordinal, duplicated stage strings or presentation codes.
Collection's source is the concrete semantic arithmetic error, retained through source-preserving
serialization/conversion. Invalid scales, missing sources, wrong native contracts and impossible
local context are checked-construction/decoder/invocation errors, not catch-all semantic variants.
Size facts describe the measured rejection; do not claim the final total's size if summation stopped
earlier. Private checked construction/decoding rejects inconsistent error fields.

| Current producer/outcome | Target original | Classification and retained public code |
| --- | --- | --- |
| Initial/confirmation anchor differs | EvmBalanceFailure::AnchorChanged, both anchors retained | InputInvalidated; anchor_changed |
| Chain-check accepted rejection/safe failure or observed wrong chain | Native ObservationRejected | Permanent; chain_identity_unavailable |
| Anchor or token-decimals accepted rejection/safe failure | Native ObservationRejected | Permanent; observation_unavailable |
| Balance Read accepted rejection/safe failure | ObserveBalanceFailure::ObservationUnavailable | Permanent; observation_unavailable |
| Confirmation accepted rejection/safe failure | Native ObservationRejected | Permanent; observation_unavailable |
| Authenticated integrity evidence at native support | Native IntegrityBlocked | Permanent; integrity_blocked |
| Authenticated integrity evidence at designated balance Read | ObserveBalanceFailure::IntegrityBlocked | Permanent; integrity_blocked |
| Legitimate scale/capacity rejection after anchor confirmation | Native Collection wrapping BalanceCollectionFailure | Permanent; observation_unavailable |
| Legitimate consolidation arithmetic rejection | BalanceCollectionFailure directly | Permanent; observation_unavailable |
| Provider/transport failure | Existing exact EvmOperationalError and its source chain | Existing classification, including Retryable for unavailable/timeout/rate limiting |
| Invalid work/context, prepare/reconstruction or completion invariant | Source-preserving invocation diagnostic at actual phase | Internal; no fabricated durable SourceUnavailable |
| Local codec/parser/binding mismatch | Existing invocation diagnostic with owning facts | No false integrity evidence or acknowledged domain failure |

The semantic State cannot declare Never: normalized unsuccessful evidence is a durable State failure,
not a projection error. project_evidence preserves Rejected/SafeFailure/IntegrityBlocked as evidence
outcomes; it does not turn them into provider exceptions. Native confirmation compares anchors before
calling shared semantic amount validation. A mismatch therefore wins over arithmetic rejection as
before. Delete the blanket balance_failure helper instead of preserving its lossy local-error cases.

Application decodes the exact original and current-call input according to the selected State
contract. Production typed accessors expose the BalanceContext<PortfolioContinuation> retained in
native support/prepared/candidate inputs. A Portfolio constructor checks the ordinal against the
retained continuation/request before constructing the public CollectionFailed projection. Ordinal
conversion or context disagreement returns a projection error, never ConsolidationFailed.

Use pure domain/native projections, not a Runtime-dependent trait or new projection registry:

```rust
// Portfolio domain, using the existing reviewed public code contract.
fn collection_failure(
    context: &BalanceContext<PortfolioContinuation>,
    code: &str,
) -> Result<PortfolioSnapshotFailure, PortfolioProjectionError>;

// Native domain; stage comes from the selected exact native State definition.
fn public_balance_failure_code(
    stage: EvmBalanceFailureStage,
    failure: &EvmBalanceFailure,
) -> &'static str;
```

The semantic error types expose equivalent fixed code projections. Reuse the current closed native
stage vocabulary where code compatibility depends on stage; never infer it by parsing arbitrary
identity strings. Application's typed cases use installed State definitions, not a new handwritten
executable-registration list. Projection does not classify, authorize recovery or replace originals.
Reporting needs no provider IO. Preserve genuine Portfolio-owned failure originals directly.

Delete MapEvmBalanceFailure and root-map reporting. A changed anchor remains eligible for the
existing admitted collection restart policy; Runtime authorizes it and appends new history without
erasing the failed candidate or original. Reset changed State/value/report ABI identities together;
no old EvmBalanceWork histories are reinterpreted as the new typed stages.

## 10. Crate placement and dependency perimeter

Use one new inward domain crate, `crates/domains/chain` (`mfm-chain`), containing shared identity,
transaction and balance modules. This replaces the previously planned, unimplemented transactions
crate; it does not add another layer. The transaction module owns contract_lifecycle; balance owns
semantic collection contracts and its 80-digit arithmetic. EVM and Portfolio depend inward on these
contracts; shared State code never imports EVM or Runtime. Values owns reusable Unsigned256 mechanics.

| Location | Target responsibility |
| --- | --- |
| kernel/capabilities | Semantic/native Read/Effect, typed adapter/binding interfaces and EffectAdapterOutcome; no State outcome or classifier dependency |
| kernel/program | Typed construction/planning, environment support contract, immutable document/executable sequence, derived executable discovery, live binding and cold load; no Store/Journal or Runtime dependency |
| domains/chain | Shared ledger/point identities; transaction contracts and lifecycle States; BalanceRead, ObserveBalance<K>, typed balance contexts, semantic arithmetic/errors |
| domains/evm | Native config/artifact contracts, request recipes, supporting States, native translation/projection and native operational originals |
| live/evm and downstream composition | Typed resource environments/binders, native adapters, installed source roots, supported native families/configuration; derive exact executable requirements |
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
| Independent Portfolio checked_collections / root-only validator | Typed Operation planning derives collection demands from committed input; endomorphic vectors preserve typed sequence connections; preserve the specified balance protocols and prove replacements before deleting old checks |
| Caller-selected capability profiles / original-source cold load | Inferred ProgramEnvironment with one installed source publication and derived dependencies |
| EvmBalanceWork and EVM-only cumulative balance context/results | Typed semantic context/prepared/candidate contracts and native prefix stages shared by snapshot/enrichment |
| ReadNativeBalance / ReadTokenBalance meaningful States | ObserveBalance<K> with EvmNativeBalance/EvmTokenBalance capability implementations |
| balance_failure catch-all / MapEvmBalanceFailure | Exact native/semantic originals, source-preserving local diagnostics and checked product projection |
| Unconditional root configuration in every child | Production planning views and Operation-local checked configuration |
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

Prove the balance contracts in sections 2.5/4.3/9.1 with both snapshot and enrichment continuations,
native/token prefixes, cold candidate reconstruction and exact report decoding before compiler
cutover. These contracts are specified; adjust private bounds without changing their ownership.

Use coherent logical commits, merging inseparable cuts:

1. Establish mfm-chain shared identity/transaction/balance contracts, scalar mechanics and native
   implementation interfaces with typed constructors, schemas, and consuming proof. Do not expose a second maintained runtime path.
2. Cut over typed Operation planning/local configuration, homogeneous vectors, inferred environment
   support, typed injection/defaults/resolution, complete Program/document/binding schema,
   adapter interfaces, Runtime callback split, native codecs, cold load and explicit read/resume
   handoff with their consumers together. Move EffectAdapterOutcome inward. Remove mutable
   DSL/wrapper/registration/Runtime assembly paths in this cutover. Include root-map removal here when required for one coherent API/wire.
3. Migrate native balance typed support and Portfolio/enrichment together with original-failure/report
   and product-projection migration if independently coherent;
   otherwise keep it with step 2. Remove the superseded transaction outcome suffix and migrate its
   exact semantics; retain the independently meaningful balance confirmation suffix.
4. Use non-generic Program/ExecutionResult with typed source adjacency proofs and exact checked
   Object result decoding. Consolidate execute/checked results on the existing engine and prove
   native pending waiting,
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

At the implementation site that declares installed public component sources, add a code comment
explaining the accepted cold-discovery limitation and link it to
[known gaps](docs/known-gaps.md#downstream-component-discovery-in-the-dsl-refactor). The comment must
say that publishing a new source makes its executable code available after restart, that consumers
recomposing installed components need no registration changes, and that dependent codecs/handlers/
injected States are derived rather than separately listed. Keep that gap entry current during the
cutover. Do not add unrelated comments to today's superseded registration tables merely to satisfy
this future implementation requirement.

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
| Operation planning | Same maintained child standalone/nested/after addition; local demand and defaults scope restored; cold discovery never invokes Plan; future84 reaches preparation without compile-time State evaluation |
| Installed support | Pure fresh/cold with no handles; recomposition across source roots without list edits; dependent handlers/injection derived; ambiguous code/resources rejected; unselected handles unnecessary |
| Portfolio migration | Snapshot and enrichment use shared typed continuation; preserve count/order/routes, empty-request rejection, occurrence recovery scopes and cumulative limits; reject vector bodies with unequal endpoints |
| Balance injection | Both prefix shapes converge on exact prepared/candidate types; native4/token5 Read boundaries; per-source chain/latest/common anchor and committed-number confirmation; wrong source/route fails locally; cold resume at every stage without configuration |
| Balance semantics | Native configured scale, token0..30, raw256-bit and aggregate80-digit limits, existing dust/exact-remainder behavior; canonical zero-upscale regression; no completed candidate before confirmation |
| Balance originals | Rejected/safe/integrity/chain mismatch matrix; exact native originals; anchor rejection before arithmetic; semantic causes retained; corrected local-invariant route; typed ordinal projection hot/cold without a mapper or fabricated failure |
| Construction/cold | Mandatory executable entries; automatically bound support/codecs/handlers; native adapter reuse across request ABIs; persisted public bindings; missing/mismatched resources rejected before return with no IO; no config/C0/setup fabrication; same exact Program identity after load |
| Checked results | One non-generic Program/ExecutionResult for fresh and cold paths; heterogeneous intermediate contracts retained; exact output decoding succeeds; wrong nominal schema with identical JSON and malformed decoding fail explicitly without changing history or triggering recovery |
| Runtime handoff | Explicit Program for execute/read/resume; wrong input contract or commitment rejected before IO/append; retained ProgramRef mismatch rejected before execution; unchanged current-state validation, no late binding or hidden load hook |
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
`git diff --check`; no production Rust/managed E2E/CI gate is selected. The isolated Rust experiment
and its limits are recorded in section 15. Production-code LOC change is zero.

## 14. Material uncertainties and handoff gates

Operation planning, local configuration, installed-support ownership and homogeneous collection
construction and the balance semantic/native contracts are specified. Remaining uncertainties
require implementation evidence, not another DSL, registry or execution layer. No uncompiled sketch
is a claim that the complete cutover is already proved.

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Environment-owned support permits inferred compile/load | Combined installed-source, native-family and binding bounds are uncompiled | Cold discovery could require source/configuration knowledge or duplicate inventories | Prove Pure and mixed fresh/cold workflows, multiple published roots/handlers, exact conflicts and selected-only binding |
| Typed balance stages preserve both consumers and native custody | Prefix specializations and typed cold report projection are uncompiled | A continuation, original or execution boundary could be lost | Compile native/token sequences with both continuations; cold-resume candidates; assert exact original and ordinal projection with no duplicate evidence carrier |
| Operation-local planning config remains available before execution | Planning views and derived demands have not been demonstrated across nested consumers | Future State computation might be incorrectly simulated or require a second config source | Compile standalone/nested lifecycle and Portfolio; execution-dependent choices must form a subsequent Program |
| Exact generic contracts compose on pinned Rust | Derive/coherence/private traversal bounds are uncompiled | Extra erasure or a second path could be introduced | Compile fixed States, typed requests, two native ABIs, recursive support, defaults and cold inventory across crates |
| Native family traversal integrates with full recursive injection | Section 6.1 replaces external enum introspection with one supported type tuple; production traversal is not implemented | The real recursive bounds could still require duplicated discovery code | Compile distinct native ABIs, resolved supporting leaves and Read/Effect sources through the same tuple traversal |
| Complete Program callbacks preserve Runtime phase ownership | Current registered runners include Runtime driver context and typed encoding | Moving whole runners would move transitions inward or alter original custody | Separate prepare/check/invoke/project/interpret/classify and prove encode-once and acknowledgement ordering |
| Persisted binding table fits exact document/capacity rules | Section 15.1 confirms existing public fields, but the new table/envelope is unimplemented | Cold load could omit a binding or exceed bounds | Measure checked Object/document encoding, deduplication, wrong-binding rejection and small-bound failures |
| Explicit Program loading preserves existing read/resume | The new program_document bootstrap is specified but unimplemented | Admission extraction or current-state checks could be weakened | Test the Runtime-owned read-only extraction, config-free load, mismatched ProgramRef rejection and fresh snapshot semantics |
| Complete prepared requests fit capacity behavior | Requests retain explicit context/artifact/evidence | Some current workloads or report thresholds may overflow | Measure maintained artifacts and small-bound failure cases without dropping facts or authority |
| Shared scalar extraction preserves native behavior | Range/decimal mechanics move inward | Accepted values or schema/serialization could drift | Boundary/overflow/native equivalence tests and explicit versioning for changed contracts |
| Native artifact/ABI binding is exact | Actual identifiers/selectors must come from the maintained artifact definition | Arbitrary bytecode could be treated as supported semantics | Wrong artifact/schema/ledger and malformed-return tests against independent oracle |
| Native adapters implement the required waiting and identities | Current Pending may return immediately; Read currently shares native/semantic identity | Busy looping or wrong evidence binding | Async cancellation/non-spinning proof and distinct native/semantic Read refs hot/cold |
| Report cutover preserves every consumer fact | Current canonical reports omit in-memory call facts and include mapped roots | Exported diagnostics or product fields could be lost | Field/source comparison, exact original cold decoding, descriptor checks, and product projection tests |

Do not claim sketches are compiled or broaden Runtime's historical validation/custody authority.
Unavailable exact implementations fail explicitly. No deadline, wall-clock limit, optional modifier
API, second registry, or native-only engine is an implicit solution to a failed proof.

## 15. Initial construction audit and bounded proof

This section records evidence gathered after adopting the complete executable Program design.
It does not mark the production cutover or the complete signatures above as compiled. No production
Rust or dependency manifest changed during this investigation.

### 15.1 Public binding custody

The existing [transaction binding](crates/domains/evm/src/transaction.rs) contains only these public
facts: authority epoch, chain ID, expected genesis hash, endpoint content reference and sender
address. The anchored Read binding is EvmTransactionRoute (chain instance and endpoint reference).
Full transaction bindings already occur in persisted commands. Moving those exact public facts
into Program closes a custody gap; it does not require copying provider configuration or secrets.

The existing [canonical command fixture](crates/domains/evm/tests/transaction_contract.rs) measures
493 bytes for its transaction binding and 365 bytes for its route, extracted and compactly encoded
with sorted JSON keys. These are fixture measurements, exclude Object envelopes, and are not limits
or measurements of the proposed whole Program. Deduplication and final document capacity still
need actual schema tests.

Existing [live transaction binding](crates/live/evm/src/transaction.rs) checks signer purpose,
sender and authority epoch locally. Existing [endpoint contracts](crates/domains/evm/src/lib.rs)
identify a public named route, not a physical URL or authenticated connection. Resource environments
must explicitly associate provider handles with those public identities. Construction can verify
that association; it cannot certify the provider's remote chain without observation. Retain native
command/route, current authority, chain and evidence checks during execution.

The existing [Keystore signer handle](crates/keystore/src/lib.rs) already supports the required
capture without moving the custody owner across threads. Keep private connection data, signer
queue internals, key slots, secret material and signed-wire custody out of Program's document.

### 15.2 Callback split and acknowledgement boundaries

Current [assembly callbacks](crates/kernel/runtime/src/assembly.rs) delegate to generic engine
runners carrying DriverContext/DriverDisposition. The [engine](crates/kernel/runtime/src/engine.rs)
combines typed invocation with recording and recovery. The replacement must separate these actions:

| Program callback work | Runtime boundary |
| --- | --- |
| Prepare semantic command and check native correspondence | Derive EffectId and acknowledge the exact prepared command before invoking IO |
| Invoke already-bound adapter and encode its exact native result/original once | Retain pending authority or handle the returned evidence/failure |
| Check native evidence and semantic projection | Append accepted native settlement only after this succeeds |
| Reproject retained settlement and interpret the State | Run only after acknowledgement; failure cannot authorize resubmission |
| Decode/classify retained original and ask the handler for a proposal | Classify only after failure acknowledgement, authorize the proposal, then record recovery |

Current pending reconciliation also reproduces S::prepare from retained input and compares it to
the acknowledged command. Preserve this equality check; complete construction does not justify
removing an execution invariant or recreating an acknowledged command from new configuration.

Current encode_failure retains position, exact failure contract, original detail/identity marked
unavailable, the encoding cause and available size facts when first encoding fails. The extracted
Program callback must preserve those facts and the originating operation/stage. A plain encoding
`?` returning only a generic diagnostic is not an equivalent replacement. Do not retry serialization,
classify an unacknowledged original, or transport opaque native originals between owners.

Async adapter wrappers currently offload pure native encoding to immediately awaited blocking work
and catch callback construction/poll panics without returning panic payloads. Section 6.5 specifies
the selected inward helper and error-phase boundary. Section 15.6 records the extracted helper proof;
actual engine integration and acknowledgement ordering still require separate tests.

### 15.3 Temporary cross-crate experiment

A disposable three-crate experiment outside the repository passed six behavioral tests and one
compile-fail doctest in the pinned Nix development shell. It used actual mfm-values, mfm-ids,
AdapterError and MfmValue derive; proposed capability/Program types were deliberately small scratch
surrogates, not the production types in this RFC. The command used was:

```sh
nix develop -c cargo test \
  --manifest-path /tmp/mfm-program-proof-rtewSo/Cargo.toml \
  --target-dir /tmp/mfm-program-proof-rtewSo/target --offline
```

That temporary path is experiment evidence, not a repository verification command or maintained
alternate framework. The six tests exercised configuration-selected mock implementations for one
concrete Deploy, missing/mismatched resources with no IO, native binding reused by two semantic
request types, configuration-free reconstruction of a recorded description, rejection of mismatched
endpoint contracts, and exact nested native failure fields through Object. The callback also checked
distinct semantic/native command references. The compile-fail case rejected a State/capability
pairing with incompatible contracts.

This supports the feasibility of inward ownership, typed resource binding and native-original
custody. It does not prove the complete public API: both mock networks used the same native ABI;
there was no injection, Read, multi-State sequence, EffectId, persisted whole-document roundtrip,
ProgramRef check, capacity, cancellation, recovery or Journal ordering. The mock callback combined
IO/projection solely to test types; production must retain the separate phase boundaries above.
Construction errors in the scratch used simple placeholders and do not prove causal diagnostics.

### 15.4 Remaining construction proof, in order

The single supported-type tuple in section 6.1 replaces the external-enum introspection gap. The
second experiment (section 15.5) proves the cross-crate dispatch mechanism without a duplicated
fresh/cold inventory. Next integrate it with actual recursive injection, resolved supporting leaves,
Read and full nominal contracts. A cold resolved supporting leaf must not acquire a root-config
binding-constructor requirement merely because family dispatch also supports fresh construction.

The original-encoding callback extraction in section 15.6 proves a narrower boundary than the full
engine. Next integrate State prepare/check/invoke/project/interpret callbacks and retain actual
original-append failure, preappend projection rejection, postacknowledgement interpretation failure,
panic, cancellation and ambiguous-acknowledgement tests. Do not replace them with scratch mocks.

Finally implement and test Runtime-owned program_document extraction, full canonical Program load
after config deletion, and explicit read/resume ProgramRef mismatch rejection. Preserve existing
current-state and Store snapshot/head checks. The new bootstrap does not itself qualify a current
RunView. Production document capacity/deduplication and Portfolio migration remain separate gates.

### 15.5 One native-family tuple: consuming-crate proof

A second disposable three-crate experiment passed eight behavioral tests and one compile-fail
doctest in the pinned Nix shell:

```sh
nix develop -c cargo test \
  --manifest-path /tmp/mfm-family-proof-7YlUIT/Cargo.toml \
  --target-dir /tmp/mfm-family-proof-7YlUIT/target --offline
```

Its outward profile declares one `(NativeA, NativeB)` implementation tuple. The inward crate owns
sealed tuple dispatch, shares its Config/Stored selection path, and calls the same checked native
leaf constructor. The former separate compile/load match lists are gone. Root native binding
construction remains separate from the typed leaf constructor, which accepts its already-derived
binding. No external enum implementation, public visitor, registry or Runtime receiver is needed.

Unlike the first experiment, these native implementations have different binding, command and
evidence types. The tests exercise selection for the same concrete State; reuse across a second
semantic request; unavailable unselected resources; selected-resource mismatch without IO; mock
descriptor serialization/deserialization and cold reconstruction without configuration resolution;
wrong endpoints, unknown implementation IDs, duplicate family IDs and forged native schema rejection;
and exact nested original preservation. Callback checks retain distinct semantic/native command
references. The compile-fail case rejects incompatible State/capability types.

The proof uses actual Values/IDs/AdapterError/derive code. Its descriptor compares semantic and
native schema identities, but is not the full production Program wire or nominal ABI. It does not
prove ProgramRef/EffectId, policies, injection traversal, multi-State tuple composition, Read,
capacity, Runtime ordering, recovery or cancellation. Construction errors remain scratch placeholders.
The production Read/Effect binding trait names in section 6.1 specialize the same dispatch mechanism;
they have not all been compiled together. This is evidence for the chosen representation, not a
claim that the complete RFC has been implemented. Temporary files are not a maintained second DSL.

### 15.6 Original encoding and callback error-phase proof

A separate extraction used the actual engine encode_failure helper with its Runtime operation
parameter removed and its final wrapper changed to CallbackFailure::Encode. It retained Values,
IDs, the actual ClassifyError contract, and the workspace-lock Tokio version. Two tests passed:

```sh
nix develop -c cargo test \
  --manifest-path /tmp/mfm-callback-proof-20260917/Cargo.toml --offline
```

They check successful nested original roundtrip with exactly one serialization; serializer failure
and task panic with exactly one attempt, exact known contract/position and unavailable original
identity/detail; classification only on successfully encoded/decoded originals; and adapter-poll
panic conversion with no panic payload in the returned diagnostic. They do not prove process panic
hook behavior or durable acknowledgement ordering. No mock transition engine was added.

The existing production regression also passed in the pinned shell:

```sh
nix develop -c cargo test -p mfm-runtime --test current_state \
  sizes::unrecordable_original_reports_known_slot_and_encoding_cause_without_append -- --exact
```

That [regression](crates/kernel/runtime/tests/current_state/sizes.rs) checks the actual engine's
ReadAdapter/Encode provenance, one serialization, retained known failure context, and unchanged
acknowledged head. Preserve and migrate it during the callback split. Passing the current engine
establishes the baseline; it does not certify an unimplemented replacement. The prototype extraction
covers neither constructor panics nor the full prepare/bind/project/interpret callback matrix.
