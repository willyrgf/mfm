use super::*;
use crate::recovery::bounds::LifecycleBound;
use mfm_ids::StatePosition;

/// One execution mode with its complete capability ABI and lifecycle bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Execution {
    /// Deterministic execution with one complete conclusion.
    Pure {
        /// Complete maximum conclusion bytes.
        bound: ConclusionBound,
    },
    /// Duplicate-safe observation with one fused conclusion.
    Read {
        /// Exact capability implementation contract.
        capability_contract_ref: ContentRef,
        /// Exact prepared intent schema.
        intent_contract_ref: ContentRef,
        /// Exact accepted evidence schema.
        evidence_contract_ref: ContentRef,
        /// Exact operational error schema.
        error_contract_ref: ContentRef,
        /// Exact State adapter context schema.
        context_contract_ref: ContentRef,
        /// Pre-bound observational route.
        binding_ref: ContentRef,
        /// Complete maximum conclusion bytes across all alternatives.
        bound: ConclusionBound,
    },
    /// Retained command authority followed by settlement.
    Effect {
        /// Exact capability implementation contract.
        capability_contract_ref: ContentRef,
        /// Exact prepared command schema.
        command_contract_ref: ContentRef,
        /// Exact accepted evidence schema.
        evidence_contract_ref: ContentRef,
        /// Exact operational error schema.
        error_contract_ref: ContentRef,
        /// Exact State adapter context schema.
        context_contract_ref: ContentRef,
        /// Pre-bound mutating route.
        binding_ref: ContentRef,
        /// Complete prepare and conclusion maxima.
        bounds: EffectBounds,
    },
}

/// One immutable typed State and its fully selected recovery contracts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct StateDeclaration(StateData);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StateData {
    pub(crate) state_implementation_ref: ContentRef,
    pub(crate) input_contract_ref: ContentRef,
    pub(crate) output_contract_ref: ContentRef,
    pub(crate) failure_contract_ref: ContentRef,
    pub(crate) execution: Execution,
    pub(crate) classifier: ClassifierBinding,
    pub(crate) handler: HandlerBinding,
    pub(crate) root_maps: Vec<MapBinding>,
    #[serde(deserialize_with = "decode_targets")]
    pub(crate) recovery_targets: Vec<RecoveryTarget>,
    pub(crate) allowances: RecoveryAllowances,
}
impl StateDeclaration {
    /// State implementation identity.
    pub const fn state_implementation_ref(&self) -> &ContentRef {
        &self.0.state_implementation_ref
    }
    /// Exact input contract.
    pub const fn input_contract_ref(&self) -> &ContentRef {
        &self.0.input_contract_ref
    }
    /// Exact success contract.
    pub const fn output_contract_ref(&self) -> &ContentRef {
        &self.0.output_contract_ref
    }
    /// Exact original failure contract.
    pub const fn failure_contract_ref(&self) -> &ContentRef {
        &self.0.failure_contract_ref
    }
    /// Complete execution-mode association.
    pub const fn execution(&self) -> &Execution {
        &self.0.execution
    }
    /// Selected classifier and mapping parameters.
    pub const fn classifier(&self) -> &ClassifierBinding {
        &self.0.classifier
    }
    /// Independently selected handler and parameters.
    pub const fn handler(&self) -> &HandlerBinding {
        &self.0.handler
    }
    /// Explicit ordered mapping path from original failure to root failure.
    pub fn root_maps(&self) -> &[MapBinding] {
        &self.0.root_maps
    }
    /// Lowered permitted targets, in author-selected order.
    pub fn recovery_targets(&self) -> &[RecoveryTarget] {
        &self.0.recovery_targets
    }
    /// Per-occurrence committed recovery allowances.
    pub const fn allowances(&self) -> RecoveryAllowances {
        self.0.allowances
    }
}

