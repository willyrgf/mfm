use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_executor::{RequiredPlanExpansion, VerifiedExecutorBinding};
use mfm_ids::{ContentRef, EntryPointId, FieldPath, FieldSegment, StableAuthorKey, StableId};
use mfm_journal::{
    ConfiguredValueBinding, ConfiguredValueKey, FactSelectionScanContract, ReadCapabilityBinding,
    ValueRef,
};
use mfm_spec::{
    AuthoredSourceSelector, CanonicalAuthoredProgram, CapabilityBindingManifest,
    CertifiedAdmissionArtifacts, CertifiedInputDestination, CertifiedJournalProtocolContracts,
    CertifiedNodeContract, CertifiedSettlementContract, CertifiedStateExecution,
    ComponentImplementationDescriptor, ComponentKind, EntryPointContract, ExpandedCertifiedSpec,
    RetainedValueContract, StateImplementationManifest,
};
use mfm_store::{
    AdmittedSupportGraph, AdmittedSupportMember, VerifiedRecordedConfiguredValue, VerifiedRunView,
};

use crate::callbacks::CandidateStateCallbackError;
use crate::{
    boundary_content_ref, ProgramError, QualifiedSettlementCodecs, QualifiedStateCallbacks, Result,
    State, StateConfig, StateContext, StateExecution, TypedQualifiedStateCallbacks,
};

/// Object-safe deterministic certification callback owned by the qualified planner.
pub trait QualifiedCertificationCallback: Send + Sync {
    /// Certifies one exact authored graph against the factory's sealed program definition.
    fn certify(
        &self,
        entry_point: &EntryPointContract,
        authored_program: &CanonicalAuthoredProgram,
    ) -> mfm_spec::Result<CertifiedAdmissionArtifacts>;
}

/// One zero-state factory for the sole certifier bound during registry assembly.
pub trait QualifiedCertificationFactory: Send + Sync {
    /// Creates the sole callback over the exact immutable program definition.
    fn create(
        &self,
        definition: Arc<QualifiedProgramDefinition>,
    ) -> Result<Arc<dyn QualifiedCertificationCallback>>;
}

/// Program-owned canonical identity of the sole composite planner surface.
pub struct CompositePlannerSurface {
    semantic_contract_canonical: PlainCanonicalJsonBytes,
    semantic_contract_ref: ContentRef,
    callback_surface_canonical: PlainCanonicalJsonBytes,
    callback_surface_ref: ContentRef,
}

impl CompositePlannerSurface {
    /// Constructs the sole current composite planner surface.
    pub fn current() -> Result<Self> {
        let semantic_contract_canonical = PlainCanonicalJsonBytes::from_json_str(
            r#"{"version":"mfm.composite-planner-contract.v1"}"#,
        )
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
        let semantic_contract_ref = mfm_spec::exact_content_ref(
            mfm_spec::schema_id("mfm.composite-planner-contract.v1")?,
            &semantic_contract_canonical,
        )?;
        let callback_surface_canonical = PlainCanonicalJsonBytes::from_json_str(
            r#"{"callbacks":["certify"],"version":"mfm.composite-planner-callback-surface.v1"}"#,
        )
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
        let callback_surface_ref = mfm_spec::exact_content_ref(
            mfm_spec::schema_id("mfm.composite-planner-callback-surface.v1")?,
            &callback_surface_canonical,
        )?;
        Ok(Self {
            semantic_contract_canonical,
            semantic_contract_ref,
            callback_surface_canonical,
            callback_surface_ref,
        })
    }

    /// Returns the exact canonical semantic contract.
    pub const fn semantic_contract_canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.semantic_contract_canonical
    }

    /// Returns the exact semantic contract reference.
    pub const fn semantic_contract_ref(&self) -> &ContentRef {
        &self.semantic_contract_ref
    }

    /// Returns the exact canonical callback surface.
    pub const fn callback_surface_canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.callback_surface_canonical
    }

    /// Returns the exact callback surface reference.
    pub const fn callback_surface_ref(&self) -> &ContentRef {
        &self.callback_surface_ref
    }

    /// Constructs the sole planner descriptor for qualification evidence.
    pub fn component_descriptor(
        &self,
        qualification_ref: ContentRef,
    ) -> Result<ComponentImplementationDescriptor> {
        ComponentImplementationDescriptor::new(
            ComponentKind::Planner,
            self.semantic_contract_ref.clone(),
            self.callback_surface_ref.clone(),
            qualification_ref,
        )
        .map_err(Into::into)
    }
}

/// Exact planner identity, descriptor, and sole certification factory.
pub struct QualifiedPlannerRegistration {
    planner_contract_ref: ContentRef,
    component_implementation_ref: ContentRef,
    descriptor: ComponentImplementationDescriptor,
    certification_factory: Arc<dyn QualifiedCertificationFactory>,
}

impl QualifiedPlannerRegistration {
    /// Qualifies one deterministic planner implementation.
    pub fn new(
        descriptor: ComponentImplementationDescriptor,
        certification_factory: Arc<dyn QualifiedCertificationFactory>,
    ) -> Result<Self> {
        let surface = CompositePlannerSurface::current()?;
        let expected = surface.component_descriptor(descriptor.qualification_ref().clone())?;
        if descriptor != expected {
            return Err(ProgramError::Registry(
                "planner descriptor differs from the sole composite certifier surface".to_owned(),
            ));
        }
        let planner_contract_ref = surface.semantic_contract_ref;
        let component_implementation_ref = descriptor.content_ref()?;
        Ok(Self {
            planner_contract_ref,
            component_implementation_ref,
            descriptor,
            certification_factory,
        })
    }

    /// Returns the planner semantic contract.
    pub const fn planner_contract_ref(&self) -> &ContentRef {
        &self.planner_contract_ref
    }

    /// Returns the exact qualified implementation identity.
    pub const fn component_implementation_ref(&self) -> &ContentRef {
        &self.component_implementation_ref
    }

    /// Returns the complete persisted component descriptor.
    pub const fn descriptor(&self) -> &ComponentImplementationDescriptor {
        &self.descriptor
    }
}

/// Exact per-operation contract selected from one admitted read catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedReadOperationContract {
    operation_id: StableId,
    operation_contract_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
    allowed_routing_generation_refs: BTreeSet<ContentRef>,
}

impl QualifiedReadOperationContract {
    /// Constructs one closed read operation and its exact routing generations.
    pub fn new(
        operation_id: StableId,
        operation_contract_ref: ContentRef,
        request_contract: RetainedValueContract,
        returned_contract: RetainedValueContract,
        safe_failure_contract: RetainedValueContract,
        allowed_routing_generation_refs: impl IntoIterator<Item = ContentRef>,
    ) -> Result<Self> {
        let allowed_routing_generation_refs = allowed_routing_generation_refs
            .into_iter()
            .collect::<BTreeSet<_>>();
        if allowed_routing_generation_refs.is_empty() {
            return Err(ProgramError::Registry(
                "read operation requires an admitted routing generation".to_owned(),
            ));
        }
        Ok(Self {
            operation_id,
            operation_contract_ref,
            request_contract,
            returned_contract,
            safe_failure_contract,
            allowed_routing_generation_refs,
        })
    }

    /// Returns the stable read operation identity.
    pub const fn operation_id(&self) -> &StableId {
        &self.operation_id
    }

    /// Returns the exact selected aggregate-catalog entry.
    pub const fn operation_contract_ref(&self) -> &ContentRef {
        &self.operation_contract_ref
    }

    /// Returns the exact authored request contract.
    pub const fn request_contract(&self) -> &RetainedValueContract {
        &self.request_contract
    }

    /// Returns the exact returned-value contract.
    pub const fn returned_contract(&self) -> &RetainedValueContract {
        &self.returned_contract
    }

    /// Returns the exact redaction-safe failure contract.
    pub const fn safe_failure_contract(&self) -> &RetainedValueContract {
        &self.safe_failure_contract
    }

    /// Returns every routing generation admitted by the selected catalog.
    pub const fn allowed_routing_generation_refs(&self) -> &BTreeSet<ContentRef> {
        &self.allowed_routing_generation_refs
    }

    /// Returns whether one exact routed generation is admitted.
    pub fn admits_routing_generation(&self, generation_ref: &ContentRef) -> bool {
        self.allowed_routing_generation_refs
            .contains(generation_ref)
    }
}

/// Immutable semantic definition of one audited read operation.
#[doc(hidden)]
pub struct QualifiedReadDefinition {
    binding_ref: ContentRef,
    binding: ReadCapabilityBinding,
    operation: QualifiedReadOperationContract,
    component_descriptor: ComponentImplementationDescriptor,
}

impl QualifiedReadDefinition {
    /// Returns the exact immutable read binding reference.
    #[doc(hidden)]
    pub fn binding_ref(&self) -> &ContentRef {
        &self.binding_ref
    }

    /// Returns the decoded immutable read binding.
    #[doc(hidden)]
    pub fn binding(&self) -> &ReadCapabilityBinding {
        &self.binding
    }

    /// Returns the selected aggregate operation-catalog entry.
    #[doc(hidden)]
    pub fn operation(&self) -> &QualifiedReadOperationContract {
        &self.operation
    }

    /// Returns the exact admitted adapter/verifier descriptor.
    #[doc(hidden)]
    pub fn component_descriptor(&self) -> &ComponentImplementationDescriptor {
        &self.component_descriptor
    }
}

/// Sole qualified process entry for one audited read operation.
pub struct QualifiedReadEntry {
    definition: Arc<QualifiedReadDefinition>,
    request_type: TypeId,
    returned_type: TypeId,
    diagnostic_type: TypeId,
    invoker_identity: TypeId,
    invoker: Arc<dyn Any + Send + Sync>,
}

impl QualifiedReadEntry {
    /// Qualifies one typed process invoker against its complete semantic entry.
    pub fn new<Request, Returned, Diagnostic, Invoker>(
        binding: ReadCapabilityBinding,
        operation: QualifiedReadOperationContract,
        component_descriptor: ComponentImplementationDescriptor,
        invoker: Invoker,
    ) -> Result<Self>
    where
        Request: Send + Sync + 'static,
        Returned: Send + Sync + 'static,
        Diagnostic: Send + Sync + 'static,
        Invoker: Any + Send + Sync,
    {
        let binding_ref = binding
            .content_ref()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        let fields = binding
            .fields()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        if component_descriptor.component_kind() != ComponentKind::ReadCapabilityAdapterVerifier
            || component_descriptor.semantic_contract_ref() != &fields.capability_contract_ref
            || component_descriptor.content_ref()? != fields.admitted_implementation_ref
        {
            return Err(ProgramError::Registry(
                "read binding and adapter descriptor differ".to_owned(),
            ));
        }
        Ok(Self {
            definition: Arc::new(QualifiedReadDefinition {
                binding_ref,
                binding,
                operation,
                component_descriptor,
            }),
            request_type: TypeId::of::<Request>(),
            returned_type: TypeId::of::<Returned>(),
            diagnostic_type: TypeId::of::<Diagnostic>(),
            invoker_identity: TypeId::of::<Invoker>(),
            invoker: Arc::new(invoker),
        })
    }

    /// Returns the exact immutable read binding reference.
    pub fn binding_ref(&self) -> &ContentRef {
        self.definition.binding_ref()
    }

    /// Returns the decoded immutable read binding.
    pub fn binding(&self) -> &ReadCapabilityBinding {
        self.definition.binding()
    }

    /// Returns the selected aggregate operation-catalog entry.
    pub fn operation(&self) -> &QualifiedReadOperationContract {
        self.definition.operation()
    }

    /// Returns the exact admitted adapter/verifier descriptor.
    pub fn component_descriptor(&self) -> &ComponentImplementationDescriptor {
        self.definition.component_descriptor()
    }

    /// Returns the concrete authored request type identity.
    pub const fn request_type(&self) -> TypeId {
        self.request_type
    }

    /// Returns the concrete returned-value type identity.
    pub const fn returned_type(&self) -> TypeId {
        self.returned_type
    }

    /// Returns the concrete optional safe-diagnostic type identity.
    pub const fn diagnostic_type(&self) -> TypeId {
        self.diagnostic_type
    }

    /// Returns the private process invoker wrapper identity.
    pub const fn invoker_identity(&self) -> TypeId {
        self.invoker_identity
    }

    /// Downcasts the process-only invoker after semantic validation.
    pub fn invoker<Invoker: Any + Send + Sync>(&self) -> Option<&Invoker> {
        self.invoker.downcast_ref()
    }
}

/// Immutable semantic definition of one effect operation.
#[doc(hidden)]
pub struct QualifiedEffectDefinition {
    operation_id: StableId,
    binding_ref: ContentRef,
    binding: VerifiedExecutorBinding,
    operation_contract: RequiredPlanExpansion,
    component_descriptor: ComponentImplementationDescriptor,
    executor_contract_ref: ContentRef,
}

impl QualifiedEffectDefinition {
    /// Returns the stable executor operation identity.
    #[doc(hidden)]
    pub fn operation_id(&self) -> &StableId {
        &self.operation_id
    }

    /// Returns the exact immutable executor binding reference.
    #[doc(hidden)]
    pub fn binding_ref(&self) -> &ContentRef {
        &self.binding_ref
    }

    /// Returns the fully verified executor binding.
    #[doc(hidden)]
    pub fn binding(&self) -> &VerifiedExecutorBinding {
        &self.binding
    }

    /// Returns the exact operation expansion contract.
    #[doc(hidden)]
    pub fn operation_contract(&self) -> &RequiredPlanExpansion {
        &self.operation_contract
    }

    /// Returns the exact admitted executor client/verifier descriptor.
    #[doc(hidden)]
    pub fn component_descriptor(&self) -> &ComponentImplementationDescriptor {
        &self.component_descriptor
    }

    /// Returns the exact executor semantic contract.
    #[doc(hidden)]
    pub const fn executor_contract_ref(&self) -> &ContentRef {
        &self.executor_contract_ref
    }
}

/// Sole qualified process entry for one effect operation.
pub struct QualifiedEffectEntry {
    definition: Arc<QualifiedEffectDefinition>,
    request_type: TypeId,
    response_type: TypeId,
    failure_type: TypeId,
    invoker_identity: TypeId,
    invoker: Arc<dyn Any + Send + Sync>,
}

impl QualifiedEffectEntry {
    /// Qualifies one typed executor invoker against its verified binding.
    pub fn new<Request, Response, Failure, Invoker>(
        operation_id: StableId,
        binding: VerifiedExecutorBinding,
        operation_contract: RequiredPlanExpansion,
        component_descriptor: ComponentImplementationDescriptor,
        invoker: Invoker,
    ) -> Result<Self>
    where
        Request: Send + Sync + 'static,
        Response: Send + Sync + 'static,
        Failure: Send + Sync + 'static,
        Invoker: Any + Send + Sync,
    {
        let binding_ref = binding.binding_ref().as_content_ref().clone();
        let executor_contract_ref = binding
            .contract()
            .reference()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        if operation_contract.executor_operation_id() != operation_id.as_str()
            || component_descriptor.component_kind() != ComponentKind::ExecutorClientVerifier
            || component_descriptor.semantic_contract_ref() != &executor_contract_ref
            || component_descriptor.content_ref()?
                != *binding.binding().admitted_implementation_ref()
        {
            return Err(ProgramError::Registry(
                "effect binding, operation, and executor descriptor differ".to_owned(),
            ));
        }
        Ok(Self {
            definition: Arc::new(QualifiedEffectDefinition {
                operation_id,
                binding_ref,
                binding,
                operation_contract,
                component_descriptor,
                executor_contract_ref,
            }),
            request_type: TypeId::of::<Request>(),
            response_type: TypeId::of::<Response>(),
            failure_type: TypeId::of::<Failure>(),
            invoker_identity: TypeId::of::<Invoker>(),
            invoker: Arc::new(invoker),
        })
    }

    /// Returns the stable executor operation identity.
    pub fn operation_id(&self) -> &StableId {
        self.definition.operation_id()
    }

    /// Returns the exact immutable executor binding reference.
    pub fn binding_ref(&self) -> &ContentRef {
        self.definition.binding_ref()
    }

    /// Returns the fully verified executor binding.
    pub fn binding(&self) -> &VerifiedExecutorBinding {
        self.definition.binding()
    }

    /// Returns the exact operation expansion contract.
    pub fn operation_contract(&self) -> &RequiredPlanExpansion {
        self.definition.operation_contract()
    }

    /// Returns the exact admitted executor client/verifier descriptor.
    pub fn component_descriptor(&self) -> &ComponentImplementationDescriptor {
        self.definition.component_descriptor()
    }

    /// Returns the concrete semantic request type identity.
    pub const fn request_type(&self) -> TypeId {
        self.request_type
    }

    /// Returns the concrete ensure response type identity.
    pub const fn response_type(&self) -> TypeId {
        self.response_type
    }

    /// Returns the concrete executor failure type identity.
    pub const fn failure_type(&self) -> TypeId {
        self.failure_type
    }

    /// Returns the private process invoker wrapper identity.
    pub const fn invoker_identity(&self) -> TypeId {
        self.invoker_identity
    }

