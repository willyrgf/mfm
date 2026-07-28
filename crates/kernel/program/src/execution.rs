use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, DigestAlgorithm, SchemaId, SemanticTypeId, StableId};
use mfm_journal::v1::{TerminalEffectEvidence, ValueRef};
use mfm_spec::{exact_content_ref, RetainedValueContract};
use mfm_values::{MfmConfig, MfmValue, StateInput};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{ProgramError, Result};

pub use mfm_facts::{FactProposal, FactSet, ProposedFactValue};

/// Canonical empty configuration for a state with no domain-specific settings.
///
/// Unlike an absent configuration sentinel, this is a real retained `{}` value
/// with an exact schema and producer-bound authority.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    mfm_program_derive::MfmConfig,
)]
#[serde(deny_unknown_fields)]
#[mfm(version = "1", schema = "mfm.program.unit_config")]
pub struct UnitConfig {}

/// Returns the one kernel-owned semantic identity for retained [`UnitConfig`].
pub fn unit_config_semantic_type_id() -> Result<SemanticTypeId> {
    SemanticTypeId::new(
        "mfm.program",
        "unit-config",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"semantic:mfm.program:unit-config:1"),
    )
    .map_err(|error| ProgramError::Spec(error.to_string()))
}

/// Builds the exact retained contract for framework-owned [`UnitConfig`].
pub fn unit_config_value_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract> {
    config_value_contract::<UnitConfig>(
        unit_config_semantic_type_id()?,
        role,
        evidence_contract_ref,
    )
}

/// Configuration value accepted by a state frame.
pub trait StateConfig: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Returns the persisted configuration schema.
    fn config_schema_id() -> Result<SchemaId>;

    /// Validates the configuration before invoking state code.
    fn validate_config(&self) -> Result<()>;
}

impl<T> StateConfig for T
where
    T: MfmConfig,
{
    fn config_schema_id() -> Result<SchemaId> {
        T::schema_id().map_err(|error| ProgramError::Codec(error.to_string()))
    }

    fn validate_config(&self) -> Result<()> {
        self.validate()
            .map_err(|error| ProgramError::Codec(error.to_string()))
    }
}

/// Marker for a state without semantic transition context.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoContext;

/// Context value accepted by a state frame.
pub trait StateContext: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Returns the persisted schema, or `None` for [`NoContext`].
    fn context_schema_id() -> Result<Option<SchemaId>>;
}

impl StateContext for NoContext {
    fn context_schema_id() -> Result<Option<SchemaId>> {
        Ok(None)
    }
}

impl<T> StateContext for T
where
    T: MfmValue,
{
    fn context_schema_id() -> Result<Option<SchemaId>> {
        T::schema_id()
            .map(Some)
            .map_err(|error| ProgramError::Codec(error.to_string()))
    }
}

/// Typed placeholder for callback slots that a pure state never exercises.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoBoundaryValue;

/// Canonicalizes one typed callback-boundary value.
pub fn encode_boundary<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(value).map_err(|error| ProgramError::Codec(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| ProgramError::Codec(error.to_string()))
}

/// Decodes one exact canonical callback-boundary value.
///
/// Re-encoding must reproduce the retained bytes exactly. This rejects serde
/// aliases and defaults that would otherwise change the committed value.
pub fn decode_boundary<T: DeserializeOwned + Serialize>(
    canonical: &PlainCanonicalJsonBytes,
) -> Result<T> {
    let value = serde_json::from_slice::<T>(canonical.as_bytes())
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    if encode_boundary(&value)? != *canonical {
        return Err(ProgramError::Codec(
            "typed decode did not round-trip exact canonical bytes".to_owned(),
        ));
    }
    Ok(value)
}

/// Derives the exact immutable reference for canonical typed bytes.
pub fn boundary_content_ref(
    schema_id: SchemaId,
    canonical: &PlainCanonicalJsonBytes,
) -> Result<ContentRef> {
    exact_content_ref(schema_id, canonical).map_err(Into::into)
}

