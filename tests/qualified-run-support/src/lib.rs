#![warn(missing_docs)]
//! Dev-only genuine qualified-run fixtures shared by backend conformance tests.
//!
//! This package assembles through the same qualified program, certifier, and
//! sealed store boundaries as production. It contains no runtime, application,
//! storage-adapter, or concrete-backend dependency.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContractV1};
use mfm_certify::CompositeCertificationFactory;
use mfm_ids::{
    AppendRequestId, ContentRef, DigestAlgorithm, EntryPointId, FieldPath, InvocationIdentity,
    ObjectEvidenceDigest, SchemaId, SemanticTypeId, StableId, TenantScopeId,
};
use mfm_journal::v1::{
    ArtifactIdPreimage, ConfiguredValueBinding, ConfiguredValueKey, ObjectEvidencePreimage,
    ProducerBinding, ValueRef,
};
use mfm_program::{
    decode_boundary, encode_boundary, state_input_value_contract, AuthoredProgramBuilder,
    CanonicalCodec, CompositePlannerSurface, FactSet, NoBoundaryValue, NoContext, ProgramError,
    QualifiedAuthoringInputs, QualifiedEntryPointInputContract, QualifiedEntryPointRegistration,
    QualifiedInputContract, QualifiedOutputProjector, QualifiedOutputSourceContract,
    QualifiedPlannerRegistration, QualifiedProgramRegistry, QualifiedProgramRegistryBuilder,
    QualifiedSettlementCodecs, QualifiedSourceContract, QualifiedStateContract,
    QualifiedStateRegistration, Settlement, State, StateBindings, StateConfig, StateExecution,
    StateFrame,
};
use mfm_program_derive::{MfmValue, StateInput};
use mfm_spec::{
    CanonicalJsonValue, CapabilityBindingManifest, CertifiedInputDestination,
    CertifiedJournalProtocolContracts, CertifiedOutputSlot, CertifiedSettlementContract,
    CertifiedStateExecution, ComponentImplementationDescriptor, ComponentKind, EntryPointContract,
    PlanningProfile, RetainedValueContract, StateImplementationManifest,
    StateImplementationManifestEntry,
};
use mfm_store::{
    AdmissionMaterial, AdmissionSourceBackend, AdmissionSourceStore, Admit, AdmitRun,
    ConfiguredValueBackend, ConfiguredValueStore, ProposedAdmissionInput, QualifiedSupportGraph,
    QualifiedSupportMember, RunAccessAuthority, RunAccessAuthorityIssuer, RunJournalBackend,
    RunJournalStore, StoreError, StoreIdentity, SupportBackend, SupportStore,
};
use mfm_values::component_object_evidence_contract_canonical;
use serde::{Deserialize, Serialize};

const DEFAULT_OPERATION_ID: &str = "mfm.fixture/qualified-run";
const EXECUTABLE_IDENTITY_PATH: &str = "executable.identity";
const STATE_MANIFEST_PATH: &str = "manifests.state_implementation";
const CAPABILITY_MANIFEST_PATH: &str = "manifests.capability_binding";
const OBJECT_EVIDENCE_PATH: &str = "qualification.object_evidence_contract";
const QUALIFICATION_PATH: &str = "qualification.descriptor";
const PLANNER_SEMANTIC_PATH: &str = "planner.semantic_contract";
const PLANNER_CALLBACK_PATH: &str = "planner.callback_surface";
const PLANNER_IMPLEMENTATION_PATH: &str = "components.planner";
const STATE_SEMANTIC_PATH: &str = "state.semantic_contract";
const STATE_CALLBACK_PATH: &str = "state.callback_surface";
const STATE_IMPLEMENTATION_PATH: &str = "components.state";
const SECOND_STATE_SEMANTIC_PATH: &str = "state.second.semantic_contract";
const SECOND_STATE_CALLBACK_PATH: &str = "state.second.callback_surface";
const SECOND_STATE_IMPLEMENTATION_PATH: &str = "components.state_second";

static FIRST_CANDIDATE_CALLBACK_COUNTS: [AtomicUsize; 256] = [const { AtomicUsize::new(0) }; 256];
static SECOND_CANDIDATE_CALLBACK_COUNTS: [AtomicUsize; 256] = [const { AtomicUsize::new(0) }; 256];