/// Immutable content-addressed ordered State sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    entry_point_id: EntryPointId,
    admitted_context_contract_ref: ContentRef,
    initial_value_ref: ContentRef,
    root_success_contract_ref: ContentRef,
    root_failure_contract_ref: ContentRef,
    declarations: Vec<StateDeclaration>,
    limits: ProgramLimits,
    canonical_bytes: PlainCanonicalJsonBytes,
    content_ref: ContentRef,
}
impl Program {
    pub(crate) fn new(
        entry_point_id: EntryPointId,
        admitted: ContentRef,
        initial_value_ref: ContentRef,
        success: ContentRef,
        failure: ContentRef,
        declarations: Vec<StateData>,
        limits: ProgramLimits,
    ) -> Result<Self> {
        validate(&admitted, &success, &failure, &declarations)?;
        if initial_value_ref.schema_id() != admitted.schema_id() {
            return Err(ProgramError::InvalidContract);
        }
        let wire = ProgramWire {
            domain: "mfm.program.v5".into(),
            entry_point_id: entry_point_id.clone(),
            admitted_context_contract_ref: admitted.clone(),
            initial_value_ref: initial_value_ref.clone(),
            root_success_contract_ref: success.clone(),
            root_failure_contract_ref: failure.clone(),
            limits,
            declarations: &declarations,
        };
        let json = serde_json::to_string(&wire).map_err(|_| ProgramError::Canonical)?;
        let canonical_bytes =
            PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| ProgramError::Canonical)?;
        if canonical_bytes.as_bytes().len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
            return Err(ProgramError::Capacity);
        }
        let schema = SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm-program-document",
            SchemaVersion::new("5").map_err(|_| ProgramError::InvalidContract)?,
            SchemaShape::CanonicalJsonTerminal {
                profile: CanonicalJsonProfile::GeneralFloatFree,
            },
        )
        .and_then(|identity| identity.schema_id())
        .map_err(|_| ProgramError::InvalidContract)?;
        let content_ref = ContentRef::new(schema, raw_content_digest(canonical_bytes.as_bytes()))
            .map_err(|_| ProgramError::InvalidContract)?;
        Ok(Self {
            entry_point_id,
            admitted_context_contract_ref: admitted,
            initial_value_ref,
            root_success_contract_ref: success,
            root_failure_contract_ref: failure,
            declarations: declarations.into_iter().map(StateDeclaration).collect(),
            limits,
            canonical_bytes,
            content_ref,
        })
    }
    /// Returns the exact checked initial value to which this sequence is specialized.
    pub const fn initial_value_ref(&self) -> &ContentRef {
        &self.initial_value_ref
    }

    /// Decodes the sole current canonical Program format and checks every structural contract.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
            return Err(ProgramError::Capacity);
        }
        PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ProgramError::Canonical)?;
        let wire: ProgramWire =
            serde_json::from_slice(bytes).map_err(|_| ProgramError::Canonical)?;
        if wire.domain != "mfm.program.v5" {
            return Err(ProgramError::Canonical);
        }
        let program = Self::new(
            wire.entry_point_id,
            wire.admitted_context_contract_ref,
            wire.initial_value_ref,
            wire.root_success_contract_ref,
            wire.root_failure_contract_ref,
            wire.declarations,
            wire.limits,
        )?;
        if program.canonical_bytes() != bytes {
            return Err(ProgramError::Canonical);
        }
        Ok(program)
    }
    /// Exact dispatch identity.
    pub const fn entry_point_id(&self) -> &EntryPointId {
        &self.entry_point_id
    }
    /// Admitted input contract.
    pub const fn admitted_context_contract_ref(&self) -> &ContentRef {
        &self.admitted_context_contract_ref
    }
    /// Terminal success contract.
    pub const fn root_success_contract_ref(&self) -> &ContentRef {
        &self.root_success_contract_ref
    }
    /// Terminal domain failure contract.
    pub const fn root_failure_contract_ref(&self) -> &ContentRef {
        &self.root_failure_contract_ref
    }
    /// Ordered State declarations; normal success advances by one.
    pub fn declarations(&self) -> &[StateDeclaration] {
        &self.declarations
    }
    /// Global committed recovery-decision limit.
    pub const fn limits(&self) -> ProgramLimits {
        self.limits
    }
    /// Complete canonical definition, including policies, parameters, checkpoints, and bounds.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical_bytes.as_bytes()
    }
    /// Exact content identity.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }
    /// Computes conservative complete-history costs; Runtime compares these with Journal ceilings.
    pub fn history_bound(&self, genesis: ConclusionBound) -> Result<HistoryBound> {
        HistoryBound::calculate(
            genesis,
            self.limits,
            self.declarations
                .iter()
                .map(|state| match state.0.execution {
                    Execution::Pure { bound } | Execution::Read { bound, .. } => {
                        LifecycleBound::Conclusion(bound)
                    }
                    Execution::Effect { bounds, .. } => LifecycleBound::Effect(bounds),
                }),
        )
    }
}