/// Canonicalizes one MFM value and derives its exact immutable reference.
pub fn encode_mfm_value<T: MfmValue>(value: &T) -> Result<(PlainCanonicalJsonBytes, ContentRef)> {
    let canonical = encode_boundary(value)?;
    let schema_id = T::schema_id().map_err(|error| ProgramError::Codec(error.to_string()))?;
    let content_ref = boundary_content_ref(schema_id, &canonical)?;
    Ok((canonical, content_ref))
}

/// Canonicalizes one planning config and derives its exact immutable reference.
pub fn encode_config<T: MfmConfig>(value: &T) -> Result<(PlainCanonicalJsonBytes, ContentRef)> {
    value
        .validate()
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    let canonical = encode_boundary(value)?;
    let schema_id = T::schema_id().map_err(|error| ProgramError::Codec(error.to_string()))?;
    let content_ref = boundary_content_ref(schema_id, &canonical)?;
    Ok((canonical, content_ref))
}

/// Canonicalizes one typed input and derives its exact immutable reference.
pub fn encode_state_input<T>(value: &T) -> Result<(PlainCanonicalJsonBytes, ContentRef)>
where
    T: StateInput + Serialize,
{
    let canonical = encode_boundary(value)?;
    let schema_id = T::input_schema_id().map_err(|error| ProgramError::Codec(error.to_string()))?;
    let content_ref = boundary_content_ref(schema_id, &canonical)?;
    Ok((canonical, content_ref))
}