/// Failure while constructing deterministic genuine fixture material.
#[derive(Debug, thiserror::Error)]
pub enum QualifiedRunFixtureError {
    /// A checked string identity was invalid.
    #[error(transparent)]
    CheckedIdentity(#[from] mfm_ids::CheckedStringError),
    /// A checked identity was invalid.
    #[error(transparent)]
    Identity(#[from] mfm_ids::IdentityError),
    /// A frozen canonical contract operation failed.
    #[error(transparent)]
    Canonical(#[from] mfm_canonical::RecoverabilityError),
    /// A journal-owned value could not be constructed.
    #[error(transparent)]
    Journal(#[from] mfm_journal::v1::JournalError),
    /// A qualified program contract was invalid.
    #[error(transparent)]
    Program(#[from] ProgramError),
    /// A certified specification contract was invalid.
    #[error(transparent)]
    Specification(#[from] mfm_spec::SpecError),
    /// A sealed store proposal was invalid.
    #[error(transparent)]
    Store(Box<StoreError>),
    /// A retained-value contract was invalid.
    #[error(transparent)]
    Value(#[from] mfm_values::ValueError),
    /// Deterministic fixture material could not be represented.
    #[error("invalid qualified-run fixture material: {0}")]
    Invalid(String),
}

/// Failure while resolving and preparing a genuine run on one backend.
#[derive(Debug, thiserror::Error)]
pub enum PrepareQualifiedRunError<E> {
    /// The backend rejected one sealed support or admission operation.
    #[error("qualified-run backend operation failed")]
    Backend(#[source] Box<E>),
    /// Deterministic fixture construction or store preparation failed.
    #[error(transparent)]
    Fixture(#[from] QualifiedRunFixtureError),
}

impl From<StoreError> for QualifiedRunFixtureError {
    fn from(error: StoreError) -> Self {
        Self::Store(Box::new(error))
    }
}

impl<E> From<ProgramError> for PrepareQualifiedRunError<E> {
    fn from(error: ProgramError) -> Self {
        Self::Fixture(error.into())
    }
}

impl<E> From<mfm_spec::SpecError> for PrepareQualifiedRunError<E> {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::Fixture(error.into())
    }
}

impl<E> From<StoreError> for PrepareQualifiedRunError<E> {
    fn from(error: StoreError) -> Self {
        Self::Fixture(error.into())
    }
}

/// Per-node callback invocations observed for one fixture discriminator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CandidateCallbackCounts {
    /// Invocations of the divergent candidate's first state.
    pub first_state: usize,
    /// Invocations of the divergent candidate's second state.
    pub second_state: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureMode {
    Single,
    IntegrityFailingStateCallback,
    TwoNodeChain,
    TwoNodeDivergentCandidate,
}

impl FixtureMode {
    const fn is_two_node(self) -> bool {
        matches!(self, Self::TwoNodeChain | Self::TwoNodeDivergentCandidate)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureConfig {
    value: u64,
    panic_on_apply: bool,
}

impl StateConfig for FixtureConfig {
    fn config_schema_id() -> mfm_program::Result<SchemaId> {
        primitive_canonical_schema().map_err(|error| ProgramError::Spec(error.to_string()))
    }

    fn validate_config(&self) -> mfm_program::Result<()> {
        let _ = self;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntegrityFailingFixtureConfig {
    expected_text: String,
}

impl StateConfig for IntegrityFailingFixtureConfig {
    fn config_schema_id() -> mfm_program::Result<SchemaId> {
        primitive_canonical_schema().map_err(|error| ProgramError::Spec(error.to_string()))
    }

    fn validate_config(&self) -> mfm_program::Result<()> {
        let _ = self;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.test.qualified_run_input")]
struct FixtureInput {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test",
    name = "qualified-run-output",
    version = "1",
    schema = "mfm.test.qualified_run_output"
)]
struct FixtureOutput {
    value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.test.qualified_run_chained_input")]
struct FixtureChainedInput {
    upstream: FixtureOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test",
    name = "qualified-run-failure",
    version = "1",
    schema = "mfm.test.qualified_run_failure"
)]
struct FixtureFailure {
    code: String,
}

struct FixtureState;
struct IntegrityFailingFixtureState;
struct FixtureSecondState;

impl State for FixtureState {
    type Config = FixtureConfig;
    type Context = NoContext;
    type Input = FixtureInput;
    type Output = FixtureOutput;
    type Failure = FixtureFailure;
    type Request = NoBoundaryValue;
    type Observation = NoBoundaryValue;
    type SafeDiagnostic = NoBoundaryValue;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        fixture_state_contract_ref().map_err(|error| ProgramError::Registry(error.to_string()))
    }
}

impl State for IntegrityFailingFixtureState {
    type Config = IntegrityFailingFixtureConfig;
    type Context = NoContext;
    type Input = FixtureInput;
    type Output = FixtureOutput;
    type Failure = FixtureFailure;
    type Request = NoBoundaryValue;
    type Observation = NoBoundaryValue;
    type SafeDiagnostic = NoBoundaryValue;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        fixture_state_contract_ref().map_err(|error| ProgramError::Registry(error.to_string()))
    }
}

impl State for FixtureSecondState {
    type Config = FixtureConfig;
    type Context = NoContext;
    type Input = FixtureChainedInput;
    type Output = FixtureOutput;
    type Failure = FixtureFailure;
    type Request = NoBoundaryValue;
    type Observation = NoBoundaryValue;
    type SafeDiagnostic = NoBoundaryValue;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        fixture_second_state_contract_ref()
            .map_err(|error| ProgramError::Registry(error.to_string()))
    }
}

fn apply_fixture_state(frame: StateFrame<'_, FixtureState>) -> Settlement<FixtureState> {
    if frame.config().value().panic_on_apply {
        panic!("qualified fixture state callback requested deterministic failure");
    }
    Settlement::succeeded(
        FixtureOutput {
            value: frame.config().value().value,
        },
        FactSet::empty(),
    )
}

fn apply_integrity_failing_fixture_state(
    _frame: StateFrame<'_, IntegrityFailingFixtureState>,
) -> Settlement<IntegrityFailingFixtureState> {
    Settlement::succeeded(FixtureOutput { value: 0 }, FactSet::empty())
}

fn apply_second_fixture_state(
    frame: StateFrame<'_, FixtureSecondState>,
) -> Settlement<FixtureSecondState> {
    if frame.config().value().panic_on_apply {
        panic!("qualified fixture state callback requested deterministic failure");
    }
    Settlement::succeeded(frame.input().value().upstream.clone(), FactSet::empty())
}

fn apply_divergent_first_candidate(
    frame: StateFrame<'_, FixtureState>,
) -> Settlement<FixtureState> {
    let config = frame.config().value();
    FIRST_CANDIDATE_CALLBACK_COUNTS[counter_index(config)].fetch_add(1, Ordering::SeqCst);
    Settlement::succeeded(
        FixtureOutput {
            value: config.value.saturating_add(1),
        },
        FactSet::empty(),
    )
}

fn apply_divergent_second_candidate(
    frame: StateFrame<'_, FixtureSecondState>,
) -> Settlement<FixtureSecondState> {
    let config = frame.config().value();
    SECOND_CANDIDATE_CALLBACK_COUNTS[counter_index(config)].fetch_add(1, Ordering::SeqCst);
    Settlement::succeeded(frame.input().value().upstream.clone(), FactSet::empty())
}

fn counter_index(config: &FixtureConfig) -> usize {
    usize::try_from(config.value.saturating_sub(1))
        .ok()
        .filter(|index| *index < FIRST_CANDIDATE_CALLBACK_COUNTS.len())
        .unwrap_or_default()
}

fn decode_fixture_output(bytes: &[u8]) -> mfm_program::Result<FixtureOutput> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    decode_boundary(&canonical)
}

fn identity_output(value: &FixtureOutput) -> &FixtureOutput {
    value
}

/// Deterministic inputs for one genuinely qualified, source-free run.
pub struct QualifiedRunFixture {
    discriminator: u8,
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    entry_point_id: EntryPointId,
    entry_point_operation_id: StableId,
    invocation_identity: InvocationIdentity,
    qualification_scope_id: SemanticTypeId,
    configured_target: StableId,
    configured_contract: RetainedValueContract,
    configured_binding: ConfiguredValueBinding,
    configured_bytes: PlainCanonicalJsonBytes,
    append_request_id: AppendRequestId,
    incompatible_planning_profile: bool,
    mode: FixtureMode,
}

/// One store-prepared genuine run and its exact qualified registry.
pub struct PreparedQualifiedRun {
    registry: Arc<QualifiedProgramRegistry>,
    authority: RunAccessAuthority<Admit>,
    append: AdmitRun,
}

impl PreparedQualifiedRun {
    /// Returns the genuine registry that certified the prepared admission.
    pub const fn registry(&self) -> &Arc<QualifiedProgramRegistry> {
        &self.registry
    }

    /// Returns the exact admission-purpose authority.
    pub const fn authority(&self) -> &RunAccessAuthority<Admit> {
        &self.authority
    }

    /// Returns the complete store-prepared admission append.
    pub const fn append(&self) -> &AdmitRun {
        &self.append
    }

    /// Consumes the result into its registry, authority, and prepared append.
    pub fn into_parts(
        self,
    ) -> (
        Arc<QualifiedProgramRegistry>,
        RunAccessAuthority<Admit>,
        AdmitRun,
    ) {
        (self.registry, self.authority, self.append)
    }
}

impl QualifiedRunFixture {
    /// Constructs one genuine fixture bound to an already bootstrapped store.
    pub fn for_store(
        store_identity: StoreIdentity,
        discriminator: u8,
    ) -> Result<Self, QualifiedRunFixtureError> {
        Self::for_store_with_operation(
            store_identity,
            discriminator,
            StableId::new(DEFAULT_OPERATION_ID)?,
        )
    }

    /// Constructs a fixture selecting an explicit candidate operation.
    pub fn for_store_with_operation(
        store_identity: StoreIdentity,
        discriminator: u8,
        entry_point_operation_id: StableId,
    ) -> Result<Self, QualifiedRunFixtureError> {
        let tenant_scope_id = TenantScopeId::new(format!(
            "{}{}",
            TenantScopeId::PREFIX,
            format!("{:02x}", discriminator.wrapping_add(1)).repeat(16)
        ))?;
        let entry_point_id =
            EntryPointId::new(format!("mfm.fixture/qualified-run-{discriminator:02x}@1"))?;
        let invocation_identity =
            InvocationIdentity::new(format!("00000000-0000-4000-8000-{discriminator:012x}"))?;
        let qualification_scope_id = semantic_type(
            &format!("qualified-run-scope-{discriminator:02x}"),
            discriminator,
        )?;
        let configured_target = entry_point_operation_id.clone();
        let configured_contract = retained_contract(
            primitive_canonical_schema()?,
            "configured-value",
            "mfm.fixture.configured-value",
            component_object_evidence_ref()?,
        )?;
        let append_request_id =
            AppendRequestId::new(format!("qualified-fixture-admission/{discriminator:02x}"))?;
        let configured_bytes = canonical_json(&FixtureConfig {
            value: u64::from(discriminator) + 1,
            panic_on_apply: false,
        })?;
        let producer = ProducerBinding::configured_value(
            store_identity.store_scope_id(),
            &tenant_scope_id,
            &entry_point_id,
            &configured_target,
        )?;
        let value_ref =
            derive_value_ref(&configured_contract, &producer, configured_bytes.as_bytes())?;
        let key = ConfiguredValueKey::new(
            store_identity.store_scope_id(),
            &tenant_scope_id,
            &entry_point_id,
            &configured_target,
        )?;
        let configured_binding = ConfiguredValueBinding::new(&key, &value_ref)?;
        Ok(Self {
            discriminator,
            store_identity,
            tenant_scope_id,
            entry_point_id,
            entry_point_operation_id,
            invocation_identity,
            qualification_scope_id,
            configured_target,
            configured_contract,
            configured_binding,
            configured_bytes,
            append_request_id,
            incompatible_planning_profile: false,
            mode: FixtureMode::Single,
        })
    }

    /// Makes the qualified pure callback panic after its verified frame is decoded.
    pub fn with_panicking_state_callback(mut self) -> Result<Self, QualifiedRunFixtureError> {
        let mut config: FixtureConfig = decode_fixture(&self.configured_bytes)?;
        config.panic_on_apply = true;
        self.configured_bytes = canonical_json(&config)?;
        self.rebuild_configured_binding()?;
        Ok(self)
    }

    /// Makes this candidate fail the global plan-comparability predicate.
    pub fn with_incompatible_planning_profile(mut self) -> Self {
        self.incompatible_planning_profile = true;
        self
    }

    /// Registers a same-identity typed callback that rejects the normal historical frame.
    pub fn with_integrity_failing_state_callback(mut self) -> Self {
        self.mode = FixtureMode::IntegrityFailingStateCallback;
        self
    }

    /// Authors the baseline two-node chain used to record independent historical frames.
    pub fn with_two_node_chain(mut self) -> Self {
        self.mode = FixtureMode::TwoNodeChain;
        self
    }

    /// Authors the same chain with counted candidate callbacks and a divergent first output.
    pub fn with_two_node_divergent_candidate(mut self) -> Self {
        self.mode = FixtureMode::TwoNodeDivergentCandidate;
        self
    }

    /// Resets candidate callback counts isolated to this fixture discriminator.
    pub fn reset_candidate_callback_counts(&self) {
        let index = usize::from(self.discriminator);
        FIRST_CANDIDATE_CALLBACK_COUNTS[index].store(0, Ordering::SeqCst);
        SECOND_CANDIDATE_CALLBACK_COUNTS[index].store(0, Ordering::SeqCst);
    }

    /// Returns candidate callback counts isolated to this fixture discriminator.
    pub fn candidate_callback_counts(&self) -> CandidateCallbackCounts {
        let index = usize::from(self.discriminator);
        CandidateCallbackCounts {
            first_state: FIRST_CANDIDATE_CALLBACK_COUNTS[index].load(Ordering::SeqCst),
            second_state: SECOND_CANDIDATE_CALLBACK_COUNTS[index].load(Ordering::SeqCst),
        }
    }

    /// Returns the authoritative store lineage selected by this fixture.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the admitted tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the fixture's versioned entry point.
    pub const fn entry_point_id(&self) -> &EntryPointId {
        &self.entry_point_id
    }

    /// Returns the stable candidate operation.
    pub const fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    /// Returns the logical invocation identity.
    pub const fn invocation_identity(&self) -> &InvocationIdentity {
        &self.invocation_identity
    }

    /// Returns the exact qualified-deployment scope.
    pub const fn qualification_scope_id(&self) -> &SemanticTypeId {
        &self.qualification_scope_id
    }

    /// Returns the configured-value lookup target.
    pub const fn configured_target(&self) -> &StableId {
        &self.configured_target
    }

    /// Returns the exact configured-value retained contract.
    pub const fn configured_contract(&self) -> &RetainedValueContract {
        &self.configured_contract
    }

    /// Returns the immutable configured binding callers must provision.
    pub const fn configured_binding(&self) -> &ConfiguredValueBinding {
        &self.configured_binding
    }

    /// Returns exact canonical configured bytes callers must provision.
    pub fn configured_bytes(&self) -> &[u8] {
        self.configured_bytes.as_bytes()
    }

    /// Returns the first admission append request identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Admits support, builds the genuine registry, and prepares one run root.
    pub async fn prepare_on<B>(
        &self,
        store: &B,
        issuer: &RunAccessAuthorityIssuer,
    ) -> Result<PreparedQualifiedRun, PrepareQualifiedRunError<B::Error>>
    where
        B: RunJournalBackend + SupportBackend + ConfiguredValueBackend + AdmissionSourceBackend,
    {
        if RunJournalStore::store_identity(store) != &self.store_identity
            || issuer.store_identity() != &self.store_identity
        {
            return Err(QualifiedRunFixtureError::Invalid(
                "fixture, backend, and issuer store identities differ".to_owned(),
            )
            .into());
        }
        let package = self.package()?;
        let deployment_authority =
            issuer.authorize_qualified_deployment(self.qualification_scope_id.clone());
        let admitted_support =
            SupportStore::admit_support_graph(store, &deployment_authority, package.support_graph)
                .await
                .map_err(|error| PrepareQualifiedRunError::Backend(Box::new(error)))?;

        let mut builder = QualifiedProgramRegistry::builder(
            package.executable_identity_ref,
            package.planner,
            admitted_support,
            CertifiedJournalProtocolContracts::current()?,
        );
        builder.register_entry_point(package.entry_point)?;
        for state in package.states {
            state.register(&mut builder)?;
        }
        let registry = Arc::new(builder.build()?);

        let authority = issuer.authorize_admit(
            self.tenant_scope_id.clone(),
            self.entry_point_id.clone(),
            self.entry_point_operation_id.clone(),
            self.invocation_identity.clone(),
        );
        let configured = ConfiguredValueStore::resolve_configured_value(
            store,
            &authority,
            &self.entry_point_id,
            &self.configured_target,
            &self.configured_contract,
        )
        .await
        .map_err(|error| PrepareQualifiedRunError::Backend(Box::new(error)))?;
        let artifacts = registry.author_and_certify_verified_parts(
            &self.entry_point_id,
            configured.binding(),
            configured.value_ref(),
            configured.bytes(),
        )?;
        let sources = AdmissionSourceStore::verify_no_admission_sources(store, &authority)
            .await
            .map_err(|error| PrepareQualifiedRunError::Backend(Box::new(error)))?;
        let input = ProposedAdmissionInput::new(
            PlainCanonicalJsonBytes::from_json_str("{}")
                .map_err(|error| QualifiedRunFixtureError::Invalid(error.to_string()))?,
            admission_input_contract()?,
        );
        let append = RunJournalStore::prepare_admission(
            store,
            &authority,
            self.append_request_id.clone(),
            AdmissionMaterial::new(
                artifacts,
                input,
                &configured,
                registry.admitted_support(),
                &sources,
            ),
        )?;
        Ok(PreparedQualifiedRun {
            registry,
            authority,
            append,
        })
    }

    fn rebuild_configured_binding(&mut self) -> Result<(), QualifiedRunFixtureError> {
        let producer = ProducerBinding::configured_value(
            self.store_identity.store_scope_id(),
            &self.tenant_scope_id,
            &self.entry_point_id,
            &self.configured_target,
        )?;
        let value_ref = derive_value_ref(
            &self.configured_contract,
            &producer,
            self.configured_bytes.as_bytes(),
        )?;
        let key = ConfiguredValueKey::new(
            self.store_identity.store_scope_id(),
            &self.tenant_scope_id,
            &self.entry_point_id,
            &self.configured_target,
        )?;
        self.configured_binding = ConfiguredValueBinding::new(&key, &value_ref)?;
        Ok(())
    }

    fn package(&self) -> Result<PackageMaterial, QualifiedRunFixtureError> {
        let object_evidence_ref = component_object_evidence_ref()?;
        let planner_surface = CompositePlannerSurface::current()?;
        let qualification_canonical = canonical_json(&serde_json::json!({
            "discriminator": self.discriminator,
            "kind": "qualified-run-fixture"
        }))?;
        let qualification_ref =
            mfm_spec::exact_content_ref(primitive_canonical_schema()?, &qualification_canonical)?;
        let planner_descriptor = planner_surface.component_descriptor(qualification_ref.clone())?;
        let planner_implementation_ref = planner_descriptor.content_ref()?;

        let first_state_support = state_support_material(
            fixture_state_contract_canonical()?,
            r#"{"callbacks":["apply"],"kind":"pure"}"#,
            &qualification_ref,
        )?;
        let second_state_support = self
            .mode
            .is_two_node()
            .then(|| {
                state_support_material(
                    fixture_second_state_contract_canonical()?,
                    r#"{"callbacks":["apply"],"kind":"pure","state":"second"}"#,
                    &qualification_ref,
                )
            })
            .transpose()?;
        let mut state_manifest_entries = vec![StateImplementationManifestEntry {
            state_contract_ref: first_state_support
                .descriptor
                .semantic_contract_ref()
                .clone(),
            component_implementation_ref: first_state_support.descriptor.content_ref()?,
        }];
        if let Some(second) = &second_state_support {
            state_manifest_entries.push(StateImplementationManifestEntry {
                state_contract_ref: second.descriptor.semantic_contract_ref().clone(),
                component_implementation_ref: second.descriptor.content_ref()?,
            });
        }
        let state_manifest = StateImplementationManifest::new(state_manifest_entries)?;
        let capability_manifest = CapabilityBindingManifest::new(Vec::new())?;
        let executable_canonical = canonical_json(&serde_json::json!({
            "discriminator": self.discriminator,
            "kind": "executable-identity"
        }))?;
        let executable_identity_ref =
            mfm_spec::exact_content_ref(primitive_canonical_schema()?, &executable_canonical)?;

        let mut members = vec![
            support_member(
                OBJECT_EVIDENCE_PATH,
                component_object_evidence_contract_canonical()
                    .map_err(|error| QualifiedRunFixtureError::Invalid(error.to_string()))?,
                retained_contract(
                    object_evidence_ref.schema_id().clone(),
                    "object-evidence-contract",
                    "mfm.qualification.object-evidence-contract",
                    object_evidence_ref.clone(),
                )?,
            )?,
            support_member(
                QUALIFICATION_PATH,
                qualification_canonical,
                retained_contract(
                    primitive_canonical_schema()?,
                    "qualification",
                    "mfm.qualification.descriptor",
                    object_evidence_ref.clone(),
                )?,
            )?,
            support_member(
                EXECUTABLE_IDENTITY_PATH,
                executable_canonical,
                retained_contract(
                    primitive_canonical_schema()?,
                    "executable-identity",
                    "mfm.qualification.executable-identity",
                    object_evidence_ref.clone(),
                )?,
            )?,
            support_member(
                PLANNER_SEMANTIC_PATH,
                planner_surface.semantic_contract_canonical().clone(),
                retained_contract(
                    planner_surface.semantic_contract_ref().schema_id().clone(),
                    "planner-semantic-contract",
                    "mfm.qualification.planner.semantic-contract",
                    object_evidence_ref.clone(),
                )?,
            )?,
            support_member(
                PLANNER_CALLBACK_PATH,
                planner_surface.callback_surface_canonical().clone(),
                retained_contract(
                    planner_surface.callback_surface_ref().schema_id().clone(),
                    "planner-callback-surface",
                    "mfm.qualification.planner.callback-surface",
                    object_evidence_ref.clone(),
                )?,
            )?,
            component_member(
                PLANNER_IMPLEMENTATION_PATH,
                &planner_descriptor,
                "mfm.qualification.planner.implementation",
                object_evidence_ref.clone(),
            )?,
            support_member(
                STATE_SEMANTIC_PATH,
                first_state_support.contract_canonical.clone(),
                retained_contract(
                    primitive_canonical_schema()?,
                    "state-semantic-contract",
                    "mfm.qualification.state.semantic-contract",
                    object_evidence_ref.clone(),
                )?,
            )?,
            support_member(
                STATE_CALLBACK_PATH,
                first_state_support.callback_canonical.clone(),
                retained_contract(
                    primitive_canonical_schema()?,
                    "state-callback-surface",
                    "mfm.qualification.state.callback-surface",
                    object_evidence_ref.clone(),
                )?,
            )?,
            component_member(
                STATE_IMPLEMENTATION_PATH,
                &first_state_support.descriptor,
                "mfm.qualification.state.implementation",
                object_evidence_ref.clone(),
            )?,
        ];
        if let Some(second) = &second_state_support {
            members.extend([
                support_member(
                    SECOND_STATE_SEMANTIC_PATH,
                    second.contract_canonical.clone(),
                    retained_contract(
                        primitive_canonical_schema()?,
                        "second-state-semantic-contract",
                        "mfm.qualification.state.second.semantic-contract",
                        object_evidence_ref.clone(),
                    )?,
                )?,
                support_member(
                    SECOND_STATE_CALLBACK_PATH,
                    second.callback_canonical.clone(),
                    retained_contract(
                        primitive_canonical_schema()?,
                        "second-state-callback-surface",
                        "mfm.qualification.state.second.callback-surface",
                        object_evidence_ref.clone(),
                    )?,
                )?,
                component_member(
                    SECOND_STATE_IMPLEMENTATION_PATH,
                    &second.descriptor,
                    "mfm.qualification.state.second.implementation",
                    object_evidence_ref.clone(),
                )?,
            ]);
        }
        members.extend([
            support_member(
                STATE_MANIFEST_PATH,
                state_manifest.canonical_json()?,
                StateImplementationManifest::retained_contract()?,
            )?,
            support_member(
                CAPABILITY_MANIFEST_PATH,
                capability_manifest.canonical_json()?,
                CapabilityBindingManifest::retained_contract()?,
            )?,
        ]);
        let support_graph =
            QualifiedSupportGraph::new(self.qualification_scope_id.clone(), members)?;

        let planner = QualifiedPlannerRegistration::new(
            planner_descriptor,
            Arc::new(CompositeCertificationFactory),
        )?;
        let output_contract = state_output_contract()?;
        let mut states = vec![first_state_registration(
            first_state_support.descriptor,
            self.configured_contract.clone(),
            output_contract.clone(),
            object_evidence_ref.clone(),
            self.mode,
        )?];
        if let Some(second) = second_state_support {
            states.push(second_state_registration(
                second.descriptor,
                self.configured_contract.clone(),
                output_contract,
                object_evidence_ref,
                self.mode,
            )?);
        }

        let profile_parameters = if self.incompatible_planning_profile {
            serde_json::json!({"candidate":"incompatible"})
        } else {
            serde_json::json!({})
        };
        let entry_contract = EntryPointContract::new(
            self.entry_point_id.clone(),
            self.entry_point_operation_id.clone(),
            PlanningProfile::new(
                planner_surface.semantic_contract_ref().clone(),
                planner_implementation_ref,
                Vec::new(),
                CanonicalJsonValue::new(profile_parameters)?,
            )?,
            primitive_canonical_schema()?,
            primitive_canonical_schema()?,
        )?;
        let entry_input_contract = QualifiedEntryPointInputContract::new(
            QualifiedSourceContract::new(admission_input_contract()?, Vec::new())?,
            QualifiedSourceContract::new(self.configured_contract.clone(), Vec::new())?,
            None,
            None,
            None,
            Vec::new(),
        )?;
        let operation_id = self.entry_point_operation_id.clone();
        let two_node = self.mode.is_two_node();
        let entry_point = QualifiedEntryPointRegistration::new(
            entry_contract,
            entry_input_contract,
            public_output_contract()?,
            Vec::new(),
            move |inputs: &QualifiedAuthoringInputs<'_>| {
                author_fixture_program(inputs, &operation_id, two_node)
            },
        )?;
        Ok(PackageMaterial {
            executable_identity_ref,
            planner,
            entry_point,
            states,
            support_graph,
        })
    }
}

struct PackageMaterial {
    executable_identity_ref: ContentRef,
    planner: QualifiedPlannerRegistration,
    entry_point: QualifiedEntryPointRegistration,
    states: Vec<FixtureStateRegistration>,
    support_graph: QualifiedSupportGraph,
}

struct StateSupportMaterial {
    contract_canonical: PlainCanonicalJsonBytes,
    callback_canonical: PlainCanonicalJsonBytes,
    descriptor: ComponentImplementationDescriptor,
}

fn state_support_material(
    contract_canonical: PlainCanonicalJsonBytes,
    callback_json: &str,
    qualification_ref: &ContentRef,
) -> Result<StateSupportMaterial, QualifiedRunFixtureError> {
    let contract_ref =
        mfm_spec::exact_content_ref(primitive_canonical_schema()?, &contract_canonical)?;
    let callback_canonical = PlainCanonicalJsonBytes::from_json_str(callback_json)
        .map_err(|error| QualifiedRunFixtureError::Invalid(error.to_string()))?;
    let callback_ref =
        mfm_spec::exact_content_ref(primitive_canonical_schema()?, &callback_canonical)?;
    let descriptor = ComponentImplementationDescriptor::new(
        ComponentKind::State,
        contract_ref,
        callback_ref,
        qualification_ref.clone(),
    )?;
    Ok(StateSupportMaterial {
        contract_canonical,
        callback_canonical,
        descriptor,
    })
}

enum FixtureStateRegistration {
    First(QualifiedStateRegistration<FixtureState>),
    IntegrityFailing(QualifiedStateRegistration<IntegrityFailingFixtureState>),
    Second(QualifiedStateRegistration<FixtureSecondState>),
}

impl FixtureStateRegistration {
    fn register(self, builder: &mut QualifiedProgramRegistryBuilder) -> mfm_program::Result<()> {
        match self {
            Self::First(state) => {
                builder.register_state(state)?;
            }
            Self::IntegrityFailing(state) => {
                builder.register_state(state)?;
            }
            Self::Second(state) => {
                builder.register_state(state)?;
            }
        }
        Ok(())
    }
}

fn first_state_registration(
    descriptor: ComponentImplementationDescriptor,
    config_contract: RetainedValueContract,
    output_contract: RetainedValueContract,
    evidence_contract_ref: ContentRef,
    mode: FixtureMode,
) -> Result<FixtureStateRegistration, QualifiedRunFixtureError> {
    let input_contract = state_input_value_contract::<FixtureInput>(
        semantic_type("state-input", 0)?,
        StableId::new("mfm.fixture.state-input")?,
        evidence_contract_ref,
    )?;
    let contract = qualified_state_contract(
        descriptor.semantic_contract_ref().clone(),
        config_contract,
        input_contract,
        Vec::new(),
        output_contract.clone(),
    )?;
    match mode {
        FixtureMode::IntegrityFailingStateCallback => Ok(
            FixtureStateRegistration::IntegrityFailing(QualifiedStateRegistration::new(
                contract,
                descriptor,
                StateExecution::pure(apply_integrity_failing_fixture_state),
                fixture_settlement_codecs(output_contract)?,
            )?),
        ),
        FixtureMode::TwoNodeDivergentCandidate => Ok(FixtureStateRegistration::First(
            QualifiedStateRegistration::new(
                contract,
                descriptor,
                StateExecution::pure(apply_divergent_first_candidate),
                fixture_settlement_codecs(output_contract)?,
            )?,
        )),
        FixtureMode::Single | FixtureMode::TwoNodeChain => Ok(FixtureStateRegistration::First(
            QualifiedStateRegistration::new(
                contract,
                descriptor,
                StateExecution::pure(apply_fixture_state),
                fixture_settlement_codecs(output_contract)?,
            )?,
        )),
    }
}

fn second_state_registration(
    descriptor: ComponentImplementationDescriptor,
    config_contract: RetainedValueContract,
    output_contract: RetainedValueContract,
    evidence_contract_ref: ContentRef,
    mode: FixtureMode,
) -> Result<FixtureStateRegistration, QualifiedRunFixtureError> {
    let input_contract = state_input_value_contract::<FixtureChainedInput>(
        semantic_type("chained-state-input", 0)?,
        StableId::new("mfm.fixture.chained-state-input")?,
        evidence_contract_ref,
    )?;
    let input_destinations = vec![QualifiedInputContract::new(
        FieldPath::new("upstream")?,
        CertifiedInputDestination::OrdinaryValue,
        QualifiedSourceContract::new(output_contract.clone(), Vec::new())?,
    )];
    let contract = qualified_state_contract(
        descriptor.semantic_contract_ref().clone(),
        config_contract,
        input_contract,
        input_destinations,
        output_contract.clone(),
    )?;
    let apply = match mode {
        FixtureMode::TwoNodeDivergentCandidate => apply_divergent_second_candidate,
        FixtureMode::TwoNodeChain => apply_second_fixture_state,
        FixtureMode::Single | FixtureMode::IntegrityFailingStateCallback => {
            return Err(QualifiedRunFixtureError::Invalid(
                "a second fixture state requires a two-node mode".to_owned(),
            ));
        }
    };
    Ok(FixtureStateRegistration::Second(
        QualifiedStateRegistration::new(
            contract,
            descriptor,
            StateExecution::pure(apply),
            fixture_settlement_codecs(output_contract)?,
        )?,
    ))
}

fn qualified_state_contract(
    state_contract_ref: ContentRef,
    config_contract: RetainedValueContract,
    input_contract: RetainedValueContract,
    input_destinations: Vec<QualifiedInputContract>,
    output_contract: RetainedValueContract,
) -> Result<QualifiedStateContract, QualifiedRunFixtureError> {
    Ok(QualifiedStateContract::new(
        state_contract_ref,
        config_contract,
        None,
        input_contract,
        input_destinations,
        vec![QualifiedOutputSourceContract::new(
            0,
            QualifiedSourceContract::new(output_contract.clone(), Vec::new())?,
        )],
        CertifiedStateExecution::Pure,
        CertifiedSettlementContract::new(
            None,
            vec![CertifiedOutputSlot::new(
                0,
                FieldPath::new("value")?,
                output_contract,
            )],
            Vec::new(),
        )?,
    )?)
}

fn fixture_settlement_codecs<S>(
    output_contract: RetainedValueContract,
) -> Result<QualifiedSettlementCodecs<S>, QualifiedRunFixtureError>
where
    S: State<Output = FixtureOutput>,
{
    Ok(QualifiedSettlementCodecs::new(
        vec![QualifiedOutputProjector::new(
            0,
            FieldPath::new("value")?,
            CanonicalCodec::new(
                output_contract,
                encode_boundary::<FixtureOutput>,
                decode_fixture_output,
            ),
            identity_output,
        )],
        None,
    )?)
}

fn author_fixture_program(
    inputs: &QualifiedAuthoringInputs<'_>,
    operation_id: &StableId,
    two_node: bool,
) -> mfm_program::Result<mfm_spec::CanonicalAuthoredProgram> {
    let _: FixtureConfig = decode_boundary(inputs.configured().canonical())?;
    let mut builder = AuthoredProgramBuilder::new(operation_id.clone());
    if two_node {
        let first = builder.state::<FixtureState>(
            StableId::new("fixture-state-first")
                .map_err(|error| ProgramError::Authoring(error.to_string()))?,
            StateBindings::new(inputs.configured().value_ref().clone(), None),
        )?;
        let second = builder.state::<FixtureSecondState>(
            StableId::new("fixture-state-second")
                .map_err(|error| ProgramError::Authoring(error.to_string()))?,
            StateBindings::new(inputs.configured().value_ref().clone(), None),
        )?;
        builder.connect_to(first.output(), &second, 0)?;
        builder.required_success(first.output())?;
        builder.required_success(second.output())?;
        builder.public_output(
            StableId::new("result").map_err(|error| ProgramError::Authoring(error.to_string()))?,
            second.output(),
        )?;
    } else {
        let state = builder.state::<FixtureState>(
            StableId::new("fixture-state")
                .map_err(|error| ProgramError::Authoring(error.to_string()))?,
            StateBindings::new(inputs.configured().value_ref().clone(), None),
        )?;
        builder.required_success(state.output())?;
        builder.public_output(
            StableId::new("result").map_err(|error| ProgramError::Authoring(error.to_string()))?,
            state.output(),
        )?;
    }
    builder.finish()
}

fn admission_input_contract() -> Result<RetainedValueContract, QualifiedRunFixtureError> {
    retained_contract(
        primitive_canonical_schema()?,
        "admission-input",
        "mfm.fixture.admission-input",
        component_object_evidence_ref()?,
    )
}

fn public_output_contract() -> Result<RetainedValueContract, QualifiedRunFixtureError> {
    retained_contract(
        primitive_canonical_schema()?,
        "public-output",
        "mfm.fixture.public-output",
        component_object_evidence_ref()?,
    )
}

fn state_output_contract() -> Result<RetainedValueContract, QualifiedRunFixtureError> {
    retained_contract(
        primitive_canonical_schema()?,
        "state-output",
        "mfm.fixture.state-output",
        component_object_evidence_ref()?,
    )
}

fn fixture_state_contract_canonical() -> Result<PlainCanonicalJsonBytes, QualifiedRunFixtureError> {
    PlainCanonicalJsonBytes::from_json_str(
        r#"{"execution":"pure","version":"mfm.test-qualified-state.v1"}"#,
    )
    .map_err(|error| QualifiedRunFixtureError::Invalid(error.to_string()))
}

fn fixture_state_contract_ref() -> Result<ContentRef, QualifiedRunFixtureError> {
    Ok(mfm_spec::exact_content_ref(
        primitive_canonical_schema()?,
        &fixture_state_contract_canonical()?,
    )?)
}

fn fixture_second_state_contract_canonical(
) -> Result<PlainCanonicalJsonBytes, QualifiedRunFixtureError> {
    PlainCanonicalJsonBytes::from_json_str(
        r#"{"execution":"pure","version":"mfm.test-qualified-second-state.v1"}"#,
    )
    .map_err(|error| QualifiedRunFixtureError::Invalid(error.to_string()))
}

fn fixture_second_state_contract_ref() -> Result<ContentRef, QualifiedRunFixtureError> {
    Ok(mfm_spec::exact_content_ref(
        primitive_canonical_schema()?,
        &fixture_second_state_contract_canonical()?,
    )?)
}

fn component_object_evidence_ref() -> Result<ContentRef, QualifiedRunFixtureError> {
    mfm_values::component_object_evidence_contract_ref().map_err(QualifiedRunFixtureError::from)
}

fn primitive_canonical_schema() -> Result<SchemaId, QualifiedRunFixtureError> {
    Ok(RecoverabilityContractV1::embedded()?
        .schema_id("mfm.primitive-canonical_value.v1")?
        .clone())
}

fn retained_contract(
    schema_id: SchemaId,
    semantic_name: &str,
    role: &str,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract, QualifiedRunFixtureError> {
    Ok(RetainedValueContract::new(
        schema_id,
        semantic_type(semantic_name, 0)?,
        StableId::new(role)?,
        "application/json",
        evidence_contract_ref,
    )?)
}

fn semantic_type(
    name: &str,
    discriminator: u8,
) -> Result<SemanticTypeId, QualifiedRunFixtureError> {
    Ok(SemanticTypeId::new(
        "mfm.fixture",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("semantic:mfm.fixture:{name}:{discriminator}").as_bytes()),
    )?)
}

fn support_member(
    path: &str,
    canonical: PlainCanonicalJsonBytes,
    value_contract: RetainedValueContract,
) -> Result<QualifiedSupportMember, QualifiedRunFixtureError> {
    Ok(QualifiedSupportMember::new(
        FieldPath::new(path)?,
        canonical,
        value_contract,
    ))
}

fn component_member(
    path: &str,
    descriptor: &ComponentImplementationDescriptor,
    role: &str,
    evidence_contract_ref: ContentRef,
) -> Result<QualifiedSupportMember, QualifiedRunFixtureError> {
    support_member(
        path,
        descriptor.canonical_json()?,
        retained_contract(
            descriptor.content_ref()?.schema_id().clone(),
            "component-implementation",
            role,
            evidence_contract_ref,
        )?,
    )
}

fn canonical_json(
    value: &impl Serialize,
) -> Result<PlainCanonicalJsonBytes, QualifiedRunFixtureError> {
    let json = serde_json::to_string(value)
        .map_err(|error| QualifiedRunFixtureError::Invalid(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| QualifiedRunFixtureError::Invalid(error.to_string()))
}

fn decode_fixture<T>(canonical: &PlainCanonicalJsonBytes) -> Result<T, QualifiedRunFixtureError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    decode_boundary(canonical).map_err(Into::into)
}

fn derive_value_ref(
    contract: &RetainedValueContract,
    producer: &ProducerBinding,
    bytes: &[u8],
) -> Result<ValueRef, QualifiedRunFixtureError> {
    let recoverability = RecoverabilityContractV1::embedded()?;
    let content_digest = recoverability.raw_content_digest(bytes);
    let artifact_id = ArtifactIdPreimage::new(
        contract.schema_id(),
        &content_digest,
        contract.semantic_type_id(),
    )?
    .artifact_id()?;
    let byte_length = u64::try_from(bytes.len()).map_err(|_| {
        QualifiedRunFixtureError::Invalid(
            "configured fixture bytes exceed the retained length bound".to_owned(),
        )
    })?;
    let evidence_hash: ObjectEvidenceDigest = ObjectEvidencePreimage::new(
        &artifact_id,
        &content_digest,
        contract.schema_id(),
        byte_length,
        contract.media_type(),
        contract.evidence_contract_ref(),
    )?
    .evidence_hash()?;
    Ok(ValueRef::new(
        &artifact_id,
        &content_digest,
        &evidence_hash,
        contract.schema_id(),
        contract.semantic_type_id(),
        contract.role(),
        byte_length,
        contract.media_type(),
        contract.evidence_contract_ref(),
        producer,
    )?)
}