    /// Downcasts the process-only invoker after semantic validation.
    pub fn invoker<Invoker: Any + Send + Sync>(&self) -> Option<&Invoker> {
        self.invoker.downcast_ref()
    }
}

/// One segment in a non-root source projection pattern.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum QualifiedSourcePathSegment {
    /// One exact object field.
    Field(FieldSegment),
    /// Any canonical array index at this path position.
    AnyIndex,
}

/// One non-root path pattern in a qualified source contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct QualifiedSourcePathPattern {
    segments: Vec<QualifiedSourcePathSegment>,
}

impl QualifiedSourcePathPattern {
    /// Constructs one nonempty unambiguous path pattern.
    pub fn new(segments: Vec<QualifiedSourcePathSegment>) -> Result<Self> {
        if segments.is_empty() {
            return Err(ProgramError::Registry(
                "qualified source projection path cannot be empty".to_owned(),
            ));
        }
        if segments.iter().any(|segment| {
            matches!(
                segment,
                QualifiedSourcePathSegment::Field(field)
                    if is_canonical_array_index(field.as_str())
            )
        }) {
            return Err(ProgramError::Registry(
                "numeric source fields are ambiguous with array indexes".to_owned(),
            ));
        }
        Ok(Self { segments })
    }

    /// Returns ordered pattern segments.
    pub fn segments(&self) -> &[QualifiedSourcePathSegment] {
        &self.segments
    }

    fn matches(&self, path: &FieldPath) -> bool {
        let concrete = path.as_str().split('.').collect::<Vec<_>>();
        concrete.len() == self.segments.len()
            && self
                .segments
                .iter()
                .zip(concrete)
                .all(|(pattern, concrete)| match pattern {
                    QualifiedSourcePathSegment::Field(field) => field.as_str() == concrete,
                    QualifiedSourcePathSegment::AnyIndex => is_canonical_array_index(concrete),
                })
    }
}

/// One exact non-root source projection and its selected-value contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedSourceProjection {
    path: QualifiedSourcePathPattern,
    value_contract: RetainedValueContract,
}

impl QualifiedSourceProjection {
    /// Binds one non-root projection pattern to its exact retained contract.
    pub const fn new(
        path: QualifiedSourcePathPattern,
        value_contract: RetainedValueContract,
    ) -> Self {
        Self {
            path,
            value_contract,
        }
    }

    /// Returns the exact projection pattern.
    pub const fn path(&self) -> &QualifiedSourcePathPattern {
        &self.path
    }

    /// Returns the retained contract selected by this projection.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }
}

/// Complete typed projection contract for one source root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedSourceContract {
    root_contract: RetainedValueContract,
    projections: Vec<QualifiedSourceProjection>,
}

impl QualifiedSourceContract {
    /// Constructs one root contract and sorted, nonoverlapping non-root projections.
    pub fn new(
        root_contract: RetainedValueContract,
        mut projections: Vec<QualifiedSourceProjection>,
    ) -> Result<Self> {
        projections.sort_by(|left, right| left.path.cmp(&right.path));
        if projections
            .windows(2)
            .any(|pair| source_patterns_overlap(&pair[0].path, &pair[1].path))
        {
            return Err(ProgramError::Registry(
                "qualified source projections must be unique and nonoverlapping".to_owned(),
            ));
        }
        Ok(Self {
            root_contract,
            projections,
        })
    }

    /// Returns the complete source-root contract.
    pub const fn root_contract(&self) -> &RetainedValueContract {
        &self.root_contract
    }

    /// Returns non-root projections in canonical pattern order.
    pub fn projections(&self) -> &[QualifiedSourceProjection] {
        &self.projections
    }

    /// Resolves the exact selected-value contract for a concrete optional path.
    pub fn selected_contract(
        &self,
        source_field_path: Option<&FieldPath>,
    ) -> Option<&RetainedValueContract> {
        let Some(path) = source_field_path else {
            return Some(&self.root_contract);
        };
        let mut matching = self
            .projections
            .iter()
            .filter(|projection| projection.path.matches(path));
        let selected = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        Some(&selected.value_contract)
    }
}

fn source_patterns_overlap(
    left: &QualifiedSourcePathPattern,
    right: &QualifiedSourcePathPattern,
) -> bool {
    left.segments.len() == right.segments.len()
        && left
            .segments
            .iter()
            .zip(&right.segments)
            .all(|(left, right)| {
                left == right
                    || matches!(
                        (left, right),
                        (
                            QualifiedSourcePathSegment::AnyIndex,
                            QualifiedSourcePathSegment::Field(field)
                        ) | (
                            QualifiedSourcePathSegment::Field(field),
                            QualifiedSourcePathSegment::AnyIndex
                        ) if is_canonical_array_index(field.as_str())
                    )
            })
}

fn is_canonical_array_index(value: &str) -> bool {
    let bytes = value.as_bytes();
    value == "0"
        || bytes
            .first()
            .is_some_and(|first| matches!(first, b'1'..=b'9'))
            && bytes[1..].iter().all(u8::is_ascii_digit)
}

fn paths_overlap(left: &FieldPath, right: &FieldPath) -> bool {
    let left = left.as_str();
    let right = right.as_str();
    left == right
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('.'))
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

/// One deliberately admitted cross-run evidence root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedCrossRunEvidenceContract {
    certified_evidence_role_ref: ContentRef,
    source_contract: QualifiedSourceContract,
}

impl QualifiedCrossRunEvidenceContract {
    /// Constructs one exact evidence-only root contract.
    pub const fn new(
        certified_evidence_role_ref: ContentRef,
        source_contract: QualifiedSourceContract,
    ) -> Self {
        Self {
            certified_evidence_role_ref,
            source_contract,
        }
    }

    /// Returns the exact admitted evidence role.
    pub const fn certified_evidence_role_ref(&self) -> &ContentRef {
        &self.certified_evidence_role_ref
    }

    /// Returns the complete retained root contract.
    pub const fn source_contract(&self) -> &QualifiedSourceContract {
        &self.source_contract
    }
}

/// Complete source-root contract for one published entry point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedEntryPointInputContract {
    run_admission_contract: QualifiedSourceContract,
    configured_value_contract: QualifiedSourceContract,
    seed_contract: Option<QualifiedSourceContract>,
    context_contract: Option<QualifiedSourceContract>,
    cross_run_effective_output_contract: Option<QualifiedSourceContract>,
    cross_run_evidence_contracts: BTreeMap<ContentRef, QualifiedCrossRunEvidenceContract>,
}

impl QualifiedEntryPointInputContract {
    /// Constructs one complete closed entry-point input contract.
    pub fn new(
        run_admission_contract: QualifiedSourceContract,
        configured_value_contract: QualifiedSourceContract,
        seed_contract: Option<QualifiedSourceContract>,
        context_contract: Option<QualifiedSourceContract>,
        cross_run_effective_output_contract: Option<QualifiedSourceContract>,
        cross_run_evidence_contracts: Vec<QualifiedCrossRunEvidenceContract>,
    ) -> Result<Self> {
        let mut evidence = BTreeMap::new();
        for contract in cross_run_evidence_contracts {
            if evidence
                .insert(contract.certified_evidence_role_ref.clone(), contract)
                .is_some()
            {
                return Err(ProgramError::Registry(
                    "duplicate cross-run evidence role".to_owned(),
                ));
            }
        }
        Ok(Self {
            run_admission_contract,
            configured_value_contract,
            seed_contract,
            context_contract,
            cross_run_effective_output_contract,
            cross_run_evidence_contracts: evidence,
        })
    }

    /// Returns the exact admitted entry-point input contract.
    pub const fn run_admission_contract(&self) -> &QualifiedSourceContract {
        &self.run_admission_contract
    }

    /// Returns the exact configured-value contract.
    pub const fn configured_value_contract(&self) -> &QualifiedSourceContract {
        &self.configured_value_contract
    }

    /// Returns the optional seed-root contract.
    pub const fn seed_contract(&self) -> Option<&QualifiedSourceContract> {
        self.seed_contract.as_ref()
    }

    /// Returns the optional predecessor-context root contract.
    pub const fn context_contract(&self) -> Option<&QualifiedSourceContract> {
        self.context_contract.as_ref()
    }

    /// Returns the optional cross-run effective-output root contract.
    pub const fn cross_run_effective_output_contract(&self) -> Option<&QualifiedSourceContract> {
        self.cross_run_effective_output_contract.as_ref()
    }

    /// Resolves one admitted cross-run evidence root by certified role.
    pub fn cross_run_evidence_contract(
        &self,
        role_ref: &ContentRef,
    ) -> Option<&QualifiedCrossRunEvidenceContract> {
        self.cross_run_evidence_contracts.get(role_ref)
    }

    /// Resolves the declared root contract for one non-node authored source.
    pub fn root_contract_for(
        &self,
        source: &AuthoredSourceSelector,
    ) -> Option<&RetainedValueContract> {
        let path = source.source_field_path();
        match source {
            AuthoredSourceSelector::RunAdmission { .. } => {
                self.run_admission_contract.selected_contract(path)
            }
            AuthoredSourceSelector::Config { .. } => {
                self.configured_value_contract.selected_contract(path)
            }
            AuthoredSourceSelector::QualifiedSupport { .. } => None,
            AuthoredSourceSelector::Seed { .. } => {
                self.seed_contract.as_ref()?.selected_contract(path)
            }
            AuthoredSourceSelector::Context { .. } => {
                self.context_contract.as_ref()?.selected_contract(path)
            }
            AuthoredSourceSelector::CrossRunEffectiveOutput { .. } => self
                .cross_run_effective_output_contract
                .as_ref()?
                .selected_contract(path),
            AuthoredSourceSelector::CrossRunEvidence {
                certified_evidence_role_ref,
                ..
            } => self
                .cross_run_evidence_contracts
                .get(certified_evidence_role_ref)
                .and_then(|contract| contract.source_contract().selected_contract(path)),
            AuthoredSourceSelector::NodeOutput { .. } => None,
        }
    }
}

/// Exact configured value admitted before deterministic program authoring.
///
/// This value carries no store lookup or mutation capability. Its constructor
/// checks the complete immutable binding, retained contract, and exact bytes.
pub struct VerifiedConfiguredValue<'a> {
    key: ConfiguredValueKey,
    binding: &'a ConfiguredValueBinding,
    value: QualifiedConfiguredValue<'a>,
}

/// Producer-independent configured value exposed to deterministic authors.
///
/// This projection contains exact retained bytes but no store lookup,
/// admission, or mutable configuration authority.
pub struct QualifiedConfiguredValue<'a> {
    value_ref: &'a ValueRef,
    canonical: PlainCanonicalJsonBytes,
}

impl<'a> VerifiedConfiguredValue<'a> {
    fn from_verified_parts(
        binding: &'a ConfiguredValueBinding,
        value_ref: &'a ValueRef,
        bytes: &[u8],
        expected_contract: &RetainedValueContract,
    ) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        let fields = binding
            .fields()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        if fields.value_ref.as_bytes() != value_ref.as_bytes() {
            return Err(ProgramError::Registry(
                "configured binding and retained reference differ".to_owned(),
            ));
        }
        verify_configured_material(value_ref, &canonical, expected_contract)?;
        Ok(Self {
            key: fields.key,
            binding,
            value: QualifiedConfiguredValue {
                value_ref,
                canonical,
            },
        })
    }

    /// Returns the exact configured-value key.
    pub const fn key(&self) -> &ConfiguredValueKey {
        &self.key
    }

    /// Returns the immutable configured-value binding.
    pub const fn binding(&self) -> &ConfiguredValueBinding {
        self.binding
    }

    /// Returns the complete producer-bound retained reference.
    pub const fn value_ref(&self) -> &ValueRef {
        self.value.value_ref()
    }

    /// Returns the exact verified canonical bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        self.value.canonical()
    }

    fn authoring_value(&self) -> &QualifiedConfiguredValue<'a> {
        &self.value
    }
}

impl QualifiedConfiguredValue<'_> {
    /// Returns the complete producer-bound retained reference.
    pub const fn value_ref(&self) -> &ValueRef {
        self.value_ref
    }

    /// Returns the exact verified canonical bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }
}

struct ReplayConfiguredValue<'a> {
    _recorded_binding: &'a ConfiguredValueBinding,
    _target: &'a StableId,
    value: QualifiedConfiguredValue<'a>,
}

impl<'a> ReplayConfiguredValue<'a> {
    fn from_verified(
        recorded: &'a VerifiedRecordedConfiguredValue,
        recorded_operation_id: &StableId,
        expected_contract: &RetainedValueContract,
    ) -> std::result::Result<Self, CandidateCertificationError> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(recorded.bytes())
            .map_err(|error| {
                CandidateCertificationError::integrity(ProgramError::Registry(error.to_string()))
            })?;
        let binding = recorded.recorded_binding();
        let binding_fields = binding.fields().map_err(|error| {
            CandidateCertificationError::integrity(ProgramError::Registry(error.to_string()))
        })?;
        let key_fields = binding_fields.key.fields().map_err(|error| {
            CandidateCertificationError::integrity(ProgramError::Registry(error.to_string()))
        })?;
        if recorded.target() != recorded_operation_id
            || key_fields.target != *recorded.target()
            || binding_fields.value_ref.as_bytes() != recorded.value_ref().as_bytes()
        {
            return Err(CandidateCertificationError::integrity(
                ProgramError::Registry(
                    "recorded configured material contradicts its operation or binding".to_owned(),
                ),
            ));
        }
        verify_configured_material(recorded.value_ref(), &canonical, expected_contract)
            .map_err(CandidateCertificationError::integrity)?;
        Ok(Self {
            _recorded_binding: binding,
            _target: recorded.target(),
            value: QualifiedConfiguredValue {
                value_ref: recorded.value_ref(),
                canonical,
            },
        })
    }

    fn authoring_value(&self) -> &QualifiedConfiguredValue<'a> {
        &self.value
    }
}

/// One store support member required by deterministic entry-point authoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedAuthoringRootRequirement {
    member_path: FieldPath,
    source_contract: QualifiedSourceContract,
}

impl QualifiedAuthoringRootRequirement {
    /// Declares one exact typed member of an admitted support graph.
    pub const fn new(member_path: FieldPath, source_contract: QualifiedSourceContract) -> Self {
        Self {
            member_path,
            source_contract,
        }
    }

    /// Returns the exact support member path.
    pub const fn member_path(&self) -> &FieldPath {
        &self.member_path
    }

    /// Returns the complete retained member contract.
    pub const fn source_contract(&self) -> &QualifiedSourceContract {
        &self.source_contract
    }
}

/// One exact store-admitted authoring root.
///
/// Construction is private and always checks the sealed admitted support
/// member, retained contract, and exact canonical bytes.
pub struct QualifiedAuthoringRoot<'a> {
    member_path: &'a FieldPath,
    source_contract: &'a QualifiedSourceContract,
    value_ref: &'a ValueRef,
    bytes: &'a [u8],
}

impl<'a> QualifiedAuthoringRoot<'a> {
    /// Returns the exact admitted support member path.
    pub const fn member_path(&self) -> &FieldPath {
        self.member_path
    }

    /// Returns the retained contract verified for this root.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        self.source_contract.root_contract()
    }

    /// Returns the complete projection contract for this root.
    pub const fn source_contract(&self) -> &QualifiedSourceContract {
        self.source_contract
    }

    /// Returns the store-authored retained authority.
    pub const fn value_ref(&self) -> &ValueRef {
        self.value_ref
    }

    /// Returns exact canonical root bytes.
    pub const fn bytes(&self) -> &[u8] {
        self.bytes
    }
}

/// Closed deterministic authoring inputs for one registered entry point.
pub struct QualifiedAuthoringInputs<'a> {
    configured: &'a QualifiedConfiguredValue<'a>,
    roots: Vec<QualifiedAuthoringRoot<'a>>,
}

impl<'a> QualifiedAuthoringInputs<'a> {
    /// Returns the exact verified configured value.
    pub const fn configured(&self) -> &QualifiedConfiguredValue<'a> {
        self.configured
    }

    /// Returns registration-declared roots in member-path order.
    pub fn roots(&self) -> &[QualifiedAuthoringRoot<'a>] {
        &self.roots
    }

    /// Resolves one exact declared authoring root.
    pub fn root(&self, member_path: &FieldPath) -> Option<&QualifiedAuthoringRoot<'a>> {
        self.roots
            .binary_search_by(|root| root.member_path.cmp(member_path))
            .ok()
            .map(|index| &self.roots[index])
    }
}

/// Object-safe deterministic package author for one entry point.
pub trait QualifiedEntryPointAuthor: Send + Sync {
    /// Authors exact topology from one closed verified input bundle.
    fn author(&self, inputs: &QualifiedAuthoringInputs<'_>) -> Result<CanonicalAuthoredProgram>;
}

impl<F> QualifiedEntryPointAuthor for F
where
    F: for<'a> Fn(&QualifiedAuthoringInputs<'a>) -> Result<CanonicalAuthoredProgram> + Send + Sync,
{
    fn author(&self, inputs: &QualifiedAuthoringInputs<'_>) -> Result<CanonicalAuthoredProgram> {
        self(inputs)
    }
}

/// One destination declared by a semantic state contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedInputContract {
    destination_field_path: FieldPath,
    destination: CertifiedInputDestination,
    source_contract: QualifiedSourceContract,
}