/// Builds the exact JSON retained-value contract for a derive-backed MFM
/// value. Package code remains responsible for its reviewed role and evidence
/// contract.
pub fn mfm_value_contract<T: MfmValue>(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract> {
    RetainedValueContract::new(
        T::schema_id().map_err(|error| ProgramError::Codec(error.to_string()))?,
        T::semantic_id().map_err(|error| ProgramError::Codec(error.to_string()))?,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|error| ProgramError::Codec(error.to_string()))
}

/// Builds an exact JSON retained-value contract for planning configuration.
///
/// [`MfmConfig`] deliberately does not invent a semantic identity, so the
/// package supplies its reviewed semantic type.
pub fn config_value_contract<T: MfmConfig>(
    semantic_type_id: SemanticTypeId,
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract> {
    RetainedValueContract::new(
        T::schema_id().map_err(|error| ProgramError::Codec(error.to_string()))?,
        semantic_type_id,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|error| ProgramError::Codec(error.to_string()))
}

/// Builds an exact JSON retained-value contract for an assembled state-input
/// root. The package supplies the semantic identity because [`StateInput`]
/// owns only its structural schema.
pub fn state_input_value_contract<T: StateInput>(
    semantic_type_id: SemanticTypeId,
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract> {
    RetainedValueContract::new(
        T::input_schema_id().map_err(|error| ProgramError::Codec(error.to_string()))?,
        semantic_type_id,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|error| ProgramError::Codec(error.to_string()))
}

/// Canonical encoder for one schema-bound callback value.
pub type BoundaryEncoder<T> = fn(&T) -> Result<PlainCanonicalJsonBytes>;

/// Strict canonical decoder for one schema-bound callback value.
pub type BoundaryDecoder<T> = fn(&[u8]) -> Result<T>;

/// Exact schema and codec used at a read or effect boundary.
///
/// The codec is carried by the closed execution case instead of inferred from
/// serde or a local descriptor. Annex-owned request and observation types can
/// therefore retain their one authoritative schema and strict decoder.
pub struct CanonicalCodec<T> {
    value_contract: RetainedValueContract,
    encode: BoundaryEncoder<T>,
    decode: BoundaryDecoder<T>,
}

impl<T> Clone for CanonicalCodec<T> {
    fn clone(&self) -> Self {
        Self {
            value_contract: self.value_contract.clone(),
            encode: self.encode,
            decode: self.decode,
        }
    }
}

impl<T> CanonicalCodec<T> {
    /// Binds one exact schema to its canonical encoder and strict decoder.
    pub const fn new(
        value_contract: RetainedValueContract,
        encode: BoundaryEncoder<T>,
        decode: BoundaryDecoder<T>,
    ) -> Self {
        Self {
            value_contract,
            encode,
            decode,
        }
    }

    /// Builds a codec for a derive-backed MFM value.
    pub fn mfm_value(value_contract: RetainedValueContract) -> Result<Self>
    where
        T: MfmValue,
    {
        let schema_id = T::schema_id().map_err(|error| ProgramError::Codec(error.to_string()))?;
        if value_contract.schema_id() != &schema_id
            || value_contract.semantic_type_id()
                != &T::semantic_id().map_err(|error| ProgramError::Codec(error.to_string()))?
        {
            return Err(ProgramError::Codec(
                "MFM value codec differs from its retained-value contract".to_owned(),
            ));
        }
        Ok(Self::new(
            value_contract,
            encode_boundary::<T>,
            decode_boundary_bytes::<T>,
        ))
    }

    /// Returns the complete retained-value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }

    /// Returns the exact schema id.
    pub const fn schema_id(&self) -> &SchemaId {
        self.value_contract.schema_id()
    }

    /// Canonicalizes one typed value and derives its exact reference.
    pub fn encode(&self, value: &T) -> Result<(PlainCanonicalJsonBytes, ContentRef)> {
        let canonical = (self.encode)(value)?;
        let reparsed = PlainCanonicalJsonBytes::from_canonical_json_slice(canonical.as_bytes())
            .map_err(|error| ProgramError::Codec(error.to_string()))?;
        let content_ref = boundary_content_ref(self.value_contract.schema_id().clone(), &reparsed)?;
        Ok((reparsed, content_ref))
    }

    /// Strictly decodes exact canonical bytes and rejects non-round-tripping
    /// representations.
    pub fn decode(&self, bytes: &[u8]) -> Result<T> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|error| ProgramError::Codec(error.to_string()))?;
        let value = (self.decode)(canonical.as_bytes())?;
        let reencoded = (self.encode)(&value)?;
        if reencoded != canonical {
            return Err(ProgramError::Codec(
                "boundary codec did not round-trip exact canonical bytes".to_owned(),
            ));
        }
        Ok(value)
    }
}

impl CanonicalCodec<mfm_facts::FactSelectionRequest> {
    /// Returns the frozen fact-selection request codec.
    pub fn fact_selection_request(value_contract: RetainedValueContract) -> Result<Self> {
        ensure_schema(&value_contract, "mfm.fact-selection-request.v1")?;
        Ok(Self::new(
            value_contract,
            encode_fact_selection_request,
            decode_fact_selection_request,
        ))
    }
}

impl CanonicalCodec<mfm_journal::v1::FactSelectionResponse> {
    /// Returns the frozen fact-selection response codec.
    pub fn fact_selection_response(value_contract: RetainedValueContract) -> Result<Self> {
        ensure_schema(&value_contract, "mfm.fact-selection-response.v1")?;
        Ok(Self::new(
            value_contract,
            encode_fact_selection_response,
            decode_fact_selection_response,
        ))
    }
}

impl CanonicalCodec<mfm_journal::v1::SafeFailure> {
    /// Returns the frozen redaction-safe access-failure codec.
    pub fn safe_failure(value_contract: RetainedValueContract) -> Result<Self> {
        ensure_schema(&value_contract, "mfm.safe-failure.v1")?;
        Ok(Self::new(
            value_contract,
            encode_safe_failure,
            decode_safe_failure,
        ))
    }
}

fn ensure_schema(value_contract: &RetainedValueContract, schema_contract: &str) -> Result<()> {
    if value_contract.schema_id() != &mfm_spec::schema_id(schema_contract)? {
        return Err(ProgramError::Codec(
            "boundary codec schema differs from retained-value contract".to_owned(),
        ));
    }
    Ok(())
}

fn encode_fact_selection_request(
    value: &mfm_facts::FactSelectionRequest,
) -> Result<PlainCanonicalJsonBytes> {
    exact_plain_bytes(value.canonical_json())
}

fn decode_fact_selection_request(bytes: &[u8]) -> Result<mfm_facts::FactSelectionRequest> {
    mfm_facts::FactSelectionRequest::from_canonical_json(bytes)
        .map_err(|error| ProgramError::Codec(error.to_string()))
}

fn encode_fact_selection_response(
    value: &mfm_journal::v1::FactSelectionResponse,
) -> Result<PlainCanonicalJsonBytes> {
    exact_plain_bytes(value.as_bytes())
}

fn decode_fact_selection_response(bytes: &[u8]) -> Result<mfm_journal::v1::FactSelectionResponse> {
    mfm_journal::v1::FactSelectionResponse::strict_decode(bytes)
        .map_err(|error| ProgramError::Codec(error.to_string()))
}

fn encode_safe_failure(value: &mfm_journal::v1::SafeFailure) -> Result<PlainCanonicalJsonBytes> {
    exact_plain_bytes(value.as_bytes())
}

fn decode_safe_failure(bytes: &[u8]) -> Result<mfm_journal::v1::SafeFailure> {
    mfm_journal::v1::SafeFailure::strict_decode(bytes)
        .map_err(|error| ProgramError::Codec(error.to_string()))
}

fn exact_plain_bytes(bytes: &[u8]) -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| ProgramError::Codec(error.to_string()))
}