fn validate(
    admitted: &ContentRef,
    success: &ContentRef,
    failure: &ContentRef,
    states: &[StateData],
) -> Result<()> {
    if states.len() > MAX_STATES {
        return Err(ProgramError::Capacity);
    }
    let never = nominal_contract_ref::<Never>()?;
    let no_context = nominal_contract_ref::<NoContext>()?;
    if admitted == &never || success == &never {
        return Err(ProgramError::InvalidContract);
    }
    if states.is_empty() {
        return if admitted == success && failure == &never {
            Ok(())
        } else {
            Err(ProgramError::InvalidContract)
        };
    }
    let mut current = admitted;
    for (index, state) in states.iter().enumerate() {
        let classifier = state.classifier.abi();
        let source = classifier.source();
        if &state.input_contract_ref != current
            || state.output_contract_ref == never
            || source.domain() != &state.failure_contract_ref
            || &classifier.mapped() != state.handler.abi().input()
        {
            return Err(ProgramError::InvalidContract);
        }
        let (error, context) = match &state.execution {
            Execution::Pure { .. } => (&never, &no_context),
            Execution::Read {
                error_contract_ref,
                context_contract_ref,
                ..
            }
            | Execution::Effect {
                error_contract_ref,
                context_contract_ref,
                ..
            } => (error_contract_ref, context_contract_ref),
        };
        if source.error() != error
            || source.context() != context
            || !state.classifier.params().matches(classifier.params())
            || !state
                .classifier
                .domain_params()
                .matches(classifier.domain_map().params())
            || !state
                .classifier
                .context_params()
                .matches(classifier.context_map().params())
            || !state.handler.params().matches(state.handler.abi().params())
        {
            return Err(ProgramError::InvalidContract);
        }
        if state.root_maps.len() > 64 {
            return Err(ProgramError::Capacity);
        }
        let mut root = &state.failure_contract_ref;
        for map in &state.root_maps {
            if map.abi().input() != root || !map.params().matches(map.abi().params()) {
                return Err(ProgramError::InvalidContract);
            }
            root = map.abi().output();
        }
        if root != failure {
            return Err(ProgramError::InvalidContract);
        }
        let mut targets = std::collections::BTreeSet::new();
        for target in &state.recovery_targets {
            if target.position().index() > index || !targets.insert(target.position()) {
                return Err(ProgramError::InvalidContract);
            }
        }
        current = &state.output_contract_ref;
    }
    if current != success {
        return Err(ProgramError::InvalidContract);
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgramWire<D = Vec<StateData>> {
    domain: String,
    entry_point_id: EntryPointId,
    admitted_context_contract_ref: ContentRef,
    initial_value_ref: ContentRef,
    root_success_contract_ref: ContentRef,
    root_failure_contract_ref: ContentRef,
    declarations: D,
    limits: ProgramLimits,
}
fn decode_targets<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<RecoveryTarget>, D::Error> {
    Vec::<StatePosition>::deserialize(deserializer).map(|positions| {
        positions
            .into_iter()
            .map(|position| RecoveryTarget { position })
            .collect()
    })
}