impl QualifiedInputContract {
    /// Constructs one typed input destination.
    pub const fn new(
        destination_field_path: FieldPath,
        destination: CertifiedInputDestination,
        source_contract: QualifiedSourceContract,
    ) -> Self {
        Self {
            destination_field_path,
            destination,
            source_contract,
        }
    }

    /// Returns the destination path.
    pub const fn destination_field_path(&self) -> &FieldPath {
        &self.destination_field_path
    }

    /// Returns the destination authority class.
    pub const fn destination(&self) -> &CertifiedInputDestination {
        &self.destination
    }

    /// Returns the exact retained value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        self.source_contract.root_contract()
    }

    /// Returns the complete projection contract for the assembled value.
    pub const fn source_contract(&self) -> &QualifiedSourceContract {
        &self.source_contract
    }
}

/// Complete projection contract for one successful state output slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedOutputSourceContract {
    output_ordinal: u32,
    source_contract: QualifiedSourceContract,
}

impl QualifiedOutputSourceContract {
    /// Binds one exact output slot to its complete projection contract.
    pub const fn new(output_ordinal: u32, source_contract: QualifiedSourceContract) -> Self {
        Self {
            output_ordinal,
            source_contract,
        }
    }

    /// Returns the certified output ordinal.
    pub const fn output_ordinal(&self) -> u32 {
        self.output_ordinal
    }

    /// Returns the output root and all admitted projections.
    pub const fn source_contract(&self) -> &QualifiedSourceContract {
        &self.source_contract
    }
}

/// Complete occurrence-independent descriptor graph for one state contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedStateContract {
    state_contract_ref: ContentRef,
    config_contract: RetainedValueContract,
    context_contract: Option<RetainedValueContract>,
    input_contract: RetainedValueContract,
    input_destinations: Vec<QualifiedInputContract>,
    output_sources: Vec<QualifiedOutputSourceContract>,
    execution: CertifiedStateExecution,
    settlement_contract: CertifiedSettlementContract,
}

impl QualifiedStateContract {
    /// Constructs one complete semantic state descriptor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_contract_ref: ContentRef,
        config_contract: RetainedValueContract,
        context_contract: Option<RetainedValueContract>,
        input_contract: RetainedValueContract,
        mut input_destinations: Vec<QualifiedInputContract>,
        mut output_sources: Vec<QualifiedOutputSourceContract>,
        execution: CertifiedStateExecution,
        settlement_contract: CertifiedSettlementContract,
    ) -> Result<Self> {
        input_destinations.sort_by(|left, right| {
            left.destination_field_path
                .cmp(&right.destination_field_path)
        });
        if input_destinations.windows(2).any(|pair| {
            paths_overlap(
                &pair[0].destination_field_path,
                &pair[1].destination_field_path,
            )
        }) {
            return Err(ProgramError::Registry(
                "state input destinations must be unique".to_owned(),
            ));
        }
        output_sources.sort_by_key(QualifiedOutputSourceContract::output_ordinal);
        if output_sources.len() != settlement_contract.output_slots().len()
            || output_sources
                .iter()
                .zip(settlement_contract.output_slots())
                .any(|(qualified, certified)| {
                    qualified.output_ordinal() != certified.output_ordinal()
                        || qualified.source_contract().root_contract() != certified.value_contract()
                })
        {
            return Err(ProgramError::Registry(
                "qualified output sources must be total and exact for settlement slots".to_owned(),
            ));
        }
        Ok(Self {
            state_contract_ref,
            config_contract,
            context_contract,
            input_contract,
            input_destinations,
            output_sources,
            execution,
            settlement_contract,
        })
    }

    /// Returns the semantic state contract.
    pub const fn state_contract_ref(&self) -> &ContentRef {
        &self.state_contract_ref
    }

    /// Returns the required configuration value contract.
    pub const fn config_contract(&self) -> &RetainedValueContract {
        &self.config_contract
    }

    /// Returns the optional context value contract.
    pub const fn context_contract(&self) -> Option<&RetainedValueContract> {
        self.context_contract.as_ref()
    }

    /// Returns the whole assembled input-root contract.
    pub const fn input_contract(&self) -> &RetainedValueContract {
        &self.input_contract
    }

    /// Returns declared destinations in path order.
    pub fn input_destinations(&self) -> &[QualifiedInputContract] {
        &self.input_destinations
    }

    /// Returns output projection contracts in ordinal order.
    pub fn output_sources(&self) -> &[QualifiedOutputSourceContract] {
        &self.output_sources
    }

    /// Resolves one exact output projection contract.
    pub fn output_source(&self, output_ordinal: u32) -> Option<&QualifiedSourceContract> {
        self.output_sources
            .binary_search_by_key(
                &output_ordinal,
                QualifiedOutputSourceContract::output_ordinal,
            )
            .ok()
            .map(|index| self.output_sources[index].source_contract())
    }

    /// Returns the closed state execution contract.
    pub const fn execution(&self) -> &CertifiedStateExecution {
        &self.execution
    }

    /// Returns the complete settlement contract.
    pub const fn settlement_contract(&self) -> &CertifiedSettlementContract {
        &self.settlement_contract
    }
}

/// Immutable semantic and implementation definition of one qualified state.
#[doc(hidden)]
pub struct QualifiedStateDefinition {
    contract: QualifiedStateContract,
    component_implementation_ref: ContentRef,
    component_descriptor: ComponentImplementationDescriptor,
}

impl QualifiedStateDefinition {
    /// Returns the complete semantic state contract.
    #[doc(hidden)]
    pub fn contract(&self) -> &QualifiedStateContract {
        &self.contract
    }

    /// Returns the exact qualified component implementation.
    #[doc(hidden)]
    pub const fn component_implementation_ref(&self) -> &ContentRef {
        &self.component_implementation_ref
    }

    /// Returns the persisted component descriptor.
    #[doc(hidden)]
    pub const fn component_descriptor(&self) -> &ComponentImplementationDescriptor {
        &self.component_descriptor
    }
}

trait ErasedQualifiedStateCallbacks: Send + Sync {
    fn callbacks(&self) -> &dyn QualifiedStateCallbacks;

    fn candidate_author_request(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
    ) -> std::result::Result<crate::QualifiedAuthoredRequest, CandidateStateCallbackError>;

    fn candidate_decode_request(
        &self,
        retained: &crate::VerifiedValueMaterial,
    ) -> std::result::Result<crate::QualifiedAuthoredRequest, CandidateStateCallbackError>;

    fn candidate_settle_pure(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
    ) -> std::result::Result<crate::QualifiedSettlement, CandidateStateCallbackError>;

    fn candidate_settle_read(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
        observation: &crate::VerifiedReadOutcome,
    ) -> std::result::Result<crate::QualifiedEvidenceVerdict, CandidateStateCallbackError>;

    fn candidate_settle_effect(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
        terminal: crate::VerifiedTerminalEffectView<'_>,
    ) -> std::result::Result<crate::QualifiedEvidenceVerdict, CandidateStateCallbackError>;
}

struct TypedQualifiedStateCallbacksEntry<S: State> {
    callbacks: TypedQualifiedStateCallbacks<S>,
}

impl<S: State> ErasedQualifiedStateCallbacks for TypedQualifiedStateCallbacksEntry<S> {
    fn callbacks(&self) -> &dyn QualifiedStateCallbacks {
        &self.callbacks
    }

    fn candidate_author_request(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
    ) -> std::result::Result<crate::QualifiedAuthoredRequest, CandidateStateCallbackError> {
        self.callbacks.candidate_author_request(frame)
    }

    fn candidate_decode_request(
        &self,
        retained: &crate::VerifiedValueMaterial,
    ) -> std::result::Result<crate::QualifiedAuthoredRequest, CandidateStateCallbackError> {
        self.callbacks.candidate_decode_request(retained)
    }

    fn candidate_settle_pure(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
    ) -> std::result::Result<crate::QualifiedSettlement, CandidateStateCallbackError> {
        self.callbacks.candidate_settle_pure(frame)
    }

    fn candidate_settle_read(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
        observation: &crate::VerifiedReadOutcome,
    ) -> std::result::Result<crate::QualifiedEvidenceVerdict, CandidateStateCallbackError> {
        self.callbacks.candidate_settle_read(frame, observation)
    }

    fn candidate_settle_effect(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
        terminal: crate::VerifiedTerminalEffectView<'_>,
    ) -> std::result::Result<crate::QualifiedEvidenceVerdict, CandidateStateCallbackError> {
        self.callbacks.candidate_settle_effect(frame, terminal)
    }
}

/// One typed state callback set sealed to its complete descriptor graph.
pub struct QualifiedStateRegistration<S: State> {
    definition: Arc<QualifiedStateDefinition>,
    callbacks: Arc<dyn ErasedQualifiedStateCallbacks>,
    _state: PhantomData<fn(S) -> S>,
}

impl<S: State> Clone for QualifiedStateRegistration<S> {
    fn clone(&self) -> Self {
        Self {
            definition: Arc::clone(&self.definition),
            callbacks: Arc::clone(&self.callbacks),
            _state: PhantomData,
        }
    }
}

impl<S: State> QualifiedStateRegistration<S> {
    /// Qualifies one exact state implementation and its closed callbacks.
    pub fn new(
        contract: QualifiedStateContract,
        component_descriptor: ComponentImplementationDescriptor,
        execution: StateExecution<S>,
        settlement_codecs: QualifiedSettlementCodecs<S>,
    ) -> Result<Self> {
        if component_descriptor.component_kind() != ComponentKind::State
            || component_descriptor.semantic_contract_ref() != contract.state_contract_ref()
            || S::state_contract_ref()? != *contract.state_contract_ref()
        {
            return Err(ProgramError::Registry(
                "state callback, semantic contract, and component descriptor differ".to_owned(),
            ));
        }
        validate_state_type_contract::<S>(&contract)?;
        validate_state_execution(&contract, &execution)?;
        settlement_codecs.validate_against(contract.settlement_contract())?;
        let component_implementation_ref = component_descriptor.content_ref()?;
        let definition = Arc::new(QualifiedStateDefinition {
            contract,
            component_implementation_ref,
            component_descriptor,
        });
        Ok(Self {
            definition: Arc::clone(&definition),
            callbacks: Arc::new(TypedQualifiedStateCallbacksEntry::<S> {
                callbacks: TypedQualifiedStateCallbacks::new(
                    execution,
                    definition,
                    settlement_codecs,
                ),
            }),
            _state: PhantomData,
        })
    }

    /// Returns the complete semantic contract projection.
    pub fn contract(&self) -> &QualifiedStateContract {
        self.definition.contract()
    }

    /// Returns the exact qualified component implementation.
    pub fn component_implementation_ref(&self) -> &ContentRef {
        self.definition.component_implementation_ref()
    }

    /// Returns the complete component descriptor.
    pub fn component_descriptor(&self) -> &ComponentImplementationDescriptor {
        self.definition.component_descriptor()
    }

    /// Returns the sealed object-safe callback surface.
    pub fn callbacks(&self) -> &dyn QualifiedStateCallbacks {
        self.callbacks.callbacks()
    }
}

/// Type-erased qualified state selected by semantic contract.
#[derive(Clone)]
pub struct QualifiedStateEntry {
    definition: Arc<QualifiedStateDefinition>,
    callbacks: Arc<dyn ErasedQualifiedStateCallbacks>,
}

impl QualifiedStateEntry {
    /// Returns the complete semantic state contract.
    pub fn contract(&self) -> &QualifiedStateContract {
        self.definition.contract()
    }

    /// Returns the exact qualified component implementation.
    pub fn component_implementation_ref(&self) -> &ContentRef {
        self.definition.component_implementation_ref()
    }

    /// Returns the persisted component descriptor.
    pub fn component_descriptor(&self) -> &ComponentImplementationDescriptor {
        self.definition.component_descriptor()
    }

    /// Returns the sealed object-safe callback surface.
    pub fn callbacks(&self) -> &dyn QualifiedStateCallbacks {
        self.callbacks.callbacks()
    }
}

/// Immutable semantic definition of one package-owned entry point.
#[doc(hidden)]
pub struct QualifiedEntryPointDefinition {
    entry_point: EntryPointContract,
    input_contract: QualifiedEntryPointInputContract,
    public_output_contract: RetainedValueContract,
    authoring_root_requirements: Vec<QualifiedAuthoringRootRequirement>,
}

impl QualifiedEntryPointDefinition {
    /// Returns the published discovery/admission contract.
    #[doc(hidden)]
    pub const fn entry_point(&self) -> &EntryPointContract {
        &self.entry_point
    }

    /// Returns the complete source-root contract.
    #[doc(hidden)]
    pub const fn input_contract(&self) -> &QualifiedEntryPointInputContract {
        &self.input_contract
    }

    /// Returns the complete assembled public-output contract.
    #[doc(hidden)]
    pub const fn public_output_contract(&self) -> &RetainedValueContract {
        &self.public_output_contract
    }

    /// Returns required admitted authoring roots in member-path order.
    #[doc(hidden)]
    pub fn authoring_root_requirements(&self) -> &[QualifiedAuthoringRootRequirement] {
        &self.authoring_root_requirements
    }

    /// Resolves the exact selected contract for one authored non-node source.
    #[doc(hidden)]
    pub fn authored_source_contract(
        &self,
        source: &AuthoredSourceSelector,
    ) -> Option<&RetainedValueContract> {
        match source {
            AuthoredSourceSelector::QualifiedSupport {
                member_path,
                source_field_path,
            } => self
                .authoring_root_requirements
                .binary_search_by(|requirement| requirement.member_path().cmp(member_path))
                .ok()
                .and_then(|index| {
                    self.authoring_root_requirements[index]
                        .source_contract()
                        .selected_contract(source_field_path.as_ref())
                }),
            _ => self.input_contract.root_contract_for(source),
        }
    }
}

/// Immutable package-owned entry-point registration used during assembly.
#[derive(Clone)]
pub struct QualifiedEntryPointRegistration {
    definition: Arc<QualifiedEntryPointDefinition>,
    author: Arc<dyn QualifiedEntryPointAuthor>,
}

impl QualifiedEntryPointRegistration {
    /// Binds one published contract to its complete input contract and author.
    pub fn new(
        entry_point: EntryPointContract,
        input_contract: QualifiedEntryPointInputContract,
        public_output_contract: RetainedValueContract,
        mut authoring_root_requirements: Vec<QualifiedAuthoringRootRequirement>,
        author: impl QualifiedEntryPointAuthor + 'static,
    ) -> Result<Self> {
        if entry_point.input_schema_id()
            != input_contract
                .run_admission_contract()
                .root_contract()
                .schema_id()
        {
            return Err(ProgramError::Registry(
                "entry-point input schema differs from its qualified input contract".to_owned(),
            ));
        }
        if entry_point.public_output_schema_id() != public_output_contract.schema_id() {
            return Err(ProgramError::Registry(
                "entry-point public-output schema differs from its qualified output contract"
                    .to_owned(),
            ));
        }
        authoring_root_requirements.sort_by(|left, right| left.member_path.cmp(&right.member_path));
        if authoring_root_requirements
            .windows(2)
            .any(|pair| pair[0].member_path == pair[1].member_path)
        {
            return Err(ProgramError::Registry(
                "entry-point authoring root requirements must be unique".to_owned(),
            ));
        }
        Ok(Self {
            definition: Arc::new(QualifiedEntryPointDefinition {
                entry_point,
                input_contract,
                public_output_contract,
                authoring_root_requirements,
            }),
            author: Arc::new(author),
        })
    }

    /// Returns the published discovery/admission contract.
    pub fn entry_point(&self) -> &EntryPointContract {
        self.definition.entry_point()
    }

    /// Returns the complete source-root contract.
    pub fn input_contract(&self) -> &QualifiedEntryPointInputContract {
        self.definition.input_contract()
    }

    /// Returns the complete assembled public-output contract.
    pub fn public_output_contract(&self) -> &RetainedValueContract {
        self.definition.public_output_contract()
    }

    /// Returns required admitted authoring roots in member-path order.
    pub fn authoring_root_requirements(&self) -> &[QualifiedAuthoringRootRequirement] {
        self.definition.authoring_root_requirements()
    }

    /// Resolves the exact selected contract for one authored non-node source.
    pub fn authored_source_contract(
        &self,
        source: &AuthoredSourceSelector,
    ) -> Option<&RetainedValueContract> {
        self.definition.authored_source_contract(source)
    }

    /// Authors exact topology from one closed verified input bundle.
    pub fn author(
        &self,
        inputs: &QualifiedAuthoringInputs<'_>,
    ) -> Result<CanonicalAuthoredProgram> {
        self.validate_authoring_inputs(inputs)?;
        let authored =
            catch_unwind(AssertUnwindSafe(|| self.author.author(inputs))).map_err(|_| {
                ProgramError::Authoring("qualified entry-point author panicked".to_owned())
            })??;
        self.validate_authored_program(&authored)?;
        Ok(authored)
    }