fn decode_boundary_bytes<T>(bytes: &[u8]) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    decode_boundary(&canonical)
}

/// Exact external operation selected by a read capability or executor contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityOperation {
    operation_contract_ref: ContentRef,
    operation_id: StableId,
    binding_ref: ContentRef,
}

impl CapabilityOperation {
    /// Constructs an exact external operation selection.
    pub fn new(
        operation_contract_ref: ContentRef,
        operation_id: StableId,
        binding_ref: ContentRef,
    ) -> Self {
        Self {
            operation_contract_ref,
            operation_id,
            binding_ref,
        }
    }

    /// Returns the capability or executor semantic operation contract.
    pub fn operation_contract_ref(&self) -> &ContentRef {
        &self.operation_contract_ref
    }

    /// Returns the operation selected within that contract.
    pub fn operation_id(&self) -> &StableId {
        &self.operation_id
    }

    /// Returns the exact qualified adapter or executor binding.
    pub fn binding_ref(&self) -> &ContentRef {
        &self.binding_ref
    }
}

/// Executor operation selected by an effect state.
pub type ExecutorOperation = CapabilityOperation;

/// Closed state contract admitted by certification.
pub trait State: Send + Sync + Sized + 'static {
    /// Canonical state configuration.
    type Config: StateConfig;
    /// Optional typed transition context.
    type Context: StateContext;
    /// Typed input tree.
    type Input: StateInput + Serialize + DeserializeOwned;
    /// Typed successful output.
    type Output: MfmValue;
    /// Typed semantic domain failure.
    type Failure: MfmValue;
    /// Typed read or effect request. Pure states use [`NoBoundaryValue`].
    type Request: Send + Sync + 'static;
    /// Typed successful response or terminal evidence.
    ///
    /// Its codec belongs to the selected execution case, so annex-owned
    /// observation types do not need a second local descriptor.
    type Observation: Send + Sync + 'static;
    /// Reviewed redaction-safe access failure value.
    ///
    /// Pure states use [`NoBoundaryValue`].
    type AccessFailure: Send + Sync + 'static;

    /// Returns the exact semantic state contract.
    fn state_contract_ref() -> Result<ContentRef>;
}

/// Borrowed typed value and exact immutable content reference.
#[derive(Debug)]
pub struct ValueView<'a, T> {
    value: &'a T,
    value_ref: &'a ValueRef,
}

impl<T> Copy for ValueView<'_, T> {}

impl<T> Clone for ValueView<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T> ValueView<'a, T> {
    /// Borrows one exact typed value.
    pub const fn new(value: &'a T, value_ref: &'a ValueRef) -> Self {
        Self { value, value_ref }
    }

    /// Returns the typed value.
    pub const fn value(self) -> &'a T {
        self.value
    }

    /// Returns its exact immutable reference.
    pub const fn value_ref(self) -> &'a ValueRef {
        self.value_ref
    }
}

