use std::any::{Any, TypeId};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentRef, FieldPath};
use mfm_journal::v2::ValueRef;
use mfm_spec::RetainedValueContract;
use mfm_values::StateInput;

use crate::{
    boundary_content_ref, decode_boundary, EvidenceVerdict, FactSet, ObservationOutcome,
    ObservationView, ProgramError, QualifiedStateDefinition, Result, Settlement, State,
    StateConfig, StateContext, StateExecution, StateExecutionKind, StateFrame, ValueView,
    VerifiedTerminalEffectView,
};

/// Exact canonical bytes and full producer-bound reference already verified by
/// the store boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedValueMaterial {
    canonical: PlainCanonicalJsonBytes,
    value_ref: ValueRef,
}

impl VerifiedValueMaterial {
    /// Constructs value-only material after structural verification.
    pub const fn new(canonical: PlainCanonicalJsonBytes, value_ref: ValueRef) -> Self {
        Self {
            canonical,
            value_ref,
        }
    }

    /// Returns the exact canonical bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    /// Returns the exact producer-bound reference.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }
}

/// Checked callback frame material assembled from one verified input manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedStateFrameMaterial {
    config: VerifiedValueMaterial,
    context: Option<VerifiedValueMaterial>,
    input: VerifiedValueMaterial,
}

impl VerifiedStateFrameMaterial {
    /// Constructs exact value-only frame material.
    pub const fn new(
        config: VerifiedValueMaterial,
        context: Option<VerifiedValueMaterial>,
        input: VerifiedValueMaterial,
    ) -> Self {
        Self {
            config,
            context,
            input,
        }
    }

    /// Returns the required verified configuration.
    pub const fn config(&self) -> &VerifiedValueMaterial {
        &self.config
    }

    /// Returns optional verified context.
    pub const fn context(&self) -> Option<&VerifiedValueMaterial> {
        self.context.as_ref()
    }

    /// Returns the store-assembled input root.
    pub const fn input(&self) -> &VerifiedValueMaterial {
        &self.input
    }
}

/// Exact canonical bytes proposed by a state callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedValueMaterial {
    canonical: PlainCanonicalJsonBytes,
    content_ref: ContentRef,
}

impl ProposedValueMaterial {
    /// Returns proposed canonical bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    /// Returns their exact raw-byte content reference.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }
}

/// One typed authored request retained for an immediate qualified call.
pub struct QualifiedAuthoredRequest {
    request_type: TypeId,
    value: Box<dyn Any + Send + Sync>,
    proposed: ProposedValueMaterial,
}

impl QualifiedAuthoredRequest {
    /// Returns the concrete request type identity.
    pub const fn request_type(&self) -> TypeId {
        self.request_type
    }

    /// Returns the exact canonical request proposal.
    pub const fn proposed(&self) -> &ProposedValueMaterial {
        &self.proposed
    }

    /// Consumes the request into its concrete type-erased value.
    pub fn into_value(self) -> Box<dyn Any + Send + Sync> {
        self.value
    }
}

/// Exact committed read outcome reconstructed by the structural verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifiedReadOutcome {
    /// One schema-valid returned value.
    Returned(VerifiedValueMaterial),
    /// The adapter proved the capability boundary was not entered.
    DidNotEnter {
        /// Generic classifier-approved safe-failure metadata.
        metadata: mfm_store::SafeFailureMetadata,
        /// Optional exact typed diagnostic.
        diagnostic: Option<VerifiedValueMaterial>,
    },
    /// Entry or outcome remains indeterminate.
    Indeterminate {
        /// Generic classifier-approved safe-failure metadata.
        metadata: mfm_store::SafeFailureMetadata,
        /// Optional exact typed diagnostic.
        diagnostic: Option<VerifiedValueMaterial>,
    },
}

/// One exact output slot proposed by a successful callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedOutputSlot {
    output_ordinal: u32,
    field_path: FieldPath,
    value_contract: RetainedValueContract,
    value: ProposedValueMaterial,
}

impl ProposedOutputSlot {
    /// Returns the certified output ordinal.
    pub const fn output_ordinal(&self) -> u32 {
        self.output_ordinal
    }

    /// Returns the certified output field path.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns the exact retained-value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }

    /// Returns the exact projected canonical value.
    pub const fn value(&self) -> &ProposedValueMaterial {
        &self.value
    }
}

type ErasedOutputProjector<S> =
    dyn Fn(&<S as State>::Output) -> Result<ProposedValueMaterial> + Send + Sync;

/// One exact typed projector for a certified successful output slot.
pub struct QualifiedOutputProjector<S: State> {
    output_ordinal: u32,
    field_path: FieldPath,
    value_contract: RetainedValueContract,
    project: Arc<ErasedOutputProjector<S>>,
}

impl<S: State> QualifiedOutputProjector<S> {
    /// Binds one certified slot to a typed field projector and exact codec.
    pub fn new<T>(
        output_ordinal: u32,
        field_path: FieldPath,
        codec: crate::CanonicalCodec<T>,
        project: for<'a> fn(&'a S::Output) -> &'a T,
    ) -> Self
    where
        T: Send + Sync + 'static,
    {
        let value_contract = codec.value_contract().clone();
        Self {
            output_ordinal,
            field_path,
            value_contract,
            project: Arc::new(move |output| {
                let (canonical, content_ref) = codec.encode(project(output))?;
                Ok(ProposedValueMaterial {
                    canonical,
                    content_ref,
                })
            }),
        }
    }

    fn project(&self, output: &S::Output) -> Result<ProposedOutputSlot> {
        Ok(ProposedOutputSlot {
            output_ordinal: self.output_ordinal,
            field_path: self.field_path.clone(),
            value_contract: self.value_contract.clone(),
            value: (self.project)(output)?,
        })
    }
}

/// Exact successful-output and typed-failure codecs for one state callback.
pub struct QualifiedSettlementCodecs<S: State> {
    output_projectors: Vec<QualifiedOutputProjector<S>>,
    failure_codec: Option<crate::CanonicalCodec<S::Failure>>,
}

impl<S: State> QualifiedSettlementCodecs<S> {
    /// Constructs one closed settlement codec set.
    pub fn new(
        mut output_projectors: Vec<QualifiedOutputProjector<S>>,
        failure_codec: Option<crate::CanonicalCodec<S::Failure>>,
    ) -> Result<Self> {
        output_projectors.sort_by_key(|projector| projector.output_ordinal);
        if output_projectors
            .windows(2)
            .any(|pair| pair[0].output_ordinal == pair[1].output_ordinal)
        {
            return Err(ProgramError::Registry(
                "qualified output projector ordinals must be unique".to_owned(),
            ));
        }
        Ok(Self {
            output_projectors,
            failure_codec,
        })
    }