    fn validate_authoring_inputs(&self, inputs: &QualifiedAuthoringInputs<'_>) -> Result<()> {
        let configured = inputs.configured();
        verify_configured_material(
            configured.value_ref(),
            configured.canonical(),
            self.input_contract()
                .configured_value_contract()
                .root_contract(),
        )?;
        if inputs.roots().len() != self.authoring_root_requirements().len()
            || inputs
                .roots()
                .iter()
                .zip(self.authoring_root_requirements())
                .any(|(root, requirement)| {
                    root.member_path() != requirement.member_path()
                        || root.source_contract() != requirement.source_contract()
                })
        {
            return Err(ProgramError::Registry(
                "authoring inputs differ from the registered root requirements".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_authored_program(&self, authored: &CanonicalAuthoredProgram) -> Result<()> {
        if authored.entry_point_operation_id() != self.entry_point().entry_point_operation_id() {
            return Err(ProgramError::Registry(
                "entry-point operation differs from its authored program".to_owned(),
            ));
        }
        authored.canonical_json()?;
        Ok(())
    }

    /// Rechecks store-verified configured parts and authors exact topology.
    ///
    /// This accepts the three borrowed projections exposed by the store's
    /// sealed configured value, so application glue neither copies authority
    /// nor performs another lookup.
    pub fn author_verified_parts<'a>(
        &'a self,
        binding: &'a ConfiguredValueBinding,
        value_ref: &'a ValueRef,
        bytes: &'a [u8],
        admitted_support: &'a AdmittedSupportGraph,
    ) -> Result<CanonicalAuthoredProgram> {
        let configured = VerifiedConfiguredValue::from_verified_parts(
            binding,
            value_ref,
            bytes,
            self.input_contract()
                .configured_value_contract()
                .root_contract(),
        )?;
        let key = configured
            .key()
            .fields()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        if &key.entry_point_id != self.entry_point().entry_point_id() {
            return Err(ProgramError::Registry(
                "configured value belongs to a different entry point".to_owned(),
            ));
        }
        self.author_configured_value(configured.authoring_value(), admitted_support)
    }

    fn author_configured_value<'a>(
        &'a self,
        configured: &'a QualifiedConfiguredValue<'a>,
        admitted_support: &'a AdmittedSupportGraph,
    ) -> Result<CanonicalAuthoredProgram> {
        let inputs = self.authoring_inputs(configured, admitted_support)?;
        self.author(&inputs)
    }

    fn authoring_inputs<'a>(
        &'a self,
        configured: &'a QualifiedConfiguredValue<'a>,
        admitted_support: &'a AdmittedSupportGraph,
    ) -> Result<QualifiedAuthoringInputs<'a>> {
        let mut roots = Vec::with_capacity(self.authoring_root_requirements().len());
        for requirement in self.authoring_root_requirements() {
            let admitted = admitted_support
                .member(requirement.member_path())
                .ok_or_else(|| {
                    ProgramError::Registry(
                        "required authoring root is absent from admitted support".to_owned(),
                    )
                })?;
            verify_retained_material(
                admitted.value_ref(),
                admitted.bytes(),
                requirement.source_contract().root_contract(),
            )?;
            roots.push(QualifiedAuthoringRoot {
                member_path: requirement.member_path(),
                source_contract: requirement.source_contract(),
                value_ref: admitted.value_ref(),
                bytes: admitted.bytes(),
            });
        }
        Ok(QualifiedAuthoringInputs { configured, roots })
    }

    fn candidate_author_configured_value<'a>(
        &'a self,
        configured: &'a QualifiedConfiguredValue<'a>,
        admitted_support: &'a AdmittedSupportGraph,
    ) -> std::result::Result<CanonicalAuthoredProgram, CandidateCertificationError> {
        let inputs = self
            .authoring_inputs(configured, admitted_support)
            .map_err(CandidateCertificationError::integrity)?;
        self.validate_authoring_inputs(&inputs)
            .map_err(CandidateCertificationError::integrity)?;
        let authored = catch_unwind(AssertUnwindSafe(|| self.author.author(&inputs)))
            .map_err(|_| CandidateCertificationError::execution_panic())?
            .map_err(CandidateCertificationError::execution)?;
        self.validate_authored_program(&authored)
            .map_err(CandidateCertificationError::integrity)?;
        Ok(authored)
    }
}

impl fmt::Debug for QualifiedEntryPointRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QualifiedEntryPointRegistration")
            .field("entry_point", self.entry_point())
            .field("input_contract", self.input_contract())
            .field("public_output_contract", self.public_output_contract())
            .field(
                "authoring_root_requirements",
                &self.authoring_root_requirements(),
            )
            .finish_non_exhaustive()
    }
}

/// Relative selector for a node named by one support expansion.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum QualifiedSupportNodeSelector {
    /// The authored occurrence protected by this expansion.
    Protected,
    /// One support occurrence in the same expansion.
    Support(StableAuthorKey),
}

/// One relative source used by a qualified support-node template.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum QualifiedSupportSourceTemplate {
    /// One value already assembled for the protected node.
    BoundaryInput {
        /// Protected input destination whose selected value is reused.
        destination_field_path: FieldPath,
        /// `None` selects that complete value.
        source_field_path: Option<FieldPath>,
    },
    /// One output of the protected or another support occurrence.
    NodeOutput {
        /// Relative producer occurrence.
        producer: QualifiedSupportNodeSelector,
        /// Producer output slot.
        output_ordinal: u32,
        /// `None` selects the complete output value.
        source_field_path: Option<FieldPath>,
    },
    /// One fact emitted by the protected or another support occurrence.
    NodeFact {
        /// Relative producer occurrence.
        producer: QualifiedSupportNodeSelector,
        /// Producer fact slot.
        emission_ordinal: u32,
    },
    /// One projection of the entry-point configuration root.
    Config {
        /// `None` selects the complete root.
        source_field_path: Option<FieldPath>,
    },
    /// One projection of the entry-point seed root.
    Seed {
        /// `None` selects the complete root.
        source_field_path: Option<FieldPath>,
    },
    /// One projection of the predecessor context root.
    Context {
        /// `None` selects the complete root.
        source_field_path: Option<FieldPath>,
    },
}

/// Complete ordered source relation for one support-state input destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedSupportInputTemplate {
    destination_field_path: FieldPath,
    ordered_sources: Vec<QualifiedSupportSourceTemplate>,
}

impl QualifiedSupportInputTemplate {
    /// Constructs one nonempty ordered alternative-source relation.
    pub fn new(
        destination_field_path: FieldPath,
        ordered_sources: Vec<QualifiedSupportSourceTemplate>,
    ) -> Result<Self> {
        if ordered_sources.is_empty()
            || ordered_sources.iter().collect::<BTreeSet<_>>().len() != ordered_sources.len()
        {
            return Err(ProgramError::Registry(
                "support input sources must be nonempty and unique".to_owned(),
            ));
        }
        Ok(Self {
            destination_field_path,
            ordered_sources,
        })
    }

    /// Returns the exact support-state destination.
    pub const fn destination_field_path(&self) -> &FieldPath {
        &self.destination_field_path
    }

    /// Returns alternative sources in preserved order.
    pub fn ordered_sources(&self) -> &[QualifiedSupportSourceTemplate] {
        &self.ordered_sources
    }
}

/// Complete relative occurrence template contributed by one expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedSupportNodeTemplate {
    local_key: StableAuthorKey,
    state_contract_ref: ContentRef,
    config_source: QualifiedSupportSourceTemplate,
    context_source: Option<QualifiedSupportSourceTemplate>,
    input_bindings: Vec<QualifiedSupportInputTemplate>,
}

impl QualifiedSupportNodeTemplate {
    /// Constructs one completely wired relative support occurrence.
    pub fn new(
        local_key: StableAuthorKey,
        state_contract_ref: ContentRef,
        config_source: QualifiedSupportSourceTemplate,
        context_source: Option<QualifiedSupportSourceTemplate>,
        mut input_bindings: Vec<QualifiedSupportInputTemplate>,
    ) -> Result<Self> {
        input_bindings.sort_by(|left, right| {
            left.destination_field_path
                .cmp(&right.destination_field_path)
        });
        if input_bindings.windows(2).any(|pair| {
            paths_overlap(
                &pair[0].destination_field_path,
                &pair[1].destination_field_path,
            )
        }) {
            return Err(ProgramError::Registry(
                "support input destinations must be unique and nonoverlapping".to_owned(),
            ));
        }
        Ok(Self {
            local_key,
            state_contract_ref,
            config_source,
            context_source,
            input_bindings,
        })
    }

    /// Returns the local key within this one expansion occurrence.
    pub const fn local_key(&self) -> &StableAuthorKey {
        &self.local_key
    }

    /// Returns the support state's semantic contract.
    pub const fn state_contract_ref(&self) -> &ContentRef {
        &self.state_contract_ref
    }

    /// Returns the required configuration source.
    pub const fn config_source(&self) -> &QualifiedSupportSourceTemplate {
        &self.config_source
    }

    /// Returns the optional context source.
    pub const fn context_source(&self) -> Option<&QualifiedSupportSourceTemplate> {
        self.context_source.as_ref()
    }

    /// Returns complete input bindings in destination-path order.
    pub fn input_bindings(&self) -> &[QualifiedSupportInputTemplate] {
        &self.input_bindings
    }
}

/// One reviewed framework expansion policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedFrameworkPolicy {
    policy_ref: ContentRef,
    eligible_state_contracts: BTreeSet<ContentRef>,
    pre_nodes: Vec<QualifiedSupportNodeTemplate>,
    post_nodes: Vec<QualifiedSupportNodeTemplate>,
}

impl QualifiedFrameworkPolicy {
    /// Constructs one explicit framework policy.
    pub fn new(
        policy_ref: ContentRef,
        eligible_state_contracts: impl IntoIterator<Item = ContentRef>,
        pre_nodes: Vec<QualifiedSupportNodeTemplate>,
        post_nodes: Vec<QualifiedSupportNodeTemplate>,
    ) -> Result<Self> {
        let eligible_state_contracts = eligible_state_contracts
            .into_iter()
            .collect::<BTreeSet<_>>();
        if eligible_state_contracts.is_empty() {
            return Err(ProgramError::Registry(
                "framework eligibility cannot be empty".to_owned(),
            ));
        }
        validate_support_template_keys(&pre_nodes, &post_nodes)?;
        Ok(Self {
            policy_ref,
            eligible_state_contracts,
            pre_nodes,
            post_nodes,
        })
    }

    /// Returns the exact policy identity.
    pub const fn policy_ref(&self) -> &ContentRef {
        &self.policy_ref
    }

    /// Returns whether the policy protects this semantic state.
    pub fn applies_to(&self, state_contract_ref: &ContentRef) -> bool {
        self.eligible_state_contracts.contains(state_contract_ref)
    }

    /// Returns complete ordered pre-node templates.
    pub fn pre_nodes(&self) -> &[QualifiedSupportNodeTemplate] {
        &self.pre_nodes
    }

    /// Returns complete ordered post-node templates.
    pub fn post_nodes(&self) -> &[QualifiedSupportNodeTemplate] {
        &self.post_nodes
    }
}

/// One reviewed executor expansion policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedExecutorExpansion {
    /// Executor contributes no support state.
    Leaf {
        /// Exact executor semantic contract.
        executor_contract_ref: ContentRef,
    },
    /// Executor contributes ordered pre/post state chains.
    Chain {
        /// Exact executor semantic contract.
        executor_contract_ref: ContentRef,
        /// Complete ordered pre-node templates.
        pre_nodes: Vec<QualifiedSupportNodeTemplate>,
        /// Complete ordered post-node templates.
        post_nodes: Vec<QualifiedSupportNodeTemplate>,
    },
}

impl QualifiedExecutorExpansion {
    /// Returns the exact executor semantic contract.
    pub const fn executor_contract_ref(&self) -> &ContentRef {
        match self {
            Self::Leaf {
                executor_contract_ref,
            }
            | Self::Chain {
                executor_contract_ref,
                ..
            } => executor_contract_ref,
        }
    }

    /// Returns complete ordered pre-node templates.
    pub fn pre_nodes(&self) -> &[QualifiedSupportNodeTemplate] {
        match self {
            Self::Leaf { .. } => &[],
            Self::Chain { pre_nodes, .. } => pre_nodes,
        }
    }

    /// Returns complete ordered post-node templates.
    pub fn post_nodes(&self) -> &[QualifiedSupportNodeTemplate] {
        match self {
            Self::Leaf { .. } => &[],
            Self::Chain { post_nodes, .. } => post_nodes,
        }
    }
}

fn validate_support_template_keys(
    pre_nodes: &[QualifiedSupportNodeTemplate],
    post_nodes: &[QualifiedSupportNodeTemplate],
) -> Result<()> {
    let keys = pre_nodes
        .iter()
        .chain(post_nodes)
        .map(QualifiedSupportNodeTemplate::local_key)
        .collect::<BTreeSet<_>>();
    if keys.len() != pre_nodes.len() + post_nodes.len() {
        return Err(ProgramError::Registry(
            "support-node template keys must be unique within one expansion".to_owned(),
        ));
    }
    for template in pre_nodes.iter().chain(post_nodes) {
        for source in std::iter::once(template.config_source())
            .chain(template.context_source())
            .chain(
                template
                    .input_bindings()
                    .iter()
                    .flat_map(QualifiedSupportInputTemplate::ordered_sources),
            )
        {
            let producer = match source {
                QualifiedSupportSourceTemplate::NodeOutput { producer, .. }
                | QualifiedSupportSourceTemplate::NodeFact { producer, .. } => producer,
                QualifiedSupportSourceTemplate::BoundaryInput { .. }
                | QualifiedSupportSourceTemplate::Config { .. }
                | QualifiedSupportSourceTemplate::Seed { .. }
                | QualifiedSupportSourceTemplate::Context { .. } => continue,
            };
            if let QualifiedSupportNodeSelector::Support(local_key) = producer {
                if !keys.contains(local_key) {
                    return Err(ProgramError::Registry(
                        "support source names an unknown local support node".to_owned(),
                    ));
                }
                if local_key == template.local_key() {
                    return Err(ProgramError::Registry(
                        "support node cannot consume its own output or fact".to_owned(),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Immutable semantic program definition shared by registry and certifier.
///
/// Construction is private to [`QualifiedProgramRegistryBuilder`]. The hidden
/// projections exist only so the certifier crate can borrow the exact same
/// allocation; they are not an alternate assembly surface.
#[doc(hidden)]
pub struct QualifiedProgramDefinition {
    executable_identity_ref: ContentRef,
    planner_contract_ref: ContentRef,
    planner_implementation_ref: ContentRef,
    planner_descriptor: ComponentImplementationDescriptor,
    journal_protocol_contracts: CertifiedJournalProtocolContracts,
    entry_points: BTreeMap<EntryPointId, Arc<QualifiedEntryPointDefinition>>,
    states: BTreeMap<ContentRef, Arc<QualifiedStateDefinition>>,
    reads: BTreeMap<(ContentRef, StableId), Arc<QualifiedReadDefinition>>,
    effects: BTreeMap<(ContentRef, StableId), Arc<QualifiedEffectDefinition>>,
    framework_policies: BTreeMap<ContentRef, QualifiedFrameworkPolicy>,
    executors: BTreeMap<ContentRef, QualifiedExecutorExpansion>,
    state_manifests: BTreeMap<ContentRef, StateImplementationManifest>,
    capability_manifests: BTreeMap<ContentRef, CapabilityBindingManifest>,
}

impl QualifiedProgramDefinition {
    /// Returns the self-attested whole-executable identity.
    #[doc(hidden)]
    pub const fn executable_identity_ref(&self) -> &ContentRef {
        &self.executable_identity_ref
    }

    /// Returns the exact planner semantic contract.
    #[doc(hidden)]
    pub const fn planner_contract_ref(&self) -> &ContentRef {
        &self.planner_contract_ref
    }

    /// Returns the exact planner implementation identity.
    #[doc(hidden)]
    pub const fn planner_implementation_ref(&self) -> &ContentRef {
        &self.planner_implementation_ref
    }

    /// Returns the persisted planner descriptor.
    #[doc(hidden)]
    pub const fn planner_descriptor(&self) -> &ComponentImplementationDescriptor {
        &self.planner_descriptor
    }

    /// Returns the exact journal-owned protocol contract set.
    #[doc(hidden)]
    pub const fn journal_protocol_contracts(&self) -> &CertifiedJournalProtocolContracts {
        &self.journal_protocol_contracts
    }

    /// Returns all entry-point semantic definitions.
    #[doc(hidden)]
    pub const fn entry_points(
        &self,
    ) -> &BTreeMap<EntryPointId, Arc<QualifiedEntryPointDefinition>> {
        &self.entry_points
    }

    /// Returns all state semantic definitions.
    #[doc(hidden)]
    pub const fn states(&self) -> &BTreeMap<ContentRef, Arc<QualifiedStateDefinition>> {
        &self.states
    }

    /// Returns all audited-read semantic definitions.
    #[doc(hidden)]
    pub const fn reads(&self) -> &BTreeMap<(ContentRef, StableId), Arc<QualifiedReadDefinition>> {
        &self.reads
    }

    /// Returns all effect semantic definitions.
    #[doc(hidden)]
    pub const fn effects(
        &self,
    ) -> &BTreeMap<(ContentRef, StableId), Arc<QualifiedEffectDefinition>> {
        &self.effects
    }

    /// Returns all framework policy definitions.
    #[doc(hidden)]
    pub const fn framework_policies(&self) -> &BTreeMap<ContentRef, QualifiedFrameworkPolicy> {
        &self.framework_policies
    }

    /// Returns all executor expansion definitions.
    #[doc(hidden)]
    pub const fn executors(&self) -> &BTreeMap<ContentRef, QualifiedExecutorExpansion> {
        &self.executors
    }

    /// Returns the exact current state manifests.
    #[doc(hidden)]
    pub const fn state_manifests(&self) -> &BTreeMap<ContentRef, StateImplementationManifest> {
        &self.state_manifests
    }

    /// Returns the exact current capability manifests.
    #[doc(hidden)]
    pub const fn capability_manifests(&self) -> &BTreeMap<ContentRef, CapabilityBindingManifest> {
        &self.capability_manifests
    }
}

/// Closed redaction-safe class of current-candidate certification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateCertificationErrorKind {
    /// No current package entry is available for the recorded operation.
    Unavailable,
    /// Qualified candidate code failed or panicked while executing.
    ExecutionFailed,
    /// Candidate identity, inputs, or outputs contradicted the sealed registry.
    IntegrityFailed,
}

/// Sealed current-candidate certification failure.
///
/// The internal typed cause is intentionally not exposed: callers receive only
/// a stable redaction-safe class and cannot classify failures from diagnostics.
pub struct CandidateCertificationError {
    kind: CandidateCertificationErrorKind,
    _source: Box<CandidateCertificationErrorSource>,
}

#[allow(dead_code)]
enum CandidateCertificationErrorSource {
    Program(ProgramError),
    Specification(mfm_spec::SpecError),
    CallbackPanic,
}

impl CandidateCertificationError {
    /// Returns the closed redaction-safe failure class.
    pub const fn kind(&self) -> CandidateCertificationErrorKind {
        self.kind
    }

    fn unavailable(error: ProgramError) -> Self {
        Self {
            kind: CandidateCertificationErrorKind::Unavailable,
            _source: Box::new(CandidateCertificationErrorSource::Program(error)),
        }
    }

    fn execution(error: ProgramError) -> Self {
        Self {
            kind: CandidateCertificationErrorKind::ExecutionFailed,
            _source: Box::new(CandidateCertificationErrorSource::Program(error)),
        }
    }

    fn execution_spec(error: mfm_spec::SpecError) -> Self {
        Self {
            kind: CandidateCertificationErrorKind::ExecutionFailed,
            _source: Box::new(CandidateCertificationErrorSource::Specification(error)),
        }
    }

    fn execution_panic() -> Self {
        Self {
            kind: CandidateCertificationErrorKind::ExecutionFailed,
            _source: Box::new(CandidateCertificationErrorSource::CallbackPanic),
        }
    }

    fn integrity(error: ProgramError) -> Self {
        Self {
            kind: CandidateCertificationErrorKind::IntegrityFailed,
            _source: Box::new(CandidateCertificationErrorSource::Program(error)),
        }
    }

    fn state_callback(error: CandidateStateCallbackError) -> Self {
        match error {
            CandidateStateCallbackError::Integrity(error) => Self::integrity(error),
            CandidateStateCallbackError::ExecutionPanic => Self::execution_panic(),
        }
    }
}

impl fmt::Debug for CandidateCertificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateCertificationError")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for CandidateCertificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            CandidateCertificationErrorKind::Unavailable => {
                "the selected current candidate is unavailable"
            }
            CandidateCertificationErrorKind::ExecutionFailed => {
                "the selected current candidate failed during execution"
            }
            CandidateCertificationErrorKind::IntegrityFailed => {
                "the selected current candidate failed integrity verification"
            }
        })
    }
}

impl std::error::Error for CandidateCertificationError {}

/// Closed pre-callback compatibility of one recorded plan with a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidatePlanCompatibility {
    /// The candidate can interpret the recorded entry and configured value.
    Comparable,
    /// The candidate cannot interpret the recorded plan without adaptation.
    NotComparable,
}