/// Value-only verified input frame supplied to one state callback.
#[derive(Debug)]
pub struct StateFrame<'a, S: State> {
    config: ValueView<'a, S::Config>,
    context: Option<ValueView<'a, S::Context>>,
    input: ValueView<'a, S::Input>,
}

impl<'a, S: State> Copy for StateFrame<'a, S> {}

impl<'a, S: State> Clone for StateFrame<'a, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, S: State> StateFrame<'a, S> {
    /// Constructs a value-only frame after runtime has verified its private proof.
    ///
    /// This constructor grants no append or access authority.
    pub const fn from_verified_values(
        config: ValueView<'a, S::Config>,
        context: Option<ValueView<'a, S::Context>>,
        input: ValueView<'a, S::Input>,
    ) -> Self {
        Self {
            config,
            context,
            input,
        }
    }

    /// Returns exact configuration.
    pub const fn config(self) -> ValueView<'a, S::Config> {
        self.config
    }

    /// Returns exact optional context.
    pub const fn context(self) -> Option<ValueView<'a, S::Context>> {
        self.context
    }

    /// Returns exact input.
    pub const fn input(self) -> ValueView<'a, S::Input> {
        self.input
    }
}

/// Closed result of one audited capability boundary operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationOutcome<R, F> {
    /// One schema-valid typed value returned.
    Returned(R),
    /// The wrapper proved that the boundary was not entered.
    DidNotEnter(F),
    /// Entry or outcome remains indeterminate.
    Indeterminate(F),
}

/// Borrowed value-only view of an exact committed request.
#[derive(Debug)]
pub struct RequestView<'a, T> {
    value: &'a T,
    value_ref: &'a ValueRef,
}

impl<T> Copy for RequestView<'_, T> {}

impl<T> Clone for RequestView<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T> RequestView<'a, T> {
    /// Borrows a request value without runtime authority.
    pub const fn new(value: &'a T, value_ref: &'a ValueRef) -> Self {
        Self { value, value_ref }
    }

    /// Returns the typed request.
    pub const fn value(self) -> &'a T {
        self.value
    }

    /// Returns the exact immutable request reference.
    pub const fn value_ref(self) -> &'a ValueRef {
        self.value_ref
    }
}

/// Borrowed value-only view of an exact committed observation.
#[derive(Debug)]
pub struct ObservationView<'a, R, F> {
    outcome: &'a ObservationOutcome<R, F>,
    value_ref: &'a ValueRef,
}

impl<R, F> Copy for ObservationView<'_, R, F> {}

impl<R, F> Clone for ObservationView<'_, R, F> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, R, F> ObservationView<'a, R, F> {
    /// Borrows an observation without runtime authority.
    pub const fn new(outcome: &'a ObservationOutcome<R, F>, value_ref: &'a ValueRef) -> Self {
        Self { outcome, value_ref }
    }

    /// Returns the typed audited outcome.
    pub const fn outcome(self) -> &'a ObservationOutcome<R, F> {
        self.outcome
    }

    /// Returns the exact immutable observation reference.
    pub const fn value_ref(self) -> &'a ValueRef {
        self.value_ref
    }
}

/// Callback-free resolver for objects already admitted into verified terminal
/// evidence closure.
///
/// Implementations must reject every reference outside the exact
/// descriptor-declared closure. Resolution is synchronous and performs no
/// ambient or live IO.
pub trait TerminalEffectResolver: Send + Sync {
    /// Resolves exact canonical bytes for one closure member.
    fn resolve(&self, value_ref: &ValueRef) -> Result<PlainCanonicalJsonBytes>;
}

/// Borrowed verified terminal executor evidence supplied to an effect state.
///
/// Pending ensure results never construct this view and never reach state
/// settlement.
pub struct VerifiedTerminalEffectView<'a> {
    evidence: &'a TerminalEffectEvidence,
    evidence_ref: &'a ValueRef,
    resolver: &'a dyn TerminalEffectResolver,
}