    pub(crate) fn validate_against(
        &self,
        contract: &mfm_spec::CertifiedSettlementContract,
    ) -> Result<()> {
        if self.output_projectors.len() != contract.output_slots().len()
            || self
                .output_projectors
                .iter()
                .zip(contract.output_slots())
                .any(|(projector, slot)| {
                    projector.output_ordinal != slot.output_ordinal()
                        || projector.field_path != *slot.field_path()
                        || projector.value_contract != *slot.value_contract()
                })
            || self
                .failure_codec
                .as_ref()
                .map(crate::CanonicalCodec::value_contract)
                != contract.typed_failure_contract()
        {
            return Err(ProgramError::Registry(
                "settlement codecs differ from the complete qualified contract".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Type-erased settlement candidate returned by qualified state code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedSettlement {
    /// Successful exact output slots and fact proposals.
    Succeeded {
        /// Per-slot output material in certified ordinal order.
        output_slots: Vec<ProposedOutputSlot>,
        /// Same-run fact proposals in callback order.
        facts: FactSet,
    },
    /// One exact typed domain failure.
    Failed {
        /// Canonical failure material.
        failure: ProposedValueMaterial,
    },
}

/// Type-erased evidence verdict returned by qualified state code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedEvidenceVerdict {
    /// Evidence is sufficient for semantic settlement.
    Settlement(QualifiedSettlement),
    /// Evidence is valid but not yet sufficient.
    InsufficientEvidence,
    /// Evidence violates the certified state evidence contract.
    InvalidEvidence,
}

/// Object-safe callback surface carried by one qualified state registration.
pub trait QualifiedStateCallbacks: Send + Sync {
    /// Returns the closed execution kind.
    fn kind(&self) -> StateExecutionKind;

    /// Returns the selected external operation, if any.
    fn operation(&self) -> Option<&crate::CapabilityOperation>;

    /// Returns the concrete request type identity.
    fn request_type(&self) -> TypeId;

    /// Returns the concrete observation type identity.
    fn observation_type(&self) -> TypeId;

    /// Returns the concrete access-failure type identity.
    fn diagnostic_type(&self) -> TypeId;

    /// Authors one immutable read/effect request.
    fn author_request(
        &self,
        frame: &VerifiedStateFrameMaterial,
    ) -> Result<QualifiedAuthoredRequest>;

    /// Strictly reconstructs one already committed request.
    fn decode_request(&self, retained: &VerifiedValueMaterial) -> Result<QualifiedAuthoredRequest>;

    /// Canonicalizes one concrete returned read value.
    fn encode_read_returned(
        &self,
        value: &(dyn Any + Send + Sync),
    ) -> Result<ProposedValueMaterial>;

    /// Canonicalizes one concrete redaction-safe read failure.
    fn encode_read_diagnostic(
        &self,
        value: &(dyn Any + Send + Sync),
    ) -> Result<ProposedValueMaterial>;

    /// Invokes one pure state callback.
    fn settle_pure(&self, frame: &VerifiedStateFrameMaterial) -> Result<QualifiedSettlement>;

    /// Invokes one read observation reducer.
    fn settle_read(
        &self,
        frame: &VerifiedStateFrameMaterial,
        observation: &VerifiedReadOutcome,
    ) -> Result<QualifiedEvidenceVerdict>;

    /// Invokes one effect reducer only after terminal evidence verification.
    fn settle_effect(
        &self,
        frame: &VerifiedStateFrameMaterial,
        terminal: VerifiedTerminalEffectView<'_>,
    ) -> Result<QualifiedEvidenceVerdict>;
}

pub(crate) struct TypedQualifiedStateCallbacks<S: State> {
    execution: StateExecution<S>,
    definition: Arc<QualifiedStateDefinition>,
    settlement_codecs: QualifiedSettlementCodecs<S>,
}

pub(crate) enum CandidateStateCallbackError {
    Integrity(ProgramError),
    ExecutionPanic,
}

impl CandidateStateCallbackError {
    fn into_program_error(self) -> ProgramError {
        match self {
            Self::Integrity(error) => error,
            Self::ExecutionPanic => {
                ProgramError::Codec("qualified state callback panicked".to_owned())
            }
        }
    }
}

impl<S: State> TypedQualifiedStateCallbacks<S> {
    pub(crate) fn new(
        execution: StateExecution<S>,
        definition: Arc<QualifiedStateDefinition>,
        settlement_codecs: QualifiedSettlementCodecs<S>,
    ) -> Self {
        Self {
            execution,
            definition,
            settlement_codecs,
        }
    }

    fn with_candidate_frame<T>(
        &self,
        material: &VerifiedStateFrameMaterial,
        callback: impl FnOnce(StateFrame<'_, S>) -> T,
    ) -> std::result::Result<T, CandidateStateCallbackError> {
        let config_schema =
            S::Config::config_schema_id().map_err(CandidateStateCallbackError::Integrity)?;
        let config_contract = self.definition.contract().config_contract();
        if config_contract.schema_id() != &config_schema {
            return Err(CandidateStateCallbackError::Integrity(ProgramError::Codec(
                "verified config differs from qualified state contract".to_owned(),
            )));
        }
        let config: S::Config = decode_boundary(material.config().canonical())
            .map_err(CandidateStateCallbackError::Integrity)?;
        config
            .validate_config()
            .map_err(CandidateStateCallbackError::Integrity)?;
        verify_material(material.config(), config_contract)
            .map_err(CandidateStateCallbackError::Integrity)?;
        let context = match (
            material.context(),
            S::Context::context_schema_id().map_err(CandidateStateCallbackError::Integrity)?,
            self.definition.contract().context_contract(),
        ) {
            (None, None, None) => None,
            (Some(material), Some(schema_id), Some(contract))
                if contract.schema_id() == &schema_id =>
            {
                let value: S::Context = decode_boundary(material.canonical())
                    .map_err(CandidateStateCallbackError::Integrity)?;
                verify_material(material, contract)
                    .map_err(CandidateStateCallbackError::Integrity)?;
                Some(value)
            }
            _ => {
                return Err(CandidateStateCallbackError::Integrity(ProgramError::Codec(
                    "verified context differs from qualified state contract".to_owned(),
                )));
            }
        };
        let input: S::Input = decode_boundary(material.input().canonical())
            .map_err(CandidateStateCallbackError::Integrity)?;
        let input_schema = S::Input::input_schema_id()
            .map_err(|error| ProgramError::Codec(error.to_string()))
            .map_err(CandidateStateCallbackError::Integrity)?;
        if self.definition.contract().input_contract().schema_id() != &input_schema {
            return Err(CandidateStateCallbackError::Integrity(ProgramError::Codec(
                "typed input differs from qualified input-root schema".to_owned(),
            )));
        }
        verify_material(
            material.input(),
            self.definition.contract().input_contract(),
        )
        .map_err(CandidateStateCallbackError::Integrity)?;

        let context_view = context
            .as_ref()
            .zip(material.context())
            .map(|(value, material)| ValueView::new(value, material.value_ref()));
        let frame = StateFrame::from_verified_values(
            ValueView::new(&config, material.config().value_ref()),
            context_view,
            ValueView::new(&input, material.input().value_ref()),
        );
        catch_unwind(AssertUnwindSafe(|| callback(frame)))
            .map_err(|_| CandidateStateCallbackError::ExecutionPanic)
    }

    fn encode_request(
        &self,
        request: S::Request,
        codec: &crate::CanonicalCodec<S::Request>,
    ) -> Result<QualifiedAuthoredRequest> {
        let (canonical, content_ref) = codec.encode(&request)?;
        Ok(QualifiedAuthoredRequest {
            request_type: TypeId::of::<S::Request>(),
            value: Box::new(request),
            proposed: ProposedValueMaterial {
                canonical,
                content_ref,
            },
        })
    }

    fn decode_request_with(
        &self,
        retained: &VerifiedValueMaterial,
        codec: &crate::CanonicalCodec<S::Request>,
    ) -> Result<QualifiedAuthoredRequest> {
        verify_material(retained, codec.value_contract())?;
        let request = codec.decode(retained.canonical().as_bytes())?;
        let (canonical, content_ref) = codec.encode(&request)?;
        verify_exact_bytes(retained, &canonical, &content_ref)?;
        Ok(QualifiedAuthoredRequest {
            request_type: TypeId::of::<S::Request>(),
            value: Box::new(request),
            proposed: ProposedValueMaterial {
                canonical,
                content_ref,
            },
        })
    }

    fn erase_settlement(&self, settlement: Settlement<S>) -> Result<QualifiedSettlement> {
        match settlement {
            Settlement::Succeeded { output, facts } => {
                let output_slots = self
                    .settlement_codecs
                    .output_projectors
                    .iter()
                    .map(|projector| projector.project(&output))
                    .collect::<Result<Vec<_>>>()?;
                verify_fact_proposals(
                    &facts,
                    self.definition
                        .contract()
                        .settlement_contract()
                        .fact_slots(),
                )?;
                Ok(QualifiedSettlement::Succeeded {
                    output_slots,
                    facts,
                })
            }
            Settlement::Failed { failure } => {
                let codec = self
                    .settlement_codecs
                    .failure_codec
                    .as_ref()
                    .ok_or_else(|| {
                        ProgramError::Codec(
                            "state returned a failure without a certified failure contract"
                                .to_owned(),
                        )
                    })?;
                let (canonical, content_ref) = codec.encode(&failure)?;
                Ok(QualifiedSettlement::Failed {
                    failure: ProposedValueMaterial {
                        canonical,
                        content_ref,
                    },
                })
            }
        }
    }

    fn decode_read_outcome(
        &self,
        material: &VerifiedReadOutcome,
        returned_codec: &crate::CanonicalCodec<S::Observation>,
        diagnostic_codec: &crate::CanonicalCodec<S::SafeDiagnostic>,
    ) -> Result<ObservationOutcome<S::Observation, S::SafeDiagnostic>> {
        match material {
            VerifiedReadOutcome::Returned(value) => {
                verify_material(value, returned_codec.value_contract())?;
                Ok(ObservationOutcome::Returned(
                    returned_codec.decode(value.canonical().as_bytes())?,
                ))
            }
            VerifiedReadOutcome::DidNotEnter {
                metadata,
                diagnostic,
            } => {
                let diagnostic = diagnostic
                    .as_ref()
                    .map(|value| {
                        verify_material(value, diagnostic_codec.value_contract())?;
                        diagnostic_codec.decode(value.canonical().as_bytes())
                    })
                    .transpose()?;
                Ok(ObservationOutcome::DidNotEnter(
                    crate::ObservedSafeFailure::new(metadata.clone(), diagnostic),
                ))
            }
            VerifiedReadOutcome::Indeterminate {
                metadata,
                diagnostic,
            } => {
                let diagnostic = diagnostic
                    .as_ref()
                    .map(|value| {
                        verify_material(value, diagnostic_codec.value_contract())?;
                        diagnostic_codec.decode(value.canonical().as_bytes())
                    })
                    .transpose()?;
                Ok(ObservationOutcome::Indeterminate(
                    crate::ObservedSafeFailure::new(metadata.clone(), diagnostic),
                ))
            }
        }
    }

    pub(crate) fn candidate_author_request(
        &self,
        frame: &VerifiedStateFrameMaterial,
    ) -> std::result::Result<QualifiedAuthoredRequest, CandidateStateCallbackError> {
        let (request, codec) = match &self.execution {
            StateExecution::Read(execution) => (
                self.with_candidate_frame(frame, |frame| (execution.request())(frame))?,
                execution.request_codec(),
            ),
            StateExecution::Effect(execution) => (
                self.with_candidate_frame(frame, |frame| (execution.request())(frame))?,
                execution.request_codec(),
            ),
            StateExecution::Pure(_) => {
                return Err(CandidateStateCallbackError::Integrity(
                    ProgramError::Registry(
                        "pure state cannot author an external request".to_owned(),
                    ),
                ));
            }
        };
        self.encode_request(request, codec)
            .map_err(CandidateStateCallbackError::Integrity)
    }

    pub(crate) fn candidate_decode_request(
        &self,
        retained: &VerifiedValueMaterial,
    ) -> std::result::Result<QualifiedAuthoredRequest, CandidateStateCallbackError> {
        match &self.execution {
            StateExecution::Read(execution) => {
                self.decode_request_with(retained, execution.request_codec())
            }
            StateExecution::Effect(execution) => {
                self.decode_request_with(retained, execution.request_codec())
            }
            StateExecution::Pure(_) => Err(ProgramError::Registry(
                "pure state has no request".to_owned(),
            )),
        }
        .map_err(CandidateStateCallbackError::Integrity)
    }

    pub(crate) fn candidate_settle_pure(
        &self,
        frame: &VerifiedStateFrameMaterial,
    ) -> std::result::Result<QualifiedSettlement, CandidateStateCallbackError> {
        let StateExecution::Pure(execution) = &self.execution else {
            return Err(CandidateStateCallbackError::Integrity(
                ProgramError::Registry("non-pure state cannot use pure settlement".to_owned()),
            ));
        };
        let settlement = self.with_candidate_frame(frame, |frame| (execution.apply())(frame))?;
        self.erase_settlement(settlement)
            .map_err(CandidateStateCallbackError::Integrity)
    }

    pub(crate) fn candidate_settle_read(
        &self,
        frame: &VerifiedStateFrameMaterial,
        observation: &VerifiedReadOutcome,
    ) -> std::result::Result<QualifiedEvidenceVerdict, CandidateStateCallbackError> {
        let StateExecution::Read(execution) = &self.execution else {
            return Err(CandidateStateCallbackError::Integrity(
                ProgramError::Registry(
                    "non-read state cannot reduce a read observation".to_owned(),
                ),
            ));
        };
        let outcome = self
            .decode_read_outcome(
                observation,
                execution.observation_codec(),
                execution.diagnostic_codec(),
            )
            .map_err(CandidateStateCallbackError::Integrity)?;
        let verdict = self.with_candidate_frame(frame, |frame| {
            (execution.apply())(frame, ObservationView::new(&outcome))
        })?;
        erase_verdict(verdict, |settlement| self.erase_settlement(settlement))
            .map_err(CandidateStateCallbackError::Integrity)
    }

    pub(crate) fn candidate_settle_effect(
        &self,
        frame: &VerifiedStateFrameMaterial,
        terminal: VerifiedTerminalEffectView<'_>,
    ) -> std::result::Result<QualifiedEvidenceVerdict, CandidateStateCallbackError> {
        let StateExecution::Effect(execution) = &self.execution else {
            return Err(CandidateStateCallbackError::Integrity(
                ProgramError::Registry(
                    "non-effect state cannot reduce terminal effect evidence".to_owned(),
                ),
            ));
        };
        let verdict =
            self.with_candidate_frame(frame, |frame| (execution.settle())(frame, terminal))?;
        erase_verdict(verdict, |settlement| self.erase_settlement(settlement))
            .map_err(CandidateStateCallbackError::Integrity)
    }
}

impl<S: State> QualifiedStateCallbacks for TypedQualifiedStateCallbacks<S> {
    fn kind(&self) -> StateExecutionKind {
        self.execution.kind()
    }

    fn operation(&self) -> Option<&crate::CapabilityOperation> {
        match &self.execution {
            StateExecution::Pure(_) => None,
            StateExecution::Read(execution) => Some(execution.operation()),
            StateExecution::Effect(execution) => Some(execution.operation()),
        }
    }

    fn request_type(&self) -> TypeId {
        TypeId::of::<S::Request>()
    }

    fn observation_type(&self) -> TypeId {
        TypeId::of::<S::Observation>()
    }

    fn diagnostic_type(&self) -> TypeId {
        TypeId::of::<S::SafeDiagnostic>()
    }

    fn author_request(
        &self,
        frame: &VerifiedStateFrameMaterial,
    ) -> Result<QualifiedAuthoredRequest> {
        self.candidate_author_request(frame)
            .map_err(CandidateStateCallbackError::into_program_error)
    }

    fn decode_request(&self, retained: &VerifiedValueMaterial) -> Result<QualifiedAuthoredRequest> {
        self.candidate_decode_request(retained)
            .map_err(CandidateStateCallbackError::into_program_error)
    }

    fn encode_read_returned(
        &self,
        value: &(dyn Any + Send + Sync),
    ) -> Result<ProposedValueMaterial> {
        let StateExecution::Read(execution) = &self.execution else {
            return Err(ProgramError::Registry(
                "only read states encode returned observations".to_owned(),
            ));
        };
        encode_erased(value, execution.observation_codec())
    }

    fn encode_read_diagnostic(
        &self,
        value: &(dyn Any + Send + Sync),
    ) -> Result<ProposedValueMaterial> {
        let StateExecution::Read(execution) = &self.execution else {
            return Err(ProgramError::Registry(
                "only read states encode safe diagnostics".to_owned(),
            ));
        };
        encode_erased(value, execution.diagnostic_codec())
    }

    fn settle_pure(&self, frame: &VerifiedStateFrameMaterial) -> Result<QualifiedSettlement> {
        self.candidate_settle_pure(frame)
            .map_err(CandidateStateCallbackError::into_program_error)
    }

    fn settle_read(
        &self,
        frame: &VerifiedStateFrameMaterial,
        observation: &VerifiedReadOutcome,
    ) -> Result<QualifiedEvidenceVerdict> {
        self.candidate_settle_read(frame, observation)
            .map_err(CandidateStateCallbackError::into_program_error)
    }

    fn settle_effect(
        &self,
        frame: &VerifiedStateFrameMaterial,
        terminal: VerifiedTerminalEffectView<'_>,
    ) -> Result<QualifiedEvidenceVerdict> {
        self.candidate_settle_effect(frame, terminal)
            .map_err(CandidateStateCallbackError::into_program_error)
    }
}

fn erase_verdict<S: State>(
    verdict: EvidenceVerdict<Settlement<S>>,
    settlement: impl FnOnce(Settlement<S>) -> Result<QualifiedSettlement>,
) -> Result<QualifiedEvidenceVerdict> {
    match verdict {
        EvidenceVerdict::Settlement(value) => {
            settlement(value).map(QualifiedEvidenceVerdict::Settlement)
        }
        EvidenceVerdict::InsufficientEvidence => Ok(QualifiedEvidenceVerdict::InsufficientEvidence),
        EvidenceVerdict::InvalidEvidence => Ok(QualifiedEvidenceVerdict::InvalidEvidence),
    }
}

fn encode_erased<T: Send + Sync + 'static>(
    value: &(dyn Any + Send + Sync),
    codec: &crate::CanonicalCodec<T>,
) -> Result<ProposedValueMaterial> {
    let value = value.downcast_ref::<T>().ok_or_else(|| {
        ProgramError::Codec("qualified adapter returned the wrong concrete type".to_owned())
    })?;
    let (canonical, content_ref) = codec.encode(value)?;
    Ok(ProposedValueMaterial {
        canonical,
        content_ref,
    })
}

fn verify_material(
    material: &VerifiedValueMaterial,
    contract: &RetainedValueContract,
) -> Result<()> {
    let fields = material
        .value_ref()
        .fields()
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    if &fields.schema_id != contract.schema_id()
        || &fields.semantic_type_id != contract.semantic_type_id()
        || &fields.role != contract.role()
        || fields.media_type != contract.media_type()
        || &fields.evidence_contract_ref != contract.evidence_contract_ref()
    {
        return Err(ProgramError::Codec(
            "verified value differs from its retained-value contract".to_owned(),
        ));
    }
    let content_ref = boundary_content_ref(contract.schema_id().clone(), material.canonical())?;
    verify_exact_bytes(material, material.canonical(), &content_ref)
}

fn verify_exact_bytes(
    material: &VerifiedValueMaterial,
    canonical: &PlainCanonicalJsonBytes,
    content_ref: &ContentRef,
) -> Result<()> {
    let fields = material
        .value_ref()
        .fields()
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    let byte_length = u64::try_from(canonical.as_bytes().len())
        .map_err(|_| ProgramError::Codec("canonical byte length exceeds u64".to_owned()))?;
    if material.canonical() != canonical
        || &fields.schema_id != content_ref.schema_id()
        || &fields.content_digest != content_ref.content_digest()
        || fields.byte_length != byte_length
    {
        return Err(ProgramError::Codec(
            "verified value bytes differ from their exact reference".to_owned(),
        ));
    }
    Ok(())
}

fn verify_fact_proposals(facts: &FactSet, slots: &[mfm_spec::CertifiedFactSlot]) -> Result<()> {
    let mut counts = vec![0_u32; slots.len()];
    for proposal in facts.as_slice() {
        let slot_index = usize::try_from(proposal.fact_slot_ordinal())
            .map_err(|_| ProgramError::Codec("fact slot ordinal exceeds usize".to_owned()))?;
        let slot = slots.get(slot_index).ok_or_else(|| {
            ProgramError::Codec("callback fact names an uncertified fact slot".to_owned())
        })?;
        counts[slot_index] = counts[slot_index]
            .checked_add(1)
            .ok_or_else(|| ProgramError::Codec("fact slot count exceeds u32".to_owned()))?;
        if slot.fact_slot_ordinal() != proposal.fact_slot_ordinal()
            || counts[slot_index] > slot.maximum_emissions()
            || proposal.descriptor_ref() != slot.fact_descriptor_ref()
            || !fact_value_matches(proposal.subject(), slot.subject_contract())
            || !fact_value_matches(proposal.response(), slot.response_contract())
        {
            return Err(ProgramError::Codec(
                "callback fact differs from its exact certified slot".to_owned(),
            ));
        }
    }
    if slots
        .iter()
        .zip(counts)
        .any(|(slot, count)| count < slot.minimum_emissions() || count > slot.maximum_emissions())
    {
        return Err(ProgramError::Codec(
            "callback fact counts differ from certified fact-slot bounds".to_owned(),
        ));
    }
    Ok(())
}

fn fact_value_matches(value: &crate::ProposedFactValue, contract: &RetainedValueContract) -> bool {
    value.schema_id() == contract.schema_id()
        && value.semantic_type_id() == contract.semantic_type_id()
        && value.role() == contract.role()
        && value.media_type() == contract.media_type()
        && value.evidence_contract_ref() == contract.evidence_contract_ref()
}