/// Sealed current-candidate identity qualified by the executable registry.
///
/// This authority is deliberately non-cloneable and non-serializable. Replay
/// may borrow its exact references but cannot mint or persist an equivalent
/// candidate claim.
#[derive(Debug)]
pub struct QualifiedCandidateIdentity {
    registry_seal: Arc<QualifiedRegistrySeal>,
    candidate_executable_identity_ref: ContentRef,
    planning_profile_ref: ContentRef,
    planner_contract_ref: ContentRef,
    planner_implementation_ref: ContentRef,
    state_implementation_manifest_ref: ContentRef,
    capability_binding_manifest_ref: ContentRef,
}

#[derive(Debug)]
struct QualifiedRegistrySeal;

impl QualifiedCandidateIdentity {
    /// Returns the self-attested candidate executable identity.
    pub const fn candidate_executable_identity_ref(&self) -> &ContentRef {
        &self.candidate_executable_identity_ref
    }

    /// Returns the exact selected planning profile.
    pub const fn planning_profile_ref(&self) -> &ContentRef {
        &self.planning_profile_ref
    }

    /// Returns the exact planner semantic contract.
    pub const fn planner_contract_ref(&self) -> &ContentRef {
        &self.planner_contract_ref
    }

    /// Returns the exact planner implementation identity.
    pub const fn planner_implementation_ref(&self) -> &ContentRef {
        &self.planner_implementation_ref
    }

    /// Returns the exact qualified state implementation manifest.
    pub const fn state_implementation_manifest_ref(&self) -> &ContentRef {
        &self.state_implementation_manifest_ref
    }

    /// Returns the exact qualified capability binding manifest.
    pub const fn capability_binding_manifest_ref(&self) -> &ContentRef {
        &self.capability_binding_manifest_ref
    }
}

/// Sealed proof that the current registry exactly matches one admitted run.
///
/// This value is for exact resume only. It is non-cloneable, non-serializable,
/// and cannot be constructed from cross-version candidate selection.
#[derive(Debug)]
pub struct QualifiedAdmittedProgram {
    candidate: QualifiedCandidateIdentity,
    recorded_operation_id: StableId,
}

impl QualifiedAdmittedProgram {
    /// Returns the exact current candidate proven equal to the admission.
    pub const fn candidate_identity(&self) -> &QualifiedCandidateIdentity {
        &self.candidate
    }

    /// Returns the operation identity fixed by the verified admission.
    pub const fn recorded_operation_id(&self) -> &StableId {
        &self.recorded_operation_id
    }
}

/// Replay-safe callback projection sealed to one exact current candidate.
///
/// This borrowed value exposes deterministic authors and reducers only. It
/// carries no capability, executor, store, or append authority.
pub struct QualifiedCandidateCallbacks<'a> {
    registry: &'a QualifiedProgramRegistry,
    _candidate: &'a QualifiedCandidateIdentity,
    entry: &'a QualifiedEntryPointRegistration,
    recorded_operation_id: &'a StableId,
    state_manifest: &'a StateImplementationManifest,
    capability_manifest: &'a CapabilityBindingManifest,
}

impl<'a> QualifiedCandidateCallbacks<'a> {
    /// Returns the exact current entry-point contract selected by operation.
    pub fn entry_point(&self) -> &EntryPointContract {
        self.entry.entry_point()
    }

    /// Returns the current configured-root contract for compatibility checks.
    pub fn configured_value_contract(&self) -> &RetainedValueContract {
        self.entry
            .input_contract()
            .configured_value_contract()
            .root_contract()
    }

    /// Classifies whether the candidate can interpret one recorded plan.
    ///
    /// Entry-point version and planner implementation identity deliberately do
    /// not affect semantic comparability. This pure check must run before any
    /// candidate author, certifier, or state callback.
    pub fn classify_plan_compatibility(
        &self,
        recorded_entry: &EntryPointContract,
        recorded_configured_contract: &RetainedValueContract,
    ) -> CandidatePlanCompatibility {
        classify_candidate_plan_compatibility(
            self.entry.entry_point(),
            self.configured_value_contract(),
            recorded_entry,
            recorded_configured_contract,
        )
    }

    /// Reauthors current topology from one store-verified historical value.
    pub fn author_recorded_configured(
        &self,
        recorded: &'a VerifiedRecordedConfiguredValue,
    ) -> std::result::Result<CanonicalAuthoredProgram, CandidateCertificationError> {
        let replay = ReplayConfiguredValue::from_verified(
            recorded,
            self.recorded_operation_id,
            self.entry
                .input_contract()
                .configured_value_contract()
                .root_contract(),
        )?;
        self.entry.candidate_author_configured_value(
            replay.authoring_value(),
            &self.registry.admitted_support,
        )
    }

    /// Resolves one replay-safe state callback projection.
    pub fn state(
        &self,
        state_contract_ref: &ContentRef,
    ) -> Option<QualifiedCandidateStateCallbacks<'a>> {
        self.registry
            .state(state_contract_ref)
            .map(|entry| QualifiedCandidateStateCallbacks { entry })
    }

    /// Reproduces certification through the sole qualified planner callback.
    pub fn certify(
        &self,
        authored_program: &CanonicalAuthoredProgram,
    ) -> std::result::Result<CertifiedAdmissionArtifacts, CandidateCertificationError> {
        if authored_program.entry_point_operation_id() != self.recorded_operation_id {
            return Err(CandidateCertificationError::integrity(
                ProgramError::Registry(
                    "candidate authored program differs from its selected operation".to_owned(),
                ),
            ));
        }
        validate_certification_manifests(
            authored_program,
            self.state_manifest,
            self.capability_manifest,
            &self.registry.states,
            &self.registry.reads,
            &self.registry.effects,
            &self.registry.admitted_support,
        )
        .map_err(CandidateCertificationError::integrity)?;
        let artifacts = catch_unwind(AssertUnwindSafe(|| {
            self.registry
                .certification_callback
                .certify(self.entry.entry_point(), authored_program)
        }))
        .map_err(|_| CandidateCertificationError::execution_panic())?
        .map_err(CandidateCertificationError::execution_spec)?;
        self.registry
            .validate_current_certification_artifacts(
                self.entry,
                authored_program,
                self.state_manifest,
                self.capability_manifest,
                &artifacts,
            )
            .map_err(CandidateCertificationError::integrity)?;
        Ok(artifacts)
    }

    /// Returns the exact registry-owned current state manifest.
    pub const fn state_manifest(&self) -> &StateImplementationManifest {
        self.state_manifest
    }

    /// Returns the exact registry-owned current capability manifest.
    pub const fn capability_manifest(&self) -> &CapabilityBindingManifest {
        self.capability_manifest
    }
}

fn classify_candidate_plan_compatibility(
    candidate_entry: &EntryPointContract,
    candidate_configured_contract: &RetainedValueContract,
    recorded_entry: &EntryPointContract,
    recorded_configured_contract: &RetainedValueContract,
) -> CandidatePlanCompatibility {
    let recorded_profile = recorded_entry.planning_profile();
    let candidate_profile = candidate_entry.planning_profile();
    if recorded_entry.entry_point_operation_id() == candidate_entry.entry_point_operation_id()
        && recorded_entry.input_schema_id() == candidate_entry.input_schema_id()
        && recorded_entry.public_output_schema_id() == candidate_entry.public_output_schema_id()
        && recorded_profile.planner_contract_ref() == candidate_profile.planner_contract_ref()
        && recorded_profile.canonical_profile_parameters()
            == candidate_profile.canonical_profile_parameters()
        && recorded_profile.framework_policy_refs() == candidate_profile.framework_policy_refs()
        && recorded_configured_contract == candidate_configured_contract
    {
        CandidatePlanCompatibility::Comparable
    } else {
        CandidatePlanCompatibility::NotComparable
    }
}

/// Replay-safe deterministic callbacks for one exact qualified state.
pub struct QualifiedCandidateStateCallbacks<'a> {
    entry: &'a QualifiedStateEntry,
}

impl QualifiedCandidateStateCallbacks<'_> {
    /// Returns the complete semantic state contract.
    pub fn contract(&self) -> &QualifiedStateContract {
        self.entry.contract()
    }

    /// Returns the closed execution kind.
    pub fn kind(&self) -> crate::StateExecutionKind {
        self.entry.callbacks().kind()
    }

    /// Authors one deterministic read/effect request.
    pub fn author_request(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
    ) -> std::result::Result<crate::QualifiedAuthoredRequest, CandidateCertificationError> {
        self.entry
            .callbacks
            .candidate_author_request(frame)
            .map_err(CandidateCertificationError::state_callback)
    }

    /// Strictly reconstructs one already committed request.
    pub fn decode_request(
        &self,
        retained: &crate::VerifiedValueMaterial,
    ) -> std::result::Result<crate::QualifiedAuthoredRequest, CandidateCertificationError> {
        self.entry
            .callbacks
            .candidate_decode_request(retained)
            .map_err(CandidateCertificationError::state_callback)
    }

    /// Invokes one pure-state settlement callback.
    pub fn settle_pure(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
    ) -> std::result::Result<crate::QualifiedSettlement, CandidateCertificationError> {
        self.entry
            .callbacks
            .candidate_settle_pure(frame)
            .map_err(CandidateCertificationError::state_callback)
    }

    /// Invokes one deterministic read-observation reducer.
    pub fn settle_read(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
        observation: &crate::VerifiedReadOutcome,
    ) -> std::result::Result<crate::QualifiedEvidenceVerdict, CandidateCertificationError> {
        self.entry
            .callbacks
            .candidate_settle_read(frame, observation)
            .map_err(CandidateCertificationError::state_callback)
    }

    /// Invokes one deterministic terminal-effect reducer.
    pub fn settle_effect(
        &self,
        frame: &crate::VerifiedStateFrameMaterial,
        terminal: crate::VerifiedTerminalEffectView<'_>,
    ) -> std::result::Result<crate::QualifiedEvidenceVerdict, CandidateCertificationError> {
        self.entry
            .callbacks
            .candidate_settle_effect(frame, terminal)
            .map_err(CandidateCertificationError::state_callback)
    }
}

/// Builder binding one static package catalog to a real executable identity.
pub struct QualifiedProgramRegistryBuilder {
    executable_identity_ref: ContentRef,
    planner: QualifiedPlannerRegistration,
    admitted_support: AdmittedSupportGraph,
    journal_protocol_contracts: CertifiedJournalProtocolContracts,
    entry_points: BTreeMap<EntryPointId, QualifiedEntryPointRegistration>,
    states: BTreeMap<ContentRef, QualifiedStateEntry>,
    reads: BTreeMap<(ContentRef, StableId), QualifiedReadEntry>,
    effects: BTreeMap<(ContentRef, StableId), QualifiedEffectEntry>,
    framework_policies: BTreeMap<ContentRef, QualifiedFrameworkPolicy>,
    executors: BTreeMap<ContentRef, QualifiedExecutorExpansion>,
}

impl QualifiedProgramRegistryBuilder {
    /// Adds one package-owned entry-point registration.
    pub fn register_entry_point(
        &mut self,
        registration: QualifiedEntryPointRegistration,
    ) -> Result<&mut Self> {
        let id = registration.entry_point().entry_point_id().clone();
        if self.entry_points.insert(id, registration).is_some() {
            return Err(ProgramError::Registry(
                "duplicate entry-point registration".to_owned(),
            ));
        }
        Ok(self)
    }

    /// Adds one typed state callback set and descriptor graph.
    pub fn register_state<S: State>(
        &mut self,
        registration: QualifiedStateRegistration<S>,
    ) -> Result<&mut Self> {
        let key = registration.contract().state_contract_ref().clone();
        let entry = QualifiedStateEntry {
            definition: Arc::clone(&registration.definition),
            callbacks: Arc::clone(&registration.callbacks),
        };
        if self.states.insert(key, entry).is_some() {
            return Err(ProgramError::Registry(
                "duplicate state registration".to_owned(),
            ));
        }
        Ok(self)
    }

    /// Adds one sole qualified audited-read entry.
    pub fn register_read(&mut self, entry: QualifiedReadEntry) -> Result<&mut Self> {
        validate_read_support(&self.admitted_support, &entry)?;
        let key = (
            entry.binding_ref().clone(),
            entry.operation().operation_id().clone(),
        );
        if self.reads.insert(key, entry).is_some() {
            return Err(ProgramError::Registry(
                "duplicate qualified read operation".to_owned(),
            ));
        }
        Ok(self)
    }

    /// Adds one sole qualified effect-executor entry.
    pub fn register_effect(&mut self, entry: QualifiedEffectEntry) -> Result<&mut Self> {
        validate_effect_support(&self.admitted_support, &entry)?;
        let key = (entry.binding_ref().clone(), entry.operation_id().clone());
        if self.effects.insert(key, entry).is_some() {
            return Err(ProgramError::Registry(
                "duplicate qualified effect operation".to_owned(),
            ));
        }
        Ok(self)
    }

    /// Adds one reviewed framework policy.
    pub fn register_framework_policy(
        &mut self,
        policy: QualifiedFrameworkPolicy,
    ) -> Result<&mut Self> {
        let key = policy.policy_ref().clone();
        if self.framework_policies.insert(key, policy).is_some() {
            return Err(ProgramError::Registry(
                "duplicate framework policy".to_owned(),
            ));
        }
        Ok(self)
    }

