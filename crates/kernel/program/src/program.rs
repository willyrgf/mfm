use super::*;
use mfm_ids::StatePosition;

/// One execution mode with its complete capability ABI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Execution {
    /// Deterministic execution with one complete conclusion.
    Pure {},
    /// Duplicate-safe observation with one fused conclusion.
    Read {
        /// Exact semantic and native implementation/value contracts.
        abi: NativeAbi,
        /// Pre-bound observational route.
        binding_ref: ContentRef,
    },
    /// Retained command authority followed by settlement.
    Effect {
        /// Exact semantic and native implementation/value contracts.
        abi: NativeAbi,
        /// Pre-bound mutating route.
        binding_ref: ContentRef,
    },
}

impl Execution {
    pub(crate) fn native(&self) -> Option<&NativeAbi> {
        match self {
            Self::Pure {} => None,
            Self::Read { abi, .. } | Self::Effect { abi, .. } => Some(abi),
        }
    }
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
    pub(crate) handler: HandlerBinding,
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
    /// Independently selected handler and parameters.
    pub const fn handler(&self) -> &HandlerBinding {
        &self.0.handler
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
pub(crate) struct ProgramDocument {
    entry_point_id: EntryPointId,
    admitted_context_contract_ref: ContentRef,
    initial_value_ref: ContentRef,
    root_success_contract_ref: ContentRef,
    declarations: Vec<StateDeclaration>,
    pub(crate) bindings: Vec<mfm_values::Object>,
    limits: ProgramLimits,
    canonical_bytes: PlainCanonicalJsonBytes,
    content_ref: ContentRef,
}
impl ProgramDocument {
    pub(crate) fn new(
        entry_point_id: EntryPointId,
        admitted: ContentRef,
        initial_value_ref: ContentRef,
        success: ContentRef,
        declarations: Vec<StateData>,
        limits: ProgramLimits,
        bindings: Vec<mfm_values::Object>,
    ) -> Result<Self> {
        validate(&admitted, &success, &declarations)?;
        validate_bindings(&declarations, &bindings)?;
        if initial_value_ref.schema_id() != admitted.schema_id() {
            return Err(ProgramError::InvalidContract);
        }
        let wire = ProgramWire {
            domain: "mfm.program.v9".into(),
            entry_point_id: entry_point_id.clone(),
            admitted_context_contract_ref: admitted.clone(),
            initial_value_ref: initial_value_ref.clone(),
            root_success_contract_ref: success.clone(),
            limits,
            declarations: &declarations,
            bindings: &bindings,
        };
        let canonical_bytes = wire.canonical(MAX_RUN_OBJECT_CANONICAL_BYTES)?;
        let schema = SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm-program-document",
            SchemaVersion::new("9")?,
            SchemaShape::CanonicalJsonTerminal {
                profile: CanonicalJsonProfile::GeneralFloatFree,
            },
        )
        .and_then(|identity| identity.schema_id())?;
        let content_ref = ContentRef::new(schema, raw_content_digest(canonical_bytes.as_bytes()))?;
        Ok(Self {
            entry_point_id,
            admitted_context_contract_ref: admitted,
            initial_value_ref,
            root_success_contract_ref: success,
            declarations: declarations.into_iter().map(StateDeclaration).collect(),
            limits,
            bindings,
            canonical_bytes,
            content_ref,
        })
    }
    /// Returns the exact checked initial value to which this sequence is specialized.
    pub const fn initial_value_ref(&self) -> &ContentRef {
        &self.initial_value_ref
    }

    /// Decodes the sole current canonical Program format and checks every structural contract.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        mfm_values::SizeLimitExceeded::check(
            bytes.len() as u64,
            MAX_RUN_OBJECT_CANONICAL_BYTES as u64,
        )
        .map_err(mfm_values::ValueError::SizeLimit)?;
        PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)?;
        let wire: ProgramWire =
            serde_json::from_slice(bytes).map_err(mfm_canonical::JsonError::new)?;
        if wire.domain != "mfm.program.v9" {
            return Err(ProgramError::Canonical);
        }
        let program = Self::new(
            wire.entry_point_id,
            wire.admitted_context_contract_ref,
            wire.initial_value_ref,
            wire.root_success_contract_ref,
            wire.declarations,
            wire.limits,
            wire.bindings,
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
    /// Ordered State declarations; normal success advances by one.
    pub fn declarations(&self) -> &[StateDeclaration] {
        &self.declarations
    }
    /// Global committed recovery-decision limit.
    pub const fn limits(&self) -> ProgramLimits {
        self.limits
    }
    /// Complete canonical definition, including policies, parameters, and checkpoints.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical_bytes.as_bytes()
    }
    /// Exact content identity.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }
}

