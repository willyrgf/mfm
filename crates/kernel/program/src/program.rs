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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDeclaration {
    pub(crate) state_implementation_ref: ContentRef,
    pub(crate) input_contract_ref: ContentRef,
    pub(crate) output_contract_ref: ContentRef,
    pub(crate) failure_contract_ref: ContentRef,
    pub(crate) execution: Execution,
    pub(crate) classifier: ClassifierBinding,
    pub(crate) handler: HandlerBinding,
    pub(crate) root_maps: Vec<MapBinding>,
    pub(crate) recovery_targets: Vec<RecoveryTarget>,
    pub(crate) allowances: RecoveryAllowances,
}
impl StateDeclaration {
    /// State implementation identity.
    pub const fn state_implementation_ref(&self) -> &ContentRef {
        &self.state_implementation_ref
    }
    /// Exact input contract.
    pub const fn input_contract_ref(&self) -> &ContentRef {
        &self.input_contract_ref
    }
    /// Exact success contract.
    pub const fn output_contract_ref(&self) -> &ContentRef {
        &self.output_contract_ref
    }
    /// Exact original failure contract.
    pub const fn failure_contract_ref(&self) -> &ContentRef {
        &self.failure_contract_ref
    }
    /// Complete execution-mode association.
    pub const fn execution(&self) -> &Execution {
        &self.execution
    }
    /// Selected classifier and mapping parameters.
    pub const fn classifier(&self) -> &ClassifierBinding {
        &self.classifier
    }
    /// Independently selected handler and parameters.
    pub const fn handler(&self) -> &HandlerBinding {
        &self.handler
    }
    /// Explicit ordered mapping path from original failure to root failure.
    pub fn root_maps(&self) -> &[MapBinding] {
        &self.root_maps
    }
    /// Lowered permitted targets, in author-selected order.
    pub fn recovery_targets(&self) -> &[RecoveryTarget] {
        &self.recovery_targets
    }
    /// Per-occurrence committed recovery allowances.
    pub const fn allowances(&self) -> RecoveryAllowances {
        self.allowances
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
        declarations: Vec<StateDeclaration>,
        limits: ProgramLimits,
    ) -> Result<Self> {
        validate(&admitted, &success, &failure, &declarations)?;
        if initial_value_ref.schema_id() != admitted.schema_id() {
            return Err(ProgramError::InvalidContract);
        }
        let wire = ProgramWire {
            domain: "mfm.program.v4".into(),
            entry_point_id: entry_point_id.clone(),
            admitted_context_contract_ref: admitted.clone(),
            initial_value_ref: initial_value_ref.clone(),
            root_success_contract_ref: success.clone(),
            root_failure_contract_ref: failure.clone(),
            limits,
            declarations: declarations.iter().map(StateWire::from).collect(),
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
            SchemaVersion::new("4").map_err(|_| ProgramError::InvalidContract)?,
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
            declarations,
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
        if wire.domain != "mfm.program.v4" {
            return Err(ProgramError::Canonical);
        }
        let program = Self::new(
            wire.entry_point_id,
            wire.admitted_context_contract_ref,
            wire.initial_value_ref,
            wire.root_success_contract_ref,
            wire.root_failure_contract_ref,
            wire.declarations
                .into_iter()
                .map(StateWire::into_declaration)
                .collect(),
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
            self.declarations.iter().map(|state| match state.execution {
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
    states: &[StateDeclaration],
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
        if state.input_contract_ref() != current
            || state.output_contract_ref() == &never
            || state.classifier.abi().source().domain() != state.failure_contract_ref()
            || state.classifier.abi().mapped() != state.handler.abi().input()
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
        let classifier = state.classifier.abi();
        if classifier.source().error() != error
            || classifier.source().context() != context
            || classifier.domain_map().input() != classifier.source().domain()
            || classifier.domain_map().output() != classifier.mapped().domain()
            || classifier.context_map().input() != classifier.source().context()
            || classifier.context_map().output() != classifier.mapped().context()
            || classifier.source().error() != classifier.mapped().error()
            || classifier.params() != state.classifier.params().contract_ref()
            || classifier.domain_map().params() != state.classifier.domain_params().contract_ref()
            || classifier.context_map().params() != state.classifier.context_params().contract_ref()
            || state.handler.abi().params() != state.handler.params().contract_ref()
        {
            return Err(ProgramError::InvalidContract);
        }
        if state.root_maps.len() > 64 {
            return Err(ProgramError::Capacity);
        }
        let mut root = state.failure_contract_ref();
        for map in &state.root_maps {
            if map.abi().input() != root || map.abi().params() != map.params().contract_ref() {
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
        current = state.output_contract_ref();
    }
    if current != success {
        return Err(ProgramError::InvalidContract);
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgramWire {
    domain: String,
    entry_point_id: EntryPointId,
    admitted_context_contract_ref: ContentRef,
    initial_value_ref: ContentRef,
    root_success_contract_ref: ContentRef,
    root_failure_contract_ref: ContentRef,
    declarations: Vec<StateWire>,
    limits: ProgramLimits,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateWire {
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    execution: Execution,
    classifier: ClassifierBinding,
    handler: HandlerBinding,
    root_maps: Vec<MapBinding>,
    recovery_targets: Vec<StatePosition>,
    allowances: RecoveryAllowances,
}
impl From<&StateDeclaration> for StateWire {
    fn from(s: &StateDeclaration) -> Self {
        Self {
            state_implementation_ref: s.state_implementation_ref.clone(),
            input_contract_ref: s.input_contract_ref.clone(),
            output_contract_ref: s.output_contract_ref.clone(),
            failure_contract_ref: s.failure_contract_ref.clone(),
            execution: s.execution.clone(),
            classifier: s.classifier.clone(),
            handler: s.handler.clone(),
            root_maps: s.root_maps.clone(),
            recovery_targets: s.recovery_targets.iter().map(|t| t.position()).collect(),
            allowances: s.allowances,
        }
    }
}
impl StateWire {
    fn into_declaration(self) -> StateDeclaration {
        StateDeclaration {
            state_implementation_ref: self.state_implementation_ref,
            input_contract_ref: self.input_contract_ref,
            output_contract_ref: self.output_contract_ref,
            failure_contract_ref: self.failure_contract_ref,
            execution: self.execution,
            classifier: self.classifier,
            handler: self.handler,
            root_maps: self.root_maps,
            recovery_targets: self
                .recovery_targets
                .into_iter()
                .map(|position| RecoveryTarget { position })
                .collect(),
            allowances: self.allowances,
        }
    }
}