    /// Adds one reviewed executor expansion.
    pub fn register_executor(
        &mut self,
        expansion: QualifiedExecutorExpansion,
    ) -> Result<&mut Self> {
        let key = expansion.executor_contract_ref().clone();
        if self.executors.insert(key, expansion).is_some() {
            return Err(ProgramError::Registry(
                "duplicate executor expansion".to_owned(),
            ));
        }
        Ok(self)
    }

    /// Validates closure and seals the registry.
    pub fn build(self) -> Result<QualifiedProgramRegistry> {
        if self.entry_points.is_empty() || self.states.is_empty() {
            return Err(ProgramError::Registry(
                "qualified registry requires an entry point and a state".to_owned(),
            ));
        }
        for contract in [
            self.journal_protocol_contracts.input_manifest_contract(),
            self.journal_protocol_contracts
                .fact_claim_envelope_contract(),
            self.journal_protocol_contracts
                .frozen_read_intent_contract(),
        ] {
            require_admitted_support_ref(&self.admitted_support, contract.evidence_contract_ref())?;
        }
        validate_component_descriptors(&self.planner, &self.states, &self.reads, &self.effects)?;
        for registration in self.entry_points.values() {
            let profile = registration.entry_point().planning_profile();
            if profile.planner_contract_ref() != self.planner.planner_contract_ref()
                || profile.planner_implementation_ref()
                    != self.planner.component_implementation_ref()
            {
                return Err(ProgramError::Registry(
                    "entry point selects a different qualified planner".to_owned(),
                ));
            }
            if profile
                .framework_policy_refs()
                .iter()
                .any(|reference| !self.framework_policies.contains_key(reference))
            {
                return Err(ProgramError::Registry(
                    "entry point selects an unregistered framework policy".to_owned(),
                ));
            }
        }
        for policy in self.framework_policies.values() {
            for state_contract_ref in &policy.eligible_state_contracts {
                if !self.states.contains_key(state_contract_ref) {
                    return Err(ProgramError::Registry(
                        "framework policy names an unregistered eligible state".to_owned(),
                    ));
                }
            }
            for template in policy.pre_nodes().iter().chain(policy.post_nodes()) {
                if !self.states.contains_key(template.state_contract_ref()) {
                    return Err(ProgramError::Registry(
                        "framework policy names an unregistered support state".to_owned(),
                    ));
                }
            }
        }
        let mut used_reads = BTreeSet::new();
        let mut used_effects = BTreeSet::new();
        for state in self.states.values() {
            validate_registered_state_execution(
                &self.admitted_support,
                state,
                &self.reads,
                &self.effects,
                &self.executors,
            )?;
            match state.contract().execution() {
                CertifiedStateExecution::Read {
                    capability_operation_id,
                    capability_binding_ref,
                    ..
                } => {
                    if capability_operation_id.as_str() != mfm_store::FACT_SELECTION_OPERATION_ID {
                        used_reads.insert((
                            capability_binding_ref.clone(),
                            capability_operation_id.clone(),
                        ));
                    }
                }
                CertifiedStateExecution::Effect {
                    executor_operation_id,
                    executor_binding_ref,
                    ..
                } => {
                    used_effects
                        .insert((executor_binding_ref.clone(), executor_operation_id.clone()));
                }
                CertifiedStateExecution::Pure => {}
            }
        }
        if used_reads != self.reads.keys().cloned().collect()
            || used_effects != self.effects.keys().cloned().collect()
        {
            return Err(ProgramError::Registry(
                "qualified operation entries are not an exact state-operation closure".to_owned(),
            ));
        }
        for executor_contract_ref in self.executors.keys() {
            validate_executor_expansion_closure(
                &self.states,
                &self.effects,
                &self.executors,
                executor_contract_ref,
                &mut Vec::new(),
            )?;
        }
        let (state_manifests, capability_manifests) =
            load_candidate_manifests(&self.admitted_support, &self.states)?;
        if state_manifests.len() != 1 || capability_manifests.len() != 1 {
            return Err(ProgramError::Registry(
                "each registered entry point requires one sole current manifest pair".to_owned(),
            ));
        }
        let QualifiedPlannerRegistration {
            planner_contract_ref,
            component_implementation_ref: planner_implementation_ref,
            descriptor: planner_descriptor,
            certification_factory,
        } = self.planner;
        let definition = Arc::new(QualifiedProgramDefinition {
            executable_identity_ref: self.executable_identity_ref,
            planner_contract_ref,
            planner_implementation_ref,
            planner_descriptor,
            journal_protocol_contracts: self.journal_protocol_contracts,
            entry_points: self
                .entry_points
                .iter()
                .map(|(key, entry)| (key.clone(), Arc::clone(&entry.definition)))
                .collect(),
            states: self
                .states
                .iter()
                .map(|(key, state)| (key.clone(), Arc::clone(&state.definition)))
                .collect(),
            reads: self
                .reads
                .iter()
                .map(|(key, read)| (key.clone(), Arc::clone(&read.definition)))
                .collect(),
            effects: self
                .effects
                .iter()
                .map(|(key, effect)| (key.clone(), Arc::clone(&effect.definition)))
                .collect(),
            framework_policies: self.framework_policies,
            executors: self.executors,
            state_manifests,
            capability_manifests,
        });
        let certification_callback = certification_factory.create(Arc::clone(&definition))?;
        Ok(QualifiedProgramRegistry {
            registry_seal: Arc::new(QualifiedRegistrySeal),
            definition,
            certification_callback,
            admitted_support: self.admitted_support,
            entry_points: self.entry_points,
            states: self.states,
            reads: self.reads,
            effects: self.effects,
        })
    }
}

fn validate_component_descriptors(
    planner: &QualifiedPlannerRegistration,
    states: &BTreeMap<ContentRef, QualifiedStateEntry>,
    reads: &BTreeMap<(ContentRef, StableId), QualifiedReadEntry>,
    effects: &BTreeMap<(ContentRef, StableId), QualifiedEffectEntry>,
) -> Result<()> {
    let qualification_ref = planner.descriptor().qualification_ref();
    let mut descriptors_by_implementation = BTreeMap::new();
    let descriptors = std::iter::once(planner.descriptor())
        .chain(
            states
                .values()
                .map(QualifiedStateEntry::component_descriptor),
        )
        .chain(reads.values().map(QualifiedReadEntry::component_descriptor))
        .chain(
            effects
                .values()
                .map(QualifiedEffectEntry::component_descriptor),
        );
    for descriptor in descriptors {
        if descriptor.qualification_ref() != qualification_ref {
            return Err(ProgramError::Registry(
                "package components select different qualification references".to_owned(),
            ));
        }
        let implementation_ref = descriptor.content_ref()?;
        if descriptors_by_implementation
            .insert(implementation_ref, descriptor.clone())
            .is_some_and(|previous| previous != *descriptor)
        {
            return Err(ProgramError::Registry(
                "one component implementation identifies different descriptors".to_owned(),
            ));
        }
    }

    let mut descriptor_tuples = BTreeSet::new();
    for descriptor in descriptors_by_implementation.values() {
        let tuple = (
            descriptor.component_kind(),
            descriptor.semantic_contract_ref().clone(),
            descriptor.callback_surface_ref().clone(),
        );
        if !descriptor_tuples.insert(tuple) {
            return Err(ProgramError::Registry(
                "package component descriptors are ambiguous".to_owned(),
            ));
        }
    }
    Ok(())
}