impl<'a> VerifiedTerminalEffectView<'a> {
    /// Borrows one structurally verified committed terminal observation.
    pub const fn new(
        evidence: &'a TerminalEffectEvidence,
        evidence_ref: &'a ValueRef,
        resolver: &'a dyn TerminalEffectResolver,
    ) -> Self {
        Self {
            evidence,
            evidence_ref,
            resolver,
        }
    }

    /// Returns the complete journal-owned terminal evidence.
    pub const fn evidence(&self) -> &'a TerminalEffectEvidence {
        self.evidence
    }

    /// Returns the exact retained terminal-evidence reference.
    pub const fn evidence_ref(&self) -> &'a ValueRef {
        self.evidence_ref
    }

    /// Resolves one member of the already verified retained closure.
    pub fn resolve(&self, value_ref: &ValueRef) -> Result<PlainCanonicalJsonBytes> {
        self.resolver.resolve(value_ref)
    }
}

/// Typed semantic settlement proposed by a state callback.
pub enum Settlement<S: State> {
    /// Successful output and fact emissions.
    Succeeded {
        /// Typed output.
        output: S::Output,
        /// Same-run fact emissions.
        facts: FactSet,
    },
    /// Typed domain truth describing semantic failure.
    Failed {
        /// Typed domain failure.
        failure: S::Failure,
    },
}

impl<S: State> Settlement<S> {
    /// Constructs successful settlement.
    pub fn succeeded(output: S::Output, facts: FactSet) -> Self {
        Self::Succeeded { output, facts }
    }

    /// Constructs typed domain failure.
    pub fn failed(failure: S::Failure) -> Self {
        Self::Failed { failure }
    }
}

/// Pure callback verdict over committed evidence.
pub enum EvidenceVerdict<T> {
    /// Evidence is sufficient and yields a settlement.
    Settlement(T),
    /// Evidence is valid but not yet sufficient.
    InsufficientEvidence,
    /// Evidence violates the exact state evidence contract.
    InvalidEvidence,
}

/// Closed execution case selected by a state contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateExecutionKind {
    /// Pure local work.
    Pure,
    /// One audited external read request.
    Read,
    /// One recoverable keyed effect request.
    Effect,
}

/// Pure apply callback.
pub type PureApply<S> = for<'a> fn(StateFrame<'a, S>) -> Settlement<S>;
/// Total read/effect request author callback.
pub type RequestAuthor<S> = for<'a> fn(StateFrame<'a, S>) -> <S as State>::Request;
/// Read observation reduction callback.
pub type ReadApply<S> = for<'a> fn(
    StateFrame<'a, S>,
    ObservationView<'a, <S as State>::Observation, <S as State>::AccessFailure>,
) -> EvidenceVerdict<Settlement<S>>;
/// Effect terminal-evidence settlement callback.
pub type EffectSettle<S> =
    for<'a> fn(StateFrame<'a, S>, VerifiedTerminalEffectView<'a>) -> EvidenceVerdict<Settlement<S>>;

/// Pure execution callbacks.
pub struct PureExecution<S: State> {
    apply: PureApply<S>,
}

impl<S: State> PureExecution<S> {
    /// Returns the pure callback.
    pub const fn apply(&self) -> PureApply<S> {
        self.apply
    }
}

/// Read execution callbacks and exact capability operation.
pub struct ReadExecution<S: State> {
    operation: CapabilityOperation,
    request_codec: CanonicalCodec<S::Request>,
    observation_codec: CanonicalCodec<S::Observation>,
    access_failure_codec: CanonicalCodec<S::AccessFailure>,
    request: RequestAuthor<S>,
    apply: ReadApply<S>,
}

impl<S: State> ReadExecution<S> {
    /// Returns the exact read operation.
    pub const fn operation(&self) -> &CapabilityOperation {
        &self.operation
    }

    /// Returns the exact request codec.
    pub const fn request_codec(&self) -> &CanonicalCodec<S::Request> {
        &self.request_codec
    }