fn validate(admitted: &ContentRef, success: &ContentRef, states: &[StateData]) -> Result<()> {
    if states.len() > MAX_STATES {
        return Err(ProgramError::Capacity);
    }
    let never = nominal_contract_ref::<Never>()?;
    if admitted == &never || success == &never {
        return Err(ProgramError::InvalidContract);
    }
    if states.is_empty() {
        return if admitted == success {
            Ok(())
        } else {
            Err(ProgramError::InvalidContract)
        };
    }
    let mut current = admitted;
    for (index, state) in states.iter().enumerate() {
        if &state.input_contract_ref != current
            || state.output_contract_ref == never
            || !state.handler.params().matches(state.handler.abi().params())
        {
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
struct ProgramWire<D = Vec<StateData>, B = Vec<mfm_values::Object>> {
    domain: String,
    entry_point_id: EntryPointId,
    admitted_context_contract_ref: ContentRef,
    initial_value_ref: ContentRef,
    root_success_contract_ref: ContentRef,
    declarations: D,
    bindings: B,
    limits: ProgramLimits,
}
impl<D: Serialize, B: Serialize> ProgramWire<D, B> {
    fn canonical(&self, limit: usize) -> Result<PlainCanonicalJsonBytes> {
        let json = mfm_canonical::to_json_bounded(self, limit)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json)?;
        mfm_values::SizeLimitExceeded::check(canonical.as_bytes().len() as u64, limit as u64)
            .map_err(mfm_values::ValueError::SizeLimit)?;
        Ok(canonical)
    }
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

fn validate_bindings(states: &[StateData], bindings: &[mfm_values::Object]) -> Result<()> {
    if bindings
        .windows(2)
        .any(|pair| pair[0].value_ref() >= pair[1].value_ref())
    {
        return Err(ProgramError::InvalidContract);
    }
    let selected: std::collections::BTreeSet<_> = states
        .iter()
        .filter_map(|state| match &state.execution {
            Execution::Pure {} => None,
            Execution::Read { binding_ref, .. } | Execution::Effect { binding_ref, .. } => {
                Some(binding_ref)
            }
        })
        .collect();
    if selected.len() != bindings.len()
        || bindings
            .iter()
            .any(|binding| !selected.contains(binding.value_ref()))
    {
        return Err(ProgramError::InvalidContract);
    }
    Ok(())
}

/// Complete immutable document and its already-bound executable realization.
#[derive(Clone)]
pub struct Program {
    inner: std::sync::Arc<ProgramInner>,
}
struct ProgramInner {
    document: ProgramDocument,
    executables: Box<[crate::executable::ExecutableState]>,
    contracts: std::collections::BTreeMap<ContentRef, SchemaDescriptor>,
}
impl Program {
    pub(crate) fn freeze(
        document: ProgramDocument,
        executables: Vec<crate::executable::ExecutableState>,
        mut contracts: std::collections::BTreeMap<ContentRef, SchemaDescriptor>,
    ) -> Result<Self> {
        if document.declarations.len() != executables.len() {
            return Err(ProgramError::InvalidContract);
        }
        let required: std::collections::BTreeSet<_> = [
            document.admitted_context_contract_ref(),
            document.root_success_contract_ref(),
        ]
        .into_iter()
        .chain(document.declarations().iter().flat_map(|state| {
            [
                state.input_contract_ref(),
                state.output_contract_ref(),
                state.failure_contract_ref(),
                state.handler().abi().params(),
            ]
        }))
        .chain(document.declarations().iter().flat_map(|state| {
            state
                .execution()
                .native()
                .into_iter()
                .flat_map(NativeAbi::contracts)
        }))
        .collect();
        if required
            .iter()
            .any(|reference| !contracts.contains_key(*reference))
        {
            return Err(ProgramError::InvalidContract);
        }
        contracts.retain(|reference, _| required.contains(reference));
        Ok(Self {
            inner: std::sync::Arc::new(ProgramInner {
                document,
                executables: executables.into_boxed_slice(),
                contracts,
            }),
        })
    }
    /// Exact admitted input commitment.
    pub fn initial_value_ref(&self) -> &ContentRef {
        self.inner.document.initial_value_ref()
    }
    /// Entry-point identity.
    pub fn entry_point_id(&self) -> &EntryPointId {
        self.inner.document.entry_point_id()
    }
    /// Exact root input contract.
    pub fn admitted_context_contract_ref(&self) -> &ContentRef {
        self.inner.document.admitted_context_contract_ref()
    }
    /// Exact terminal success contract.
    pub fn root_success_contract_ref(&self) -> &ContentRef {
        self.inner.document.root_success_contract_ref()
    }
    /// Expanded ordered declarations.
    pub fn declarations(&self) -> &[StateDeclaration] {
        self.inner.document.declarations()
    }
    /// Program-wide recovery allowance.
    pub fn limits(&self) -> ProgramLimits {
        self.inner.document.limits()
    }
    /// Complete canonical document, excluding executable handles.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.inner.document.canonical_bytes()
    }
    /// Document identity, independent of executable pointer identity.
    pub fn content_ref(&self) -> &ContentRef {
        self.inner.document.content_ref()
    }
    /// Canonical public binding Objects in deterministic reference order.
    pub fn bindings(&self) -> &[mfm_values::Object] {
        &self.inner.document.bindings
    }
    /// Kernel-only checked access to the corresponding executable occurrence.
    #[doc(hidden)]
    pub fn executable(
        &self,
        position: mfm_ids::StatePosition,
    ) -> Option<&crate::executable::ExecutableState> {
        self.inner.executables.get(position.index())
    }
    /// Kernel-only admission against an exact contract derived during construction.
    #[doc(hidden)]
    pub fn admit(
        &self,
        value: &mfm_values::Object,
        expected: &ContentRef,
    ) -> std::result::Result<(), mfm_values::InvocationDiagnostic> {
        let descriptor = self.inner.contracts.get(expected).ok_or_else(|| {
            mfm_values::InvocationDiagnostic::from_fields(
                "program_contract",
                "admit",
                expected,
                None,
            )
        })?;
        value
            .admit(descriptor)
            .map_err(|cause| cause.into_diagnostic("admit"))
    }
}

#[cfg(test)]
#[path = "program/tests.rs"]
mod tests;