fn load_candidate_manifests(
    support: &AdmittedSupportGraph,
    states: &BTreeMap<ContentRef, QualifiedStateEntry>,
) -> Result<(
    BTreeMap<ContentRef, StateImplementationManifest>,
    BTreeMap<ContentRef, CapabilityBindingManifest>,
)> {
    let state_contract = StateImplementationManifest::retained_contract()?;
    let capability_contract = CapabilityBindingManifest::retained_contract()?;
    let expected_states = states
        .values()
        .map(|state| {
            (
                state.contract().state_contract_ref().clone(),
                state.component_implementation_ref().clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut expected_bindings = BTreeMap::new();
    for state in states.values() {
        let Some((operation_id, binding_ref)) = state.contract().execution().operation_binding()
        else {
            continue;
        };
        if expected_bindings
            .insert(operation_id.clone(), binding_ref.clone())
            .is_some_and(|previous| previous != *binding_ref)
        {
            return Err(ProgramError::Registry(
                "qualified states bind one operation to multiple capabilities".to_owned(),
            ));
        }
    }

    let mut state_manifests = BTreeMap::new();
    let mut capability_manifests = BTreeMap::new();
    for member in support.members().values() {
        let fields = member
            .value_ref()
            .fields()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        if &fields.schema_id == state_contract.schema_id() {
            verify_retained_material(member.value_ref(), member.bytes(), &state_contract)?;
            let manifest = StateImplementationManifest::from_canonical_json(member.bytes())?;
            let actual = manifest
                .entries()
                .iter()
                .map(|entry| {
                    (
                        entry.state_contract_ref.clone(),
                        entry.component_implementation_ref.clone(),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            if actual != expected_states {
                return Err(ProgramError::Registry(
                    "admitted state manifest is not the exact current registry closure".to_owned(),
                ));
            }
            let reference = manifest.content_ref()?;
            if state_manifests.insert(reference, manifest).is_some() {
                return Err(ProgramError::Registry(
                    "admitted support repeats one current state manifest".to_owned(),
                ));
            }
        } else if &fields.schema_id == capability_contract.schema_id() {
            verify_retained_material(member.value_ref(), member.bytes(), &capability_contract)?;
            let manifest = CapabilityBindingManifest::from_canonical_json(member.bytes())?;
            let mut actual = BTreeMap::new();
            for entry in manifest.entries() {
                if entry.operation_id.as_str() == mfm_store::FACT_SELECTION_OPERATION_ID
                    && !is_reserved_fact_selection_binding(support, entry)?
                {
                    return Err(ProgramError::Registry(
                        "reserved fact-selection manifest entry is invalid".to_owned(),
                    ));
                }
                actual.insert(entry.operation_id.clone(), entry.binding_ref.clone());
            }
            if actual != expected_bindings {
                return Err(ProgramError::Registry(
                    "admitted capability manifest is not the exact current registry closure"
                        .to_owned(),
                ));
            }
            let reference = manifest.content_ref()?;
            if capability_manifests.insert(reference, manifest).is_some() {
                return Err(ProgramError::Registry(
                    "admitted support repeats one current capability manifest".to_owned(),
                ));
            }
        }
    }
    if state_manifests.is_empty() || capability_manifests.is_empty() {
        return Err(ProgramError::Registry(
            "admitted support omits current state or capability manifests".to_owned(),
        ));
    }
    Ok((state_manifests, capability_manifests))
}

fn admitted_support_member<'a>(
    support: &'a AdmittedSupportGraph,
    reference: &ContentRef,
) -> Result<Option<&'a AdmittedSupportMember>> {
    for member in support.members().values() {
        let fields = member
            .value_ref()
            .fields()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        if &fields.schema_id == reference.schema_id()
            && &fields.content_digest == reference.content_digest()
        {
            return Ok(Some(member));
        }
    }
    Ok(None)
}

fn require_admitted_support_ref(
    support: &AdmittedSupportGraph,
    reference: &ContentRef,
) -> Result<()> {
    if admitted_support_member(support, reference)?.is_none() {
        return Err(ProgramError::Registry(
            "qualified operation references material outside admitted support".to_owned(),
        ));
    }
    Ok(())
}

fn validate_read_support(support: &AdmittedSupportGraph, entry: &QualifiedReadEntry) -> Result<()> {
    let binding_member =
        admitted_support_member(support, entry.binding_ref())?.ok_or_else(|| {
            ProgramError::Registry("read binding is absent from admitted support".to_owned())
        })?;
    let decoded = ReadCapabilityBinding::strict_decode(binding_member.bytes())
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    if &decoded != entry.binding() {
        return Err(ProgramError::Registry(
            "admitted read binding bytes differ from the qualified entry".to_owned(),
        ));
    }
    let fields = entry
        .binding()
        .fields()
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    for reference in [
        &fields.capability_contract_ref,
        &fields.admitted_implementation_ref,
        &fields.safe_classifier_contract_ref,
        &fields.safe_failure_contract_ref,
        &fields.reviewed_source_scope_ref,
        &fields.routing_catalog_ref,
        entry.operation().operation_contract_ref(),
    ]
    .into_iter()
    .chain(entry.operation().allowed_routing_generation_refs())
    {
        require_admitted_support_ref(support, reference)?;
    }
    Ok(())
}

fn validate_effect_support(
    support: &AdmittedSupportGraph,
    entry: &QualifiedEffectEntry,
) -> Result<()> {
    let contract_ref = entry
        .binding()
        .contract()
        .reference()
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    let deployment_ref = entry
        .binding()
        .binding()
        .executor_deployment_ref()
        .as_content_ref();
    for reference in [
        entry.binding_ref(),
        &contract_ref,
        entry.binding().binding().admitted_implementation_ref(),
        deployment_ref,
        entry.operation_contract().typed_boundary_contract_ref(),
        entry.operation_contract().expansion_contract_ref(),
    ] {
        require_admitted_support_ref(support, reference)?;
    }
    if let Some(ownership) = entry.binding().resource_ownership() {
        let ownership_ref = ownership
            .reference()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        require_admitted_support_ref(support, ownership_ref.as_content_ref())?;
    }
    Ok(())
}

fn validate_registered_state_execution(
    support: &AdmittedSupportGraph,
    state: &QualifiedStateEntry,
    reads: &BTreeMap<(ContentRef, StableId), QualifiedReadEntry>,
    effects: &BTreeMap<(ContentRef, StableId), QualifiedEffectEntry>,
    executors: &BTreeMap<ContentRef, QualifiedExecutorExpansion>,
) -> Result<()> {
    match state.contract().execution() {
        CertifiedStateExecution::Pure => {
            if state.callbacks().operation().is_some() {
                return Err(ProgramError::Registry(
                    "pure state exposes an external operation".to_owned(),
                ));
            }
        }
        CertifiedStateExecution::Read {
            capability_operation_id,
            capability_binding_ref,
            request_contract,
            returned_contract,
            safe_failure_contract,
        } => {
            if capability_operation_id.as_str() == mfm_store::FACT_SELECTION_OPERATION_ID {
                return validate_reserved_fact_selection_state(
                    support,
                    state,
                    capability_operation_id,
                    capability_binding_ref,
                    request_contract,
                    returned_contract,
                );
            }
            let entry = reads
                .get(&(
                    capability_binding_ref.clone(),
                    capability_operation_id.clone(),
                ))
                .ok_or_else(|| {
                    ProgramError::Registry(
                        "read state selects an unregistered qualified read".to_owned(),
                    )
                })?;
            let callback_operation = state.callbacks().operation().ok_or_else(|| {
                ProgramError::Registry("read state has no callback operation".to_owned())
            })?;
            if callback_operation.operation_contract_ref()
                != entry.operation().operation_contract_ref()
                || state.callbacks().request_type() != entry.request_type()
                || state.callbacks().observation_type() != entry.returned_type()
                || state.callbacks().diagnostic_type() != entry.diagnostic_type()
                || request_contract != entry.operation().request_contract()
                || returned_contract != entry.operation().returned_contract()
                || safe_failure_contract != entry.operation().safe_failure_contract()
            {
                return Err(ProgramError::Registry(
                    "read state differs from its sole qualified read entry".to_owned(),
                ));
            }
        }
        CertifiedStateExecution::Effect {
            executor_operation_id,
            executor_binding_ref,
            request_contract,
            ensure_result_contract,
            terminal_evidence_contract,
            ..
        } => {
            let entry = effects
                .get(&(executor_binding_ref.clone(), executor_operation_id.clone()))
                .ok_or_else(|| {
                    ProgramError::Registry(
                        "effect state selects an unregistered qualified executor".to_owned(),
                    )
                })?;
            let callback_operation = state.callbacks().operation().ok_or_else(|| {
                ProgramError::Registry("effect state has no callback operation".to_owned())
            })?;
            let executor_contract_ref = entry
                .binding()
                .contract()
                .reference()
                .map_err(|error| ProgramError::Registry(error.to_string()))?;
            let retained = entry.binding().contract().retained_closure_contract();
            if callback_operation.operation_contract_ref() != &executor_contract_ref
                || state.callbacks().request_type() != entry.request_type()
                || request_contract != entry.binding().contract().semantic_request_contract()
                || ensure_result_contract != retained.ensure_result_contract()
                || terminal_evidence_contract != retained.terminal_evidence_contract()
                || !executors.contains_key(&executor_contract_ref)
            {
                return Err(ProgramError::Registry(
                    "effect state differs from its sole qualified executor entry".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_reserved_fact_selection_state(
    support: &AdmittedSupportGraph,
    state: &QualifiedStateEntry,
    operation_id: &StableId,
    binding_ref: &ContentRef,
    request_contract: &RetainedValueContract,
    returned_contract: &RetainedValueContract,
) -> Result<()> {
    let scan =
        reserved_fact_selection_contract(support, operation_id, binding_ref)?.ok_or_else(|| {
            ProgramError::Registry(
                "reserved fact-selection state has a non-reserved operation".to_owned(),
            )
        })?;
    let callback_operation = state.callbacks().operation().ok_or_else(|| {
        ProgramError::Registry("fact-selection state has no callback operation".to_owned())
    })?;
    if callback_operation.operation_id() != operation_id
        || callback_operation.binding_ref() != binding_ref
        || callback_operation.operation_contract_ref() != &scan.capability_contract_ref
        || request_contract != &scan.contract.request_contract
        || returned_contract != &scan.contract.response_contract
    {
        return Err(ProgramError::Registry(
            "fact-selection state differs from its reserved admitted binding".to_owned(),
        ));
    }
    Ok(())
}

fn validate_executor_expansion_closure(
    states: &BTreeMap<ContentRef, QualifiedStateEntry>,
    effects: &BTreeMap<(ContentRef, StableId), QualifiedEffectEntry>,
    executors: &BTreeMap<ContentRef, QualifiedExecutorExpansion>,
    executor_contract_ref: &ContentRef,
    stack: &mut Vec<ContentRef>,
) -> Result<()> {
    if stack.contains(executor_contract_ref) {
        return Err(ProgramError::Registry(
            "executor expansion graph contains a cycle".to_owned(),
        ));
    }
    let expansion = executors.get(executor_contract_ref).ok_or_else(|| {
        ProgramError::Registry("executor expansion references an unregistered executor".to_owned())
    })?;
    if matches!(expansion, QualifiedExecutorExpansion::Leaf { .. }) {
        return Ok(());
    }

    stack.push(executor_contract_ref.clone());
    for template in expansion.pre_nodes().iter().chain(expansion.post_nodes()) {
        let state = states.get(template.state_contract_ref()).ok_or_else(|| {
            ProgramError::Registry(
                "executor expansion names an unregistered support state".to_owned(),
            )
        })?;
        if !matches!(
            state.contract().execution(),
            CertifiedStateExecution::Effect { .. }
        ) {
            continue;
        }
        let CertifiedStateExecution::Effect {
            executor_operation_id,
            executor_binding_ref,
            ..
        } = state.contract().execution()
        else {
            unreachable!("effect state was matched above");
        };
        let nested = effects
            .get(&(executor_binding_ref.clone(), executor_operation_id.clone()))
            .ok_or_else(|| {
                ProgramError::Registry(
                    "effect support state selects an unregistered executor entry".to_owned(),
                )
            })?;
        let nested_contract_ref = nested
            .binding()
            .contract()
            .reference()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        validate_executor_expansion_closure(
            states,
            effects,
            executors,
            &nested_contract_ref,
            stack,
        )?;
        if !matches!(
            executors.get(&nested_contract_ref),
            Some(QualifiedExecutorExpansion::Leaf { .. })
        ) {
            return Err(ProgramError::Registry(
                "executor support state selects a non-leaf executor".to_owned(),
            ));
        }
    }
    stack.pop();
    Ok(())
}

/// Sole qualified program/callback registry used by certification and runtime.
pub struct QualifiedProgramRegistry {
    registry_seal: Arc<QualifiedRegistrySeal>,
    definition: Arc<QualifiedProgramDefinition>,
    certification_callback: Arc<dyn QualifiedCertificationCallback>,
    admitted_support: AdmittedSupportGraph,
    entry_points: BTreeMap<EntryPointId, QualifiedEntryPointRegistration>,
    states: BTreeMap<ContentRef, QualifiedStateEntry>,
    reads: BTreeMap<(ContentRef, StableId), QualifiedReadEntry>,
    effects: BTreeMap<(ContentRef, StableId), QualifiedEffectEntry>,
}

impl QualifiedProgramRegistry {
    /// Starts binding one static package catalog to a real executable identity.
    pub fn builder(
        executable_identity_ref: ContentRef,
        planner: QualifiedPlannerRegistration,
        admitted_support: AdmittedSupportGraph,
        journal_protocol_contracts: CertifiedJournalProtocolContracts,
    ) -> QualifiedProgramRegistryBuilder {
        QualifiedProgramRegistryBuilder {
            executable_identity_ref,
            planner,
            admitted_support,
            journal_protocol_contracts,
            entry_points: BTreeMap::new(),
            states: BTreeMap::new(),
            reads: BTreeMap::new(),
            effects: BTreeMap::new(),
            framework_policies: BTreeMap::new(),
            executors: BTreeMap::new(),
        }
    }

    /// Returns the self-attested whole-executable identity.
    pub fn executable_identity_ref(&self) -> &ContentRef {
        self.definition.executable_identity_ref()
    }

    /// Returns the exact current store-admitted support closure.
    pub const fn admitted_support(&self) -> &AdmittedSupportGraph {
        &self.admitted_support
    }

    /// Returns the exact journal-owned runtime protocol value contracts.
    pub fn journal_protocol_contracts(&self) -> &CertifiedJournalProtocolContracts {
        self.definition.journal_protocol_contracts()
    }

    /// Returns package-owned entry points in identifier order.
    pub const fn entry_points(&self) -> &BTreeMap<EntryPointId, QualifiedEntryPointRegistration> {
        &self.entry_points
    }

    /// Resolves one versioned entry point.
    pub fn entry_point(&self, id: &EntryPointId) -> Option<&QualifiedEntryPointRegistration> {
        self.entry_points.get(id)
    }

    /// Authors one registered entry point from verified configured material.
    pub fn author_verified_parts(
        &self,
        entry_point_id: &EntryPointId,
        binding: &ConfiguredValueBinding,
        value_ref: &ValueRef,
        bytes: &[u8],
    ) -> Result<CanonicalAuthoredProgram> {
        self.entry_point(entry_point_id)
            .ok_or_else(|| {
                ProgramError::Registry(
                    "entry point is absent from the qualified registry".to_owned(),
                )
            })?
            .author_verified_parts(binding, value_ref, bytes, &self.admitted_support)
    }

    /// Authors and certifies one verified configured entry-point value.
    pub fn author_and_certify_verified_parts(
        &self,
        entry_point_id: &EntryPointId,
        binding: &ConfiguredValueBinding,
        value_ref: &ValueRef,
        bytes: &[u8],
    ) -> Result<CertifiedAdmissionArtifacts> {
        let registration = self.entry_point(entry_point_id).ok_or_else(|| {
            ProgramError::Registry("entry point is absent from the qualified registry".to_owned())
        })?;
        let authored_program = registration.author_verified_parts(
            binding,
            value_ref,
            bytes,
            &self.admitted_support,
        )?;
        self.certify_current(registration, &authored_program)
    }

    /// Revalidates an app-authored admission artifact set against this exact current registry.
    ///
    /// Runtime calls this immediately before store preparation so an artifact set authored by a
    /// foreign registry, executable, entry point, planner, or manifest set cannot be appended.
    pub fn validate_current_admission_artifacts(
        &self,
        entry_point_id: &EntryPointId,
        artifacts: &CertifiedAdmissionArtifacts,
    ) -> Result<()> {
        let registration = self.entry_point(entry_point_id).ok_or_else(|| {
            ProgramError::Registry("entry point is absent from the qualified registry".to_owned())
        })?;
        let state_manifest =
            sole_registry_value(self.definition.state_manifests(), "current state manifest")?;
        let capability_manifest = sole_registry_value(
            self.definition.capability_manifests(),
            "current capability manifest",
        )?;
        self.validate_current_certification_artifacts(
            registration,
            artifacts.authored_program(),
            state_manifest,
            capability_manifest,
            artifacts,
        )
    }

    fn certify_current(
        &self,
        registration: &QualifiedEntryPointRegistration,
        authored_program: &CanonicalAuthoredProgram,
    ) -> Result<CertifiedAdmissionArtifacts> {
        let entry_point = registration.entry_point();
        if authored_program.entry_point_operation_id() != entry_point.entry_point_operation_id()
            || entry_point.planning_profile().planner_contract_ref()
                != self.definition.planner_contract_ref()
            || entry_point.planning_profile().planner_implementation_ref()
                != self.definition.planner_implementation_ref()
        {
            return Err(ProgramError::Registry(
                "certification inputs differ from the registered entry-point tuple".to_owned(),
            ));
        }
        let state_manifest =
            sole_registry_value(self.definition.state_manifests(), "current state manifest")?;
        let capability_manifest = sole_registry_value(
            self.definition.capability_manifests(),
            "current capability manifest",
        )?;
        validate_certification_manifests(
            authored_program,
            state_manifest,
            capability_manifest,
            &self.states,
            &self.reads,
            &self.effects,
            &self.admitted_support,
        )?;

        let artifacts = catch_unwind(AssertUnwindSafe(|| {
            self.certification_callback
                .certify(entry_point, authored_program)
        }))
        .map_err(|_| ProgramError::Registry("qualified planner certification panicked".to_owned()))?
        .map_err(|error| {
            ProgramError::Registry(format!("qualified planner certification failed: {error}"))
        })?;
        self.validate_current_certification_artifacts(
            registration,
            authored_program,
            state_manifest,
            capability_manifest,
            &artifacts,
        )?;
        Ok(artifacts)
    }

    fn validate_current_certification_artifacts(
        &self,
        registration: &QualifiedEntryPointRegistration,
        authored_program: &CanonicalAuthoredProgram,
        state_manifest: &StateImplementationManifest,
        capability_manifest: &CapabilityBindingManifest,
        artifacts: &CertifiedAdmissionArtifacts,
    ) -> Result<()> {
        let entry_point = registration.entry_point();
        if artifacts.entry_point() != entry_point
            || artifacts.authored_program() != authored_program
            || artifacts.state_manifest() != state_manifest
            || artifacts.capability_manifest() != capability_manifest
            || artifacts.expanded_spec().journal_protocol_contracts()
                != self.journal_protocol_contracts()
            || artifacts
                .expanded_spec()
                .public_output_contract()
                .value_contract()
                != registration.public_output_contract()
        {
            return Err(ProgramError::Registry(
                "qualified planner returned artifacts outside the registered contract".to_owned(),
            ));
        }
        self.validate_current_graph_and_manifests(
            registration,
            artifacts.expanded_spec(),
            artifacts.state_manifest(),
            artifacts.capability_manifest(),
        )?;
        validate_certified_public_outputs(self, artifacts.expanded_spec())?;
        validate_certificate_proof_closure(self, artifacts)?;
        Ok(())
    }

    /// Returns all state registrations by semantic contract.
    pub const fn states(&self) -> &BTreeMap<ContentRef, QualifiedStateEntry> {
        &self.states
    }

    /// Resolves one semantic state contract.
    pub fn state(&self, state_contract_ref: &ContentRef) -> Option<&QualifiedStateEntry> {
        self.states.get(state_contract_ref)
    }

    /// Resolves one exact audited-read entry.
    pub fn read(
        &self,
        binding_ref: &ContentRef,
        operation_id: &StableId,
    ) -> Option<&QualifiedReadEntry> {
        self.reads.get(&(binding_ref.clone(), operation_id.clone()))
    }

    /// Resolves one exact effect-executor entry.
    pub fn effect(
        &self,
        binding_ref: &ContentRef,
        operation_id: &StableId,
    ) -> Option<&QualifiedEffectEntry> {
        self.effects
            .get(&(binding_ref.clone(), operation_id.clone()))
    }

    /// Borrows replay-safe deterministic callbacks for one exact candidate.
    pub fn candidate_callbacks<'a>(
        &'a self,
        candidate: &'a QualifiedCandidateIdentity,
        recorded_operation_id: &'a StableId,
    ) -> std::result::Result<QualifiedCandidateCallbacks<'a>, CandidateCertificationError> {
        if !Arc::ptr_eq(&self.registry_seal, &candidate.registry_seal) {
            return Err(CandidateCertificationError::integrity(
                ProgramError::Registry(
                    "candidate callback identity carries a foreign registry seal".to_owned(),
                ),
            ));
        }
        let entry = self.current_entry_for_operation(recorded_operation_id)?;
        let (state_manifest_ref, state_manifest, capability_manifest_ref, capability_manifest) =
            self.current_candidate_manifests()?;
        if candidate.candidate_executable_identity_ref() != self.executable_identity_ref()
            || candidate.planning_profile_ref() != entry.entry_point().planning_profile_ref()
            || candidate.planner_contract_ref() != self.definition.planner_contract_ref()
            || candidate.planner_implementation_ref()
                != self.definition.planner_implementation_ref()
            || candidate.state_implementation_manifest_ref() != state_manifest_ref
            || candidate.capability_binding_manifest_ref() != capability_manifest_ref
        {
            return Err(CandidateCertificationError::integrity(
                ProgramError::Registry(
                    "candidate callback identity differs from the qualified registry seal"
                        .to_owned(),
                ),
            ));
        }
        Ok(QualifiedCandidateCallbacks {
            registry: self,
            _candidate: candidate,
            entry,
            recorded_operation_id,
            state_manifest,
            capability_manifest,
        })
    }

    /// Selects the unique current candidate for one recorded operation.
    pub fn select_current_candidate(
        &self,
        recorded_operation_id: &StableId,
    ) -> std::result::Result<QualifiedCandidateIdentity, CandidateCertificationError> {
        let entry = self.current_entry_for_operation(recorded_operation_id)?;
        let (state_manifest_ref, _, capability_manifest_ref, _) =
            self.current_candidate_manifests()?;
        Ok(QualifiedCandidateIdentity {
            registry_seal: Arc::clone(&self.registry_seal),
            candidate_executable_identity_ref: self.executable_identity_ref().clone(),
            planning_profile_ref: entry.entry_point().planning_profile_ref().clone(),
            planner_contract_ref: self.definition.planner_contract_ref().clone(),
            planner_implementation_ref: self.definition.planner_implementation_ref().clone(),
            state_implementation_manifest_ref: state_manifest_ref.clone(),
            capability_binding_manifest_ref: capability_manifest_ref.clone(),
        })
    }

    /// Qualifies the current package only when it exactly matches one admitted run.
    pub fn qualify_admitted_run(
        &self,
        view: &VerifiedRunView,
    ) -> std::result::Result<QualifiedAdmittedProgram, CandidateCertificationError> {
        let admission = view.admission().fields().map_err(|error| {
            CandidateCertificationError::integrity(ProgramError::Registry(error.to_string()))
        })?;
        let candidate = self.select_current_candidate(&admission.entry_point_operation_id)?;
        let entry = self.current_entry_for_operation(&admission.entry_point_operation_id)?;
        let operation_contract_ref = entry.entry_point().content_ref().map_err(|error| {
            CandidateCertificationError::integrity(ProgramError::Spec(error.to_string()))
        })?;
        if candidate.candidate_executable_identity_ref() != &admission.executable_identity_ref
            || operation_contract_ref != admission.operation_contract_ref
            || candidate.state_implementation_manifest_ref()
                != &admission.state_implementation_manifest_ref
            || candidate.capability_binding_manifest_ref()
                != &admission.capability_binding_manifest_ref
        {
            return Err(CandidateCertificationError::integrity(
                ProgramError::Registry(
                    "admitted program identity differs from the current registry".to_owned(),
                ),
            ));
        }
        self.validate_current_graph_and_manifests(
            entry,
            view.certified_spec(),
            view.state_implementation_manifest(),
            view.capability_binding_manifest(),
        )
        .map_err(CandidateCertificationError::integrity)?;
        Ok(QualifiedAdmittedProgram {
            candidate,
            recorded_operation_id: admission.entry_point_operation_id,
        })
    }

    fn validate_current_graph_and_manifests(
        &self,
        entry: &QualifiedEntryPointRegistration,
        spec: &ExpandedCertifiedSpec,
        state_manifest: &StateImplementationManifest,
        capability_manifest: &CapabilityBindingManifest,
    ) -> Result<()> {
        if spec.planning_profile_ref() != entry.entry_point().planning_profile_ref() {
            return Err(ProgramError::Registry(
                "candidate spec selects a different planning profile".to_owned(),
            ));
        }
        let state_manifest_ref = state_manifest.content_ref()?;
        let capability_manifest_ref = capability_manifest.content_ref()?;
        if self.definition.state_manifests().get(&state_manifest_ref) != Some(state_manifest)
            || self
                .definition
                .capability_manifests()
                .get(&capability_manifest_ref)
                != Some(capability_manifest)
        {
            return Err(ProgramError::Registry(
                "candidate manifests are not the exact current admitted registry values".to_owned(),
            ));
        }

        let mut expected_states = BTreeMap::<ContentRef, ContentRef>::new();
        let mut expected_bindings = BTreeMap::<StableId, ContentRef>::new();
        for node in spec.nodes() {
            let state = self.state(node.state_contract_ref()).ok_or_else(|| {
                ProgramError::Registry(
                    "candidate spec uses an unregistered state contract".to_owned(),
                )
            })?;
            if !qualified_state_matches_node(state.contract(), node) {
                return Err(ProgramError::Registry(
                    "candidate node differs from its qualified state projection".to_owned(),
                ));
            }
            let implementation = state.component_implementation_ref().clone();
            if expected_states
                .insert(node.state_contract_ref().clone(), implementation.clone())
                .is_some_and(|previous| previous != implementation)
            {
                return Err(ProgramError::Registry(
                    "candidate state contract selects multiple implementations".to_owned(),
                ));
            }
            if let Some((operation_id, binding_ref)) = node.execution().operation_binding() {
                if expected_bindings
                    .insert(operation_id.clone(), binding_ref.clone())
                    .is_some_and(|previous| previous != *binding_ref)
                {
                    return Err(ProgramError::Registry(
                        "candidate operation selects multiple bindings".to_owned(),
                    ));
                }
            }
        }

        let actual_states = state_manifest
            .entries()
            .iter()
            .map(|entry| {
                (
                    entry.state_contract_ref.clone(),
                    entry.component_implementation_ref.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let actual_bindings = capability_manifest
            .entries()
            .iter()
            .map(|entry| (entry.operation_id.clone(), entry.binding_ref.clone()))
            .collect::<BTreeMap<_, _>>();
        if expected_states != actual_states || expected_bindings != actual_bindings {
            return Err(ProgramError::Registry(
                "candidate manifests differ from exact qualified graph closure".to_owned(),
            ));
        }
        Ok(())
    }

    fn current_entry_for_operation(
        &self,
        recorded_operation_id: &StableId,
    ) -> std::result::Result<&QualifiedEntryPointRegistration, CandidateCertificationError> {
        let mut entries = self.entry_points.values().filter(|entry| {
            entry.entry_point().entry_point_operation_id() == recorded_operation_id
        });
        let entry = entries.next().ok_or_else(|| {
            CandidateCertificationError::unavailable(ProgramError::Registry(
                "current package has no entry for the recorded operation".to_owned(),
            ))
        })?;
        if entries.next().is_some() {
            return Err(CandidateCertificationError::integrity(
                ProgramError::Registry(
                    "current package has ambiguous entries for one operation".to_owned(),
                ),
            ));
        }
        Ok(entry)
    }

    #[allow(clippy::type_complexity)]
    fn current_candidate_manifests(
        &self,
    ) -> std::result::Result<
        (
            &ContentRef,
            &StateImplementationManifest,
            &ContentRef,
            &CapabilityBindingManifest,
        ),
        CandidateCertificationError,
    > {
        let mut states = self.definition.state_manifests().iter();
        let Some((state_ref, state)) = states.next() else {
            return Err(CandidateCertificationError::unavailable(
                ProgramError::Registry("current state manifest is unavailable".to_owned()),
            ));
        };
        let mut capabilities = self.definition.capability_manifests().iter();
        let Some((capability_ref, capability)) = capabilities.next() else {
            return Err(CandidateCertificationError::unavailable(
                ProgramError::Registry("current capability manifest is unavailable".to_owned()),
            ));
        };
        if states.next().is_some() || capabilities.next().is_some() {
            return Err(CandidateCertificationError::integrity(
                ProgramError::Registry("current manifest selection is ambiguous".to_owned()),
            ));
        }
        Ok((state_ref, state, capability_ref, capability))
    }

    /// Resolves one framework policy.
    pub fn framework_policy(&self, policy_ref: &ContentRef) -> Option<&QualifiedFrameworkPolicy> {
        self.definition.framework_policies().get(policy_ref)
    }

    /// Resolves one executor expansion.
    pub fn executor(&self, contract_ref: &ContentRef) -> Option<&QualifiedExecutorExpansion> {
        self.definition.executors().get(contract_ref)
    }
}

fn sole_registry_value<'a, K, V>(values: &'a BTreeMap<K, V>, description: &str) -> Result<&'a V> {
    let mut values = values.values();
    let value = values
        .next()
        .ok_or_else(|| ProgramError::Registry(format!("{description} is unavailable")))?;
    if values.next().is_some() {
        return Err(ProgramError::Registry(format!(
            "{description} is ambiguous"
        )));
    }
    Ok(value)
}

fn validate_certification_manifests(
    authored_program: &CanonicalAuthoredProgram,
    state_manifest: &StateImplementationManifest,
    capability_manifest: &CapabilityBindingManifest,
    states: &BTreeMap<ContentRef, QualifiedStateEntry>,
    reads: &BTreeMap<(ContentRef, StableId), QualifiedReadEntry>,
    effects: &BTreeMap<(ContentRef, StableId), QualifiedEffectEntry>,
    admitted_support: &AdmittedSupportGraph,
) -> Result<()> {
    let state_entries = state_manifest
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.state_contract_ref.clone(),
                entry.component_implementation_ref.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (state_contract_ref, component_implementation_ref) in &state_entries {
        let registered = states.get(state_contract_ref).ok_or_else(|| {
            ProgramError::Registry(
                "state manifest names a state outside the qualified registry".to_owned(),
            )
        })?;
        if registered.component_implementation_ref() != component_implementation_ref {
            return Err(ProgramError::Registry(
                "state manifest selects a different qualified implementation".to_owned(),
            ));
        }
    }
    if authored_program
        .nodes()
        .iter()
        .any(|node| !state_entries.contains_key(node.state_contract_ref()))
    {
        return Err(ProgramError::Registry(
            "state manifest omits an authored state contract".to_owned(),
        ));
    }

    for entry in capability_manifest.entries() {
        let key = (entry.binding_ref.clone(), entry.operation_id.clone());
        if !reads.contains_key(&key)
            && !effects.contains_key(&key)
            && !is_reserved_fact_selection_binding(admitted_support, entry)?
        {
            return Err(ProgramError::Registry(
                "capability manifest names an operation outside the qualified registry".to_owned(),
            ));
        }
    }
    Ok(())
}

fn is_reserved_fact_selection_binding(
    support: &AdmittedSupportGraph,
    entry: &mfm_spec::CapabilityBindingManifestEntry,
) -> Result<bool> {
    reserved_fact_selection_contract(support, &entry.operation_id, &entry.binding_ref)
        .map(|contract| contract.is_some())
}

struct QualifiedReservedFactSelectionContract {
    capability_contract_ref: ContentRef,
    contract: mfm_journal::FactSelectionScanContractFields,
}

fn reserved_fact_selection_contract(
    support: &AdmittedSupportGraph,
    operation_id: &StableId,
    binding_ref: &ContentRef,
) -> Result<Option<QualifiedReservedFactSelectionContract>> {
    if operation_id.as_str() != mfm_store::FACT_SELECTION_OPERATION_ID {
        return Ok(None);
    }
    let binding_member = admitted_support_member(support, binding_ref)?.ok_or_else(|| {
        ProgramError::Registry(
            "reserved fact-selection binding is absent from admitted support".to_owned(),
        )
    })?;
    let binding = ReadCapabilityBinding::strict_decode(binding_member.bytes())
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    if binding
        .content_ref()
        .map_err(|error| ProgramError::Registry(error.to_string()))?
        != *binding_ref
    {
        return Err(ProgramError::Registry(
            "reserved fact-selection binding reference is not exact".to_owned(),
        ));
    }
    let fields = binding
        .fields()
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    for reference in [
        &fields.admitted_implementation_ref,
        &fields.safe_classifier_contract_ref,
        &fields.safe_failure_contract_ref,
        &fields.reviewed_source_scope_ref,
        &fields.routing_catalog_ref,
    ] {
        require_admitted_support_ref(support, reference)?;
    }
    let contract_member = admitted_support_member(support, &fields.capability_contract_ref)?
        .ok_or_else(|| {
            ProgramError::Registry(
                "reserved fact-selection contract is absent from admitted support".to_owned(),
            )
        })?;
    let contract = FactSelectionScanContract::strict_decode(contract_member.bytes())
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    let contract_fields = contract
        .fields()
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    if contract
        .content_ref()
        .map_err(|error| ProgramError::Registry(error.to_string()))?
        != fields.capability_contract_ref
        || contract_fields.capability_operation_id != *operation_id
    {
        return Err(ProgramError::Registry(
            "reserved fact-selection contract differs from its manifest binding".to_owned(),
        ));
    }
    Ok(Some(QualifiedReservedFactSelectionContract {
        capability_contract_ref: fields.capability_contract_ref,
        contract: contract_fields,
    }))
}

fn validate_certified_public_outputs(
    registry: &QualifiedProgramRegistry,
    spec: &ExpandedCertifiedSpec,
) -> Result<()> {
    for binding in spec.public_output_contract().bindings() {
        let node = spec
            .nodes()
            .iter()
            .find(|node| node.node_id() == binding.source_node_id())
            .ok_or_else(|| {
                ProgramError::Registry(
                    "certified public output names an unknown producer".to_owned(),
                )
            })?;
        let state = registry.state(node.state_contract_ref()).ok_or_else(|| {
            ProgramError::Registry(
                "certified public output names an unregistered producer state".to_owned(),
            )
        })?;
        let selected = state
            .contract()
            .output_source(binding.output_ordinal())
            .and_then(|source| source.selected_contract(binding.source_field_path()));
        if selected != Some(binding.value_contract()) {
            return Err(ProgramError::Registry(
                "certified public output differs from its qualified source projection".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_certificate_proof_closure(
    registry: &QualifiedProgramRegistry,
    artifacts: &CertifiedAdmissionArtifacts,
) -> Result<()> {
    let entry_point_ref = artifacts.entry_point().content_ref()?;
    let authored_program_ref = artifacts.authored_program().content_ref()?;
    let spec_ref = artifacts.expanded_spec().content_ref()?;
    let state_manifest_ref = artifacts.state_manifest().content_ref()?;
    let capability_manifest_ref = artifacts.capability_manifest().content_ref()?;
    let mut expected = BTreeSet::new();
    let mut insert =
        |kind: &str, subject_ref: ContentRef, evidence_ref: ContentRef| -> Result<()> {
            expected.insert((
                StableId::new(kind).map_err(|error| ProgramError::Registry(error.to_string()))?,
                subject_ref,
                evidence_ref,
            ));
            Ok(())
        };
    insert(
        "entry-point-profile",
        entry_point_ref,
        artifacts.entry_point().planning_profile_ref().clone(),
    )?;
    insert("authored-program", spec_ref.clone(), authored_program_ref)?;
    insert(
        "planner-implementation",
        registry.definition.planner_contract_ref().clone(),
        registry.definition.planner_implementation_ref().clone(),
    )?;
    insert("state-manifest", spec_ref.clone(), state_manifest_ref)?;
    insert(
        "capability-manifest",
        spec_ref.clone(),
        capability_manifest_ref.clone(),
    )?;
    insert("expanded-spec", spec_ref.clone(), spec_ref)?;
    for policy_ref in artifacts
        .entry_point()
        .planning_profile()
        .framework_policy_refs()
    {
        insert(
            "framework-policy",
            policy_ref.clone(),
            artifacts.entry_point().planning_profile_ref().clone(),
        )?;
    }
    for entry in artifacts.state_manifest().entries() {
        insert(
            "state-implementation",
            entry.state_contract_ref.clone(),
            entry.component_implementation_ref.clone(),
        )?;
    }
    for binding_ref in artifacts
        .capability_manifest()
        .entries()
        .iter()
        .map(|entry| entry.binding_ref.clone())
        .collect::<BTreeSet<_>>()
    {
        insert(
            "capability-binding",
            binding_ref,
            capability_manifest_ref.clone(),
        )?;
    }

    let actual = artifacts
        .certificate()
        .proof_closure()
        .iter()
        .map(|proof| {
            (
                proof.proof_kind.clone(),
                proof.subject_ref.clone(),
                proof.evidence_ref.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    if actual != expected || artifacts.certificate().proof_closure().len() != expected.len() {
        return Err(ProgramError::Registry(
            "certificate proof closure differs from the qualified registry".to_owned(),
        ));
    }
    Ok(())
}

fn validate_state_type_contract<S: State>(contract: &QualifiedStateContract) -> Result<()> {
    let config_schema = S::Config::config_schema_id()?;
    if &config_schema != contract.config_contract().schema_id() {
        return Err(ProgramError::Registry(
            "state config type differs from its qualified contract".to_owned(),
        ));
    }
    let context_schema = S::Context::context_schema_id()?;
    if context_schema.as_ref()
        != contract
            .context_contract()
            .map(RetainedValueContract::schema_id)
    {
        return Err(ProgramError::Registry(
            "state context type differs from its qualified contract".to_owned(),
        ));
    }
    let input_schema = <S::Input as mfm_values::StateInput>::input_schema_id()
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    if contract.input_contract().schema_id() != &input_schema {
        return Err(ProgramError::Registry(
            "state input type differs from its qualified root contract".to_owned(),
        ));
    }
    let expected_destinations = <S::Input as mfm_values::StateInput>::input_destination_paths()
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    if expected_destinations
        != contract
            .input_destinations()
            .iter()
            .map(QualifiedInputContract::destination_field_path)
            .cloned()
            .collect::<Vec<_>>()
    {
        return Err(ProgramError::Registry(
            "state input destinations differ from the named StateInput contract".to_owned(),
        ));
    }
    if let Some(failure_contract) = contract.settlement_contract().typed_failure_contract() {
        let failure_schema = <S::Failure as mfm_values::MfmValue>::schema_id()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        let failure_semantic = <S::Failure as mfm_values::MfmValue>::semantic_id()
            .map_err(|error| ProgramError::Registry(error.to_string()))?;
        if failure_contract.schema_id() != &failure_schema
            || failure_contract.semantic_type_id() != &failure_semantic
        {
            return Err(ProgramError::Registry(
                "state failure type differs from its qualified contract".to_owned(),
            ));
        }
    }
    Ok(())
}

fn qualified_state_matches_node(
    contract: &QualifiedStateContract,
    node: &CertifiedNodeContract,
) -> bool {
    contract.state_contract_ref() == node.state_contract_ref()
        && contract.config_contract() == node.config_binding().value_contract()
        && contract.context_contract()
            == node
                .context_binding()
                .map(mfm_spec::CertifiedFrameBinding::value_contract)
        && contract.input_contract() == node.input_contract()
        && contract.execution() == node.execution()
        && contract.settlement_contract() == node.settlement_contract()
        && contract.input_destinations().len() == node.input_bindings().len()
        && contract
            .input_destinations()
            .iter()
            .zip(node.input_bindings())
            .all(|(qualified, certified)| {
                qualified.destination_field_path() == certified.destination_field_path()
                    && qualified.destination() == certified.destination()
                    && qualified.value_contract() == certified.value_contract()
            })
}

fn validate_state_execution<S: State>(
    contract: &QualifiedStateContract,
    execution: &StateExecution<S>,
) -> Result<()> {
    let matches = match (execution, contract.execution()) {
        (StateExecution::Pure(_), CertifiedStateExecution::Pure) => true,
        (
            StateExecution::Read(callbacks),
            CertifiedStateExecution::Read {
                capability_operation_id,
                capability_binding_ref,
                request_contract,
                returned_contract,
                safe_failure_contract,
            },
        ) => {
            callbacks.operation().operation_id() == capability_operation_id
                && callbacks.operation().binding_ref() == capability_binding_ref
                && callbacks.request_codec().value_contract() == request_contract
                && callbacks.observation_codec().value_contract() == returned_contract
                && callbacks.diagnostic_codec().value_contract() == safe_failure_contract
        }
        (
            StateExecution::Effect(callbacks),
            CertifiedStateExecution::Effect {
                executor_operation_id,
                executor_binding_ref,
                request_contract,
                ensure_result_contract,
                terminal_evidence_contract,
                domain_result_contract,
            },
        ) => {
            callbacks.operation().operation_id() == executor_operation_id
                && callbacks.operation().binding_ref() == executor_binding_ref
                && callbacks.request_codec().value_contract() == request_contract
                && callbacks.ensure_result_contract() == ensure_result_contract
                && callbacks.terminal_evidence_contract() == terminal_evidence_contract
                && callbacks.domain_result_contract() == domain_result_contract
        }
        _ => false,
    };
    if !matches {
        return Err(ProgramError::Registry(
            "state execution callbacks differ from the complete qualified contract".to_owned(),
        ));
    }
    Ok(())
}

fn verify_configured_material(
    value_ref: &ValueRef,
    canonical: &PlainCanonicalJsonBytes,
    expected: &RetainedValueContract,
) -> Result<()> {
    let fields = value_ref
        .fields()
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    let exact_ref = boundary_content_ref(expected.schema_id().clone(), canonical)?;
    let byte_length = u64::try_from(canonical.as_bytes().len())
        .map_err(|_| ProgramError::Registry("configured value exceeds u64 bytes".to_owned()))?;
    if &fields.schema_id != expected.schema_id()
        || &fields.semantic_type_id != expected.semantic_type_id()
        || &fields.role != expected.role()
        || fields.media_type != expected.media_type()
        || &fields.evidence_contract_ref != expected.evidence_contract_ref()
        || fields.byte_length != byte_length
        || &fields.content_digest != exact_ref.content_digest()
    {
        return Err(ProgramError::Registry(
            "configured value differs from its exact retained contract".to_owned(),
        ));
    }
    Ok(())
}

fn verify_retained_material(
    value_ref: &ValueRef,
    bytes: &[u8],
    expected: &RetainedValueContract,
) -> Result<()> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| ProgramError::Registry(error.to_string()))?;
    verify_configured_material(value_ref, &canonical, expected)
}

#[cfg(test)]
mod registry_tests;