    /// Returns the exact successful-observation codec.
    pub const fn observation_codec(&self) -> &CanonicalCodec<S::Observation> {
        &self.observation_codec
    }

    /// Returns the exact safe-access-failure codec.
    pub const fn access_failure_codec(&self) -> &CanonicalCodec<S::AccessFailure> {
        &self.access_failure_codec
    }

    /// Returns the total request author.
    pub const fn request(&self) -> RequestAuthor<S> {
        self.request
    }

    /// Returns the observation reducer.
    pub const fn apply(&self) -> ReadApply<S> {
        self.apply
    }
}

/// Effect execution callbacks and exact executor operation.
pub struct EffectExecution<S: State> {
    operation: ExecutorOperation,
    request_codec: CanonicalCodec<S::Request>,
    ensure_result_contract: RetainedValueContract,
    terminal_evidence_contract: RetainedValueContract,
    domain_result_contract: RetainedValueContract,
    request: RequestAuthor<S>,
    settle: EffectSettle<S>,
}

impl<S: State> EffectExecution<S> {
    /// Returns the exact executor operation.
    pub const fn operation(&self) -> &ExecutorOperation {
        &self.operation
    }

    /// Returns the exact request codec.
    pub const fn request_codec(&self) -> &CanonicalCodec<S::Request> {
        &self.request_codec
    }

    /// Returns the fixed verified ensure-result contract.
    pub const fn ensure_result_contract(&self) -> &RetainedValueContract {
        &self.ensure_result_contract
    }

    /// Returns the structurally verified terminal-evidence contract.
    pub const fn terminal_evidence_contract(&self) -> &RetainedValueContract {
        &self.terminal_evidence_contract
    }

    /// Returns the state-owned domain converter result contract.
    pub const fn domain_result_contract(&self) -> &RetainedValueContract {
        &self.domain_result_contract
    }

    /// Returns the total request author.
    pub const fn request(&self) -> RequestAuthor<S> {
        self.request
    }

    /// Returns the terminal evidence verifier.
    pub const fn settle(&self) -> EffectSettle<S> {
        self.settle
    }
}

/// One closed state execution case.
#[allow(clippy::large_enum_variant)]
pub enum StateExecution<S: State> {
    /// Pure local execution.
    Pure(PureExecution<S>),
    /// Audited external read.
    Read(ReadExecution<S>),
    /// Recoverable keyed effect.
    Effect(EffectExecution<S>),
}

impl<S: State> StateExecution<S> {
    /// Constructs pure execution.
    pub const fn pure(apply: PureApply<S>) -> Self {
        Self::Pure(PureExecution { apply })
    }

    /// Constructs read execution with one total request and one observation reducer.
    pub fn read(
        operation: CapabilityOperation,
        request_codec: CanonicalCodec<S::Request>,
        observation_codec: CanonicalCodec<S::Observation>,
        access_failure_codec: CanonicalCodec<S::AccessFailure>,
        request: RequestAuthor<S>,
        apply: ReadApply<S>,
    ) -> Self {
        Self::Read(ReadExecution {
            operation,
            request_codec,
            observation_codec,
            access_failure_codec,
            request,
            apply,
        })
    }

    /// Constructs recoverable effect execution.
    pub fn effect(
        operation: ExecutorOperation,
        request_codec: CanonicalCodec<S::Request>,
        ensure_result_contract: RetainedValueContract,
        terminal_evidence_contract: RetainedValueContract,
        domain_result_contract: RetainedValueContract,
        request: RequestAuthor<S>,
        settle: EffectSettle<S>,
    ) -> Self {
        Self::Effect(EffectExecution {
            operation,
            request_codec,
            ensure_result_contract,
            terminal_evidence_contract,
            domain_result_contract,
            request,
            settle,
        })
    }

    /// Returns the closed execution kind.
    pub const fn kind(&self) -> StateExecutionKind {
        match self {
            Self::Pure(_) => StateExecutionKind::Pure,
            Self::Read(_) => StateExecutionKind::Read,
            Self::Effect(_) => StateExecutionKind::Effect,
        }
    }
}
