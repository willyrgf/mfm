use super::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor, EffectSpec, ManagedPlatformWrite,
};
use mfm_ids::{
    ArtifactId, DigestBytes, EffectKind, EffectVersion, EventId, SchemaId, ScopeId, SeedId,
    SemanticTypeId, StateKind, StateVersion, TrustScopeId,
};
use mfm_manual_auth::{
    ManualAuthorizationSignatureBytes, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionEvidenceRef,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, PublicOutputKey, PureState, RootBuilder, ScopeKey,
    StateKey, StateRegistryBuilder, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use mfm_store::v1::RunEventStore;
use serde::{Deserialize, Serialize};

const D0: DigestBytes = DigestBytes::from_array([0x10; 32]);
const D1: DigestBytes = DigestBytes::from_array([0x11; 32]);
const D2: DigestBytes = DigestBytes::from_array([0x12; 32]);
const D3: DigestBytes = DigestBytes::from_array([0x13; 32]);
const D4: DigestBytes = DigestBytes::from_array([0x14; 32]);
const D5: DigestBytes = DigestBytes::from_array([0x15; 32]);
const D6: DigestBytes = DigestBytes::from_array([0x16; 32]);
const D7: DigestBytes = DigestBytes::from_array([0x17; 32]);
const D8: DigestBytes = DigestBytes::from_array([0x18; 32]);
const D9: DigestBytes = DigestBytes::from_array([0x19; 32]);

fn fixture_value_semantic_id() -> SemanticTypeId {
    SemanticTypeId::new("mfm.test", "value", "1", DigestAlgorithm::Sha256JcsV1, D9)
        .expect("semantic")
}

fn fixture_value_schema_id() -> SchemaId {
    SchemaId::new("mfm.test.value", "1", DigestAlgorithm::Sha256JcsV1, DA).expect("schema")
}

fn fixture_trust_scope_id() -> TrustScopeId {
    TrustScopeId::new("mfm.trust_scope.v1:10101010101010101010101010101010")
        .expect("test trust scope")
}

fn run_identity_material(runtime_spec: &CertifiedRuntimeSpec) -> events::RunIdentityMaterialV1 {
    run_identity_material_with_distinct(runtime_spec, None)
}

fn run_identity_material_with_distinct(
    runtime_spec: &CertifiedRuntimeSpec,
    distinct_run_key_digest: Option<ContentDigest>,
) -> events::RunIdentityMaterialV1 {
    events::RunIdentityMaterialV1 {
        certified_spec_hash: runtime_spec.spec_hash().clone(),
        trust_scope_id: fixture_trust_scope_id(),
        distinct_run_key_digest,
    }
}

fn fixture_run_identity_material(fixture: &Fixture) -> events::RunIdentityMaterialV1 {
    run_identity_material_with_distinct(
        &fixture.runtime_spec,
        fixture.distinct_run_key_digest.clone(),
    )
}

fn refresh_fixture_run_id(fixture: &mut Fixture) {
    let distinct_run_key_digest = fixture.distinct_run_key_digest.clone();
    refresh_fixture_run_id_with_distinct(fixture, distinct_run_key_digest);
}

fn refresh_fixture_run_id_with_distinct(
    fixture: &mut Fixture,
    distinct_run_key_digest: Option<ContentDigest>,
) {
    fixture.distinct_run_key_digest = distinct_run_key_digest.clone();
    fixture.run_id =
        run_identity_material_with_distinct(&fixture.runtime_spec, distinct_run_key_digest)
            .derive_run_id()
            .expect("fixture run id");
}

macro_rules! store_typed_commit_request {
    (
        run_id: $run_id:expr,
        expected_next_seq: $expected_next_seq:expr,
        commit_key: $commit_key:expr,
        payloads: $payloads:expr,
        required_artifacts: $required_artifacts:expr,
        preconditions: $preconditions:expr $(,)?
    ) => {
        store::CommitRequest::from_payloads(
            $run_id,
            $expected_next_seq,
            $commit_key,
            $payloads,
            $required_artifacts,
            $preconditions,
        )
        .expect("typed commit request")
    };
}

trait TestPreparedCommitExt {
    fn append_prepared_commit(
        &mut self,
        request: store::CommitRequest,
    ) -> store::Result<store::CommitOutcome>;
}

impl TestPreparedCommitExt for TestTypedRunStore {
    fn append_prepared_commit(
        &mut self,
        request: store::CommitRequest,
    ) -> store::Result<store::CommitOutcome> {
        let admitted_artifacts = request.required_artifacts().to_vec();
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_test_commit_plan(plan)
    }
}

fn test_bundle_from_plan(
    plan: store::PreparedCommitPlan,
) -> store::Result<store::PreparedCommitBundle> {
    let existing = plan
        .admitted_artifacts()
        .iter()
        .map(|evidence| {
            Ok(store::ExistingArtifactAdmission::new(
                evidence.artifact_id.clone(),
                evidence.evidence_hash()?,
            ))
        })
        .collect::<store::Result<Vec<_>>>()?;
    store::PreparedCommitBundle::new(plan, Vec::new(), existing)
}

fn test_prepared_commit_plan(
    request: store::CommitRequest,
    admitted_artifacts: Vec<store::ArtifactEvidenceRef>,
) -> store::Result<store::PreparedCommitPlan> {
    let artifacts = store::CommitArtifactEvidenceSet::new(
        request.required_artifacts().to_vec(),
        admitted_artifacts,
    )?;
    if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, events::KernelEventPayload::RunAdmitted(_)))
    {
        return store::PreparedCommit::<store::RunAdmission>::new(request, artifacts)
            .map(store::PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, events::KernelEventPayload::StateAttemptStarted(_)))
    {
        let mut preconditions = request.preconditions().clone();
        preconditions.required_run_state = store::RequiredRunState::NotCompleted;
        let request = request.with_preconditions(preconditions);
        return store::PreparedCommit::<store::StateAttemptStarted>::new(request, artifacts)
            .map(store::PreparedCommitPlan::from);
    }
    if request.payloads().iter().any(test_is_run_completed_payload) {
        return store::PreparedCommit::<store::AttemptTerminal>::new(request, artifacts)
            .map(store::PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .any(test_is_side_effect_terminal_payload)
    {
        return store::PreparedCommit::<store::SideEffectTerminal>::new(request, artifacts)
            .map(store::PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .any(|payload| payload.side_effect_ref().is_some())
    {
        return store::PreparedCommit::<store::SideEffectProgress>::new(request, artifacts)
            .map(store::PreparedCommitPlan::from);
    }
    if request.payloads().iter().any(test_is_retention_payload) {
        return store::PreparedCommit::<store::Retention>::new(request, artifacts)
            .map(store::PreparedCommitPlan::from);
    }
    store::PreparedCommit::<store::AttemptTerminal>::new(request, artifacts)
        .map(store::PreparedCommitPlan::from)
}

fn test_is_retention_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(
        payload,
        events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
    )
}

fn test_is_run_completed_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(payload, events::KernelEventPayload::RunCompleted(_))
}

fn test_is_side_effect_terminal_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(
        payload,
        events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_)
            | events::KernelEventPayload::ResourceLaneReleased(_)
    )
}

fn validate_runtime_stream_for_tests(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    RuntimeRunView::from_stream(runtime_spec, run_id, stream).map(|_| ())
}

fn block_on_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(future.as_mut(), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("test in-memory store future unexpectedly pending"),
    }
}

#[derive(Clone, Default)]
struct TestTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
}

impl TestTypedRunStore {
    fn new() -> Self {
        Self::default()
    }

    fn append_test_commit_plan(
        &self,
        plan: store::PreparedCommitPlan,
    ) -> store::Result<store::CommitOutcome> {
        self.inner
            .seed_artifact_evidence_for_test(plan.admitted_artifacts())?;
        let bundle = test_bundle_from_plan(plan)?;
        block_on_ready(self.inner.append_prepared_commit_bundle(bundle))
    }

    fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        block_on_ready(self.inner.load_run_stream(run_id)).expect("test store read")
    }

    fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
        block_on_ready(self.inner.expected_next_seq(run_id)).expect("test store next seq")
    }

    fn projection_snapshot(&self) -> store::ProjectionSnapshot {
        self.inner
            .projection_snapshot()
            .expect("test store projection")
    }
}

impl store::RunEventStore for TestTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            self.inner.append_prepared_commit_bundle(bundle).await
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }
}

impl store::RetainedArtifactReadProvider for TestTypedRunStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        self.inner.read_retained_artifact(requirement)
    }
}

#[derive(Clone)]
struct RecordedPreparedCommit {
    seq: store::StreamSeq,
    commit_key: store::CommitKey,
    payloads: Vec<events::KernelEventPayload>,
    admitted_artifacts: Vec<store::ArtifactEvidenceRef>,
}

struct RecordingTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
    commits: Arc<Mutex<Vec<RecordedPreparedCommit>>>,
}

impl RecordingTypedRunStore {
    fn new() -> Self {
        Self {
            inner: store::AsyncInMemoryRunStore::new(),
            commits: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn commits(&self) -> Vec<RecordedPreparedCommit> {
        self.commits
            .lock()
            .expect("recording store commits lock")
            .clone()
    }

    async fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        self.inner
            .load_run_stream(run_id)
            .await
            .expect("recording store read")
    }

    async fn projection_snapshot(&self, run_id: &RunId) -> store::ProjectionSnapshot {
        self.inner
            .status_projection_snapshot(run_id)
            .await
            .expect("recording store projection")
    }
}

struct StaleOnceTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
    stale_terminal_injected: Mutex<bool>,
}

impl StaleOnceTypedRunStore {
    fn new() -> Self {
        Self {
            inner: store::AsyncInMemoryRunStore::new(),
            stale_terminal_injected: Mutex::new(false),
        }
    }

    async fn projection_snapshot(&self, run_id: &RunId) -> store::ProjectionSnapshot {
        self.inner
            .status_projection_snapshot(run_id)
            .await
            .expect("stale-once store projection")
    }
}

impl store::RunEventStore for RecordingTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let payloads = bundle.request().payloads().to_vec();
        let admitted_artifacts = bundle.admitted_artifacts().to_vec();
        Box::pin(async move {
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            let outcome = self.inner.append_prepared_commit_bundle(bundle).await?;
            if let store::CommitOutcome::Appended(batch) = &outcome {
                self.commits
                    .lock()
                    .map_err(|_| {
                        store::StoreError::Event("recording store lock poisoned".to_owned())
                    })?
                    .push(RecordedPreparedCommit {
                        seq: batch.seq(),
                        commit_key: batch.commit_key().clone(),
                        payloads,
                        admitted_artifacts,
                    });
            }
            Ok(outcome)
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }
}

impl store::RunEventStore for StaleOnceTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            let is_run_start = bundle
                .request()
                .payloads()
                .iter()
                .any(|payload| matches!(payload, events::KernelEventPayload::RunAdmitted(_)));
            let has_terminal = bundle.request().payloads().iter().any(|payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::StateAttemptCompleted(_)
                        | events::KernelEventPayload::StateAttemptFailed(_)
                        | events::KernelEventPayload::StateAttemptInterrupted(_)
                )
            });
            let should_inject = {
                let mut injected = self.stale_terminal_injected.lock().map_err(|_| {
                    store::StoreError::Event("stale-once store lock poisoned".to_owned())
                })?;
                let should_inject = !*injected && !is_run_start && has_terminal;
                if should_inject {
                    *injected = true;
                }
                should_inject
            };
            if should_inject {
                let expected = bundle.request().expected_next_seq();
                let run_id = bundle.request().run_id().clone();
                self.inner
                    .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
                self.inner.append_prepared_commit_bundle(bundle).await?;
                return Err(store::StoreError::StaleExpectedNextSeq {
                    expected,
                    actual: self.inner.expected_next_seq(&run_id).await?,
                });
            }
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            self.inner.append_prepared_commit_bundle(bundle).await
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }
}

type TestArtifactMap = BTreeMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>;

#[derive(Clone, Default)]
struct TestRuntimeArtifactStore {
    artifacts: Arc<Mutex<TestArtifactMap>>,
}

impl store::RetainedArtifactReadProvider for TestRuntimeArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let (bytes, evidence) = self
                .artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .get(&requirement.artifact_id)
                .cloned()
                .ok_or_else(|| store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                })?;
            store::VerifiedRunArtifactBytes::new(bytes, evidence, requirement)
        })
    }
}

#[derive(Clone)]
struct RecordingRuntimeArtifactStore {
    artifacts: Arc<Mutex<TestArtifactMap>>,
}

impl store::RetainedArtifactReadProvider for RecordingRuntimeArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let (bytes, evidence) = self
                .artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .get(&requirement.artifact_id)
                .cloned()
                .ok_or_else(|| store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                })?;
            store::VerifiedRunArtifactBytes::new(bytes, evidence, requirement)
        })
    }
}

#[derive(Clone)]
struct FilteringRuntimeArtifactStore {
    source: TestTypedRunStore,
    missing_artifacts: Arc<Mutex<BTreeSet<ArtifactId>>>,
}

impl FilteringRuntimeArtifactStore {
    fn new(source: TestTypedRunStore) -> Self {
        Self {
            source,
            missing_artifacts: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    fn hide_artifact(&self, artifact_id: ArtifactId) {
        self.missing_artifacts
            .lock()
            .expect("filtering artifact store")
            .insert(artifact_id);
    }
}

impl store::RetainedArtifactReadProvider for FilteringRuntimeArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            if self
                .missing_artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .contains(&requirement.artifact_id)
            {
                return Err(store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            }
            self.source.read_retained_artifact(requirement).await
        })
    }
}

fn test_scheduler(registry: ErasedRunnerRegistry) -> SerialTypedScheduler {
    test_scheduler_with_artifacts(registry, Arc::new(TestRuntimeArtifactStore::default()))
}

fn test_scheduler_with_artifacts(
    registry: ErasedRunnerRegistry,
    artifact_store: Arc<dyn RuntimeArtifactStore>,
) -> SerialTypedScheduler {
    SerialTypedScheduler::new(registry, artifact_store)
}

fn register_fixture_capabilities(
    mut registry: ErasedRunnerRegistry,
    fixture: &Fixture,
) -> ErasedRunnerRegistry {
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
}

fn register_spec_capabilities(
    registry: &mut ErasedRunnerRegistry,
    runtime_spec: &CertifiedRuntimeSpec,
) {
    let implementation_id =
        CapabilityImplementationId::new("mfm.test.capability").expect("capability implementation");
    for node in runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(runtime_spec.spec().remediations.values())
    {
        registry
            .register_capability_set(&node.capability_bindings, implementation_id.clone())
            .expect("capability binding");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "value",
    version = "1",
    schema = "mfm.runtime.test.value"
)]
struct CertifierValue {
    amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FixtureOutputValue {
    amount: u64,
    node_id: String,
    attempt_id: String,
}

impl mfm_values::MfmValue for FixtureOutputValue {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        <CertifierValue as mfm_values::MfmValue>::schema_descriptor()
    }

    fn schema_id() -> mfm_values::Result<SchemaId> {
        Ok(fixture_value_schema_id())
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        Ok(fixture_value_semantic_id())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "side_effect_evidence",
    version = "1",
    schema = "mfm.runtime.test.side_effect_evidence"
)]
struct FixtureSideEffectEvidence {
    amount: u64,
    node_id: String,
    attempt_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct CertifierConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.runtime.test.public_outputs")]
struct CertifierPublicOutputs<'p, 's> {
    result: mfm_program::Handle<'p, 's, CertifierValue>,
}

struct CertifierState {
    config: CertifierConfig,
}

impl StateSpec for CertifierState {
    type Config = CertifierConfig;
    type Input = CertifierValue;
    type Output = CertifierValue;
    type Effect = mfm_effects::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.runtime.test",
            "multiply",
            DigestAlgorithm::Sha256JcsV1,
            D1,
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.runtime.test.multiply.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.runtime.test.multiply"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for CertifierState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(CertifierValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

fn certifier_backed_runtime_authority() -> (
    mfm_certify::CertifiedTypedSpec,
    mfm_certify::CertificationRegistry,
) {
    let mut states = StateRegistryBuilder::new();
    let registered = states
        .register::<CertifierState>()
        .expect("state registration");
    let mut registry = mfm_certify::CertificationRegistry::new();
    registry
        .register_state(&registered)
        .expect("certification registry");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed"),
            )?;
            let result = root.scope().state::<CertifierState, _>(
                StateKey::new("multiply-state")?,
                CertifierConfig { multiplier: 3 },
                seed,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &CertifierPublicOutputs { result },
            )
        },
    )
    .expect("program draft");
    (
        mfm_certify::certify_program_draft(&draft).expect("certified program"),
        registry,
    )
}

const DA: DigestBytes = DigestBytes::from_array([0x1a; 32]);
const DB: DigestBytes = DigestBytes::from_array([0x1b; 32]);
const DC: DigestBytes = DigestBytes::from_array([0x1c; 32]);
const DD: DigestBytes = DigestBytes::from_array([0x1d; 32]);
const DE: DigestBytes = DigestBytes::from_array([0x1e; 32]);
const DF: DigestBytes = DigestBytes::from_array([0x1f; 32]);
const TEST_CONFIG_BYTES: &[u8] = b"{}";
const TEST_SEED_BYTES: &[u8] = br#"{"seed":true}"#;

#[derive(Clone)]
struct Fixture {
    runtime_spec: CertifiedRuntimeSpec,
    run_id: RunId,
    distinct_run_key_digest: Option<ContentDigest>,
    seed_ref: events::SeedCellRef,
    descriptor_a: DescriptorId,
    descriptor_b: DescriptorId,
    descriptor_c: Option<DescriptorId>,
    render_node: NodeId,
    render_cell: CellId,
    cell_a: CellId,
    cell_b: CellId,
    cell_c: Option<CellId>,
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
}

struct RecordingRunner {
    expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
}

impl ErasedNodeRunner for RecordingRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            for (kind, version) in &self.expected_caps {
                assert!(ctx.caps().contains(kind, version));
            }
            let output_cell = ctx.node().output_cell.clone();
            let cell = match ctx.inputs().root.clone() {
                MaterializedInputNode::Cell(cell) => cell,
                MaterializedInputNode::Unit
                | MaterializedInputNode::Tuple(_)
                | MaterializedInputNode::Struct(_)
                | MaterializedInputNode::Vec(_)
                | MaterializedInputNode::NonEmptyVec(_) => {
                    panic!("expected cell input")
                }
            };
            assert!(matches!(
                cell.terminal,
                MaterializedCellTerminal::Seed { .. } | MaterializedCellTerminal::Produced { .. }
            ));
            let certified_cell = ctx.projections().cell_terminal(&output_cell).is_none();
            assert!(certified_cell);
            let artifact = store::ArtifactEvidenceRef {
                artifact_id: self.output_artifact.clone(),
                digest: self.output_digest.clone(),
                byte_len: 17,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                producer_node_id: Some(ctx.node().node_id.clone()),
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::StateOutput,
            };
            let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: Vec::new(),
                payloads: vec![RunnerEventPayload::CellProduced(events::CellProduced {
                    spec_hash: ctx.spec_hash().clone(),
                    node_id: ctx.node().node_id.clone(),
                    cell_id: ctx.node().output_cell.clone(),
                    scope_id: ctx.node().scope_id.clone(),
                    attempt_id: ctx.attempt_id().clone(),
                    semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
                    schema_id: ctx.descriptor().output_schema_id.clone(),
                    value_lineage: ctx.output_cell().value_lineage.clone(),
                    artifact_id: self.output_artifact.clone(),
                    content_digest: self.output_digest.clone(),
                    producer_state_kind: Some(ctx.node().state_kind.clone()),
                    producer_state_version: Some(ctx.node().state_version.clone()),
                })],
            })
        })
    }
}

struct BlockingRunner;

impl ErasedNodeRunner for BlockingRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            Err(RuntimeError::Blocked(format!(
                "node {} is failed manually in this fixture",
                ctx.node().node_id
            )))
        })
    }
}

struct ErrorRunner {
    error: RuntimeError,
}

impl ErasedNodeRunner for ErrorRunner {
    fn run_erased<'a>(&'a self, _ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { Err(self.error.clone()) })
    }
}

#[test]
fn runner_kit_builders_create_context_bound_artifacts_payloads_and_output() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let config_artifact = config_artifact(&fixture.runtime_spec, &node.config_ref).evidence;
    let projections = store::ProjectionSnapshot::default();
    let run_stream = Vec::new();
    let invocation = PreparedRunnerInvocation {
        runtime_spec: &fixture.runtime_spec,
        run_id: &fixture.run_id,
        spec_hash: fixture.runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id: &attempt_id,
        attempt_no: 1,
        config_artifact,
        inputs: MaterializedInputs {
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            root: MaterializedInputNode::Unit,
        },
        caps: CertifiedRuntimeCapabilities::new(
            node.node_id.clone(),
            node.capability_bindings.clone(),
        ),
        recorded_facts: RecordedFacts::default(),
        projections: &projections,
        run_stream: &run_stream,
    };
    let ctx = ErasedRunCtx::from_prepared(&invocation);
    let artifacts = RunnerArtifactBuilder::new(&ctx);
    let payloads = RunnerPayloadBuilder::new(&ctx);
    let value = CertifierValue { amount: 42 };
    let binding = RunnerCapabilityBinding {
        capability_kind: fixture.cap_kind.clone(),
        capability_version: fixture.cap_version.clone(),
        adapter_kind: fixture.adapter_kind.clone(),
        adapter_version: fixture.adapter_version.clone(),
    };

    let state = artifacts.state_output(&value).expect("state output");
    assert_eq!(
        state.evidence().artifact_role,
        events::ArtifactRole::StateOutput
    );
    assert_eq!(
        state.evidence().producer_node_id.as_ref(),
        Some(&ctx.node().node_id)
    );
    assert_eq!(
        artifacts.content_digest(&value).expect("content digest"),
        state.evidence().digest
    );

    let cell = payloads.cell_produced(&state).expect("cell payload");
    match &cell {
        RunnerEventPayload::CellProduced(payload) => {
            assert_eq!(payload.spec_hash, *ctx.spec_hash());
            assert_eq!(payload.node_id, ctx.node().node_id);
            assert_eq!(payload.cell_id, ctx.node().output_cell);
            assert_eq!(payload.schema_id, ctx.output_cell().schema_id);
            assert_eq!(payload.semantic_type_id, ctx.output_cell().semantic_type_id);
            assert_eq!(payload.artifact_id, state.evidence().artifact_id);
            assert_eq!(payload.content_digest, state.evidence().digest);
        }
        _ => panic!("expected cell produced payload"),
    }

    let response = artifacts.fact_response(&value).expect("fact response");
    let fact = payloads
        .fact_recorded(
            events::FactKey::new("mfm.test.runner_kit.fact").expect("fact key"),
            &value,
            &response,
            binding.clone(),
        )
        .expect("fact payload");
    match &fact {
        RunnerEventPayload::FactRecorded(payload) => {
            assert_eq!(payload.node_id, ctx.node().node_id);
            assert_eq!(payload.attempt_id, *ctx.attempt_id());
            assert_eq!(payload.request_hash, state.evidence().digest);
            assert_eq!(payload.response_hash, response.evidence().digest);
            assert_eq!(payload.artifact_id, response.evidence().artifact_id);
        }
        _ => panic!("expected fact recorded payload"),
    }

    let ledger_key =
        events::SideEffectLedgerKey::new("mfm.test.runner_kit.ledger").expect("ledger key");
    let side_effect = RunnerSideEffectBinding {
        ledger_key: ledger_key.clone(),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        invocation_epoch: 1,
    };
    let idempotency_key =
        events::IdempotencyKeyRef::new("mfm.test.runner_kit.idem").expect("idempotency key");
    let owner = events::RunnerInvocationId::new("mfm.test.runner_kit.owner").expect("claim owner");
    let next_owner =
        events::RunnerInvocationId::new("mfm.test.runner_kit.owner.next").expect("claim owner");
    let token =
        events::side_effect::ClaimFencingToken::new("mfm.test.runner_kit.token").expect("token");
    let next_token = events::side_effect::ClaimFencingToken::new("mfm.test.runner_kit.token.next")
        .expect("next token");
    let verifier = events::ReplayVerifierId::new("mfm.test.runner_kit.verifier").expect("verifier");

    let intent = artifacts
        .side_effect_intent(&value)
        .expect("side-effect intent");
    let intent_payload = payloads
        .side_effect_intent_persisted(
            side_effect.clone(),
            &intent,
            &value,
            idempotency_key,
            binding,
        )
        .expect("intent payload");
    match &intent_payload {
        RunnerEventPayload::SideEffectIntentPersisted(payload) => {
            assert_eq!(payload.ledger_key, ledger_key);
            assert_eq!(payload.intent_hash, intent.evidence().digest);
            assert_eq!(payload.intent_artifact_id, intent.evidence().artifact_id);
            assert_eq!(payload.idempotency_input_hash, state.evidence().digest);
        }
        _ => panic!("expected side-effect intent payload"),
    }

    let claim = payloads.side_effect_claimed(
        side_effect.clone(),
        RunnerClaimBinding {
            claim_owner: owner.clone(),
            claim_generation: 1,
            claim_fencing_token: token.clone(),
        },
    );
    match &claim {
        RunnerEventPayload::SideEffectClaimed(payload) => {
            assert_eq!(payload.claim_owner, owner);
            assert_eq!(payload.claim_generation, 1);
            assert_eq!(payload.claim_fencing_token, token);
        }
        _ => panic!("expected side-effect claimed payload"),
    }

    let takeover = payloads.side_effect_claim_taken_over(
        side_effect.clone(),
        RunnerClaimTakeoverBinding {
            previous_claim_owner: owner.clone(),
            new_claim_owner: next_owner.clone(),
            previous_claim_generation: 1,
            claim_generation: 2,
            claim_fencing_token: next_token.clone(),
        },
    );
    match &takeover {
        RunnerEventPayload::SideEffectClaimTakenOver(payload) => {
            assert_eq!(payload.previous_claim_owner, owner);
            assert_eq!(payload.new_claim_owner, next_owner);
            assert_eq!(payload.previous_claim_generation, 1);
            assert_eq!(payload.claim_generation, 2);
            assert_eq!(payload.claim_fencing_token, next_token);
        }
        _ => panic!("expected side-effect claim takeover payload"),
    }

    let prepared = artifacts
        .prepared_invocation(&serde_json::json!({"prepared": true}))
        .expect("prepared invocation");
    assert_eq!(
        prepared.evidence().artifact_role,
        events::ArtifactRole::PreparedInvocation
    );
    assert!(prepared.evidence().schema_id.is_none());
    let prepared_payload = payloads
        .side_effect_invocation_prepared(
            side_effect.clone(),
            Some(&prepared),
            RunnerPreparedInvocationBinding {
                claim_generation: 2,
                claim_fencing_token: next_token.clone(),
                resource_key: None,
            },
        )
        .expect("prepared payload");
    match &prepared_payload {
        RunnerEventPayload::SideEffectInvocationPrepared(payload) => {
            assert_eq!(
                payload.prepared_artifact_id.as_ref(),
                Some(&prepared.evidence().artifact_id)
            );
            assert_eq!(
                payload.prepared_hash.as_ref(),
                Some(&prepared.evidence().digest)
            );
            assert_eq!(payload.claim_generation, 2);
        }
        _ => panic!("expected side-effect invocation prepared payload"),
    }

    let started = payloads.side_effect_invocation_started(
        side_effect.clone(),
        RunnerClaimBinding {
            claim_owner: next_owner.clone(),
            claim_generation: 2,
            claim_fencing_token: next_token.clone(),
        },
    );
    match &started {
        RunnerEventPayload::SideEffectInvocationStarted(payload) => {
            assert_eq!(payload.claim_owner, next_owner);
            assert_eq!(payload.claim_generation, 2);
            assert_eq!(payload.claim_fencing_token, next_token);
        }
        _ => panic!("expected side-effect invocation started payload"),
    }

    let not_submitted = artifacts
        .not_submitted_proof(&value)
        .expect("not-submitted proof");
    let not_submitted_payload = payloads
        .side_effect_not_submitted_proven(side_effect.clone(), &not_submitted)
        .expect("not-submitted payload");
    match &not_submitted_payload {
        RunnerEventPayload::SideEffectNotSubmittedProven(payload) => {
            assert_eq!(payload.proof_hash, not_submitted.evidence().digest);
            assert_eq!(
                payload.proof_artifact_id,
                not_submitted.evidence().artifact_id
            );
        }
        _ => panic!("expected side-effect not-submitted payload"),
    }

    let submission = artifacts.submission(&value).expect("submission");
    let submission_payload = payloads
        .side_effect_submission_observed(side_effect.clone(), &submission)
        .expect("submission payload");
    match &submission_payload {
        RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
            assert_eq!(payload.submission_hash, submission.evidence().digest);
            assert_eq!(
                payload.submission_artifact_id,
                submission.evidence().artifact_id
            );
        }
        _ => panic!("expected side-effect submission payload"),
    }

    let submission_unknown = artifacts
        .submission_unknown(&value)
        .expect("submission unknown evidence");
    let submission_unknown_payload = payloads
        .side_effect_submission_unknown(side_effect.clone(), &submission_unknown)
        .expect("submission unknown payload");
    match &submission_unknown_payload {
        RunnerEventPayload::SideEffectSubmissionUnknown(payload) => {
            assert_eq!(payload.evidence_hash, submission_unknown.evidence().digest);
            assert_eq!(
                payload.evidence_artifact_id,
                submission_unknown.evidence().artifact_id
            );
        }
        _ => panic!("expected side-effect submission unknown payload"),
    }

    let receipt = artifacts.receipt(&value).expect("receipt");
    let receipt_payload = payloads
        .side_effect_receipt_observed(side_effect.clone(), &receipt, verifier.clone(), None)
        .expect("receipt payload");
    match &receipt_payload {
        RunnerEventPayload::SideEffectReceiptObserved(payload) => {
            assert_eq!(payload.receipt_hash, receipt.evidence().digest);
            assert_eq!(payload.receipt_artifact_id, receipt.evidence().artifact_id);
            assert_eq!(payload.replay_verifier_id, verifier);
        }
        _ => panic!("expected side-effect receipt payload"),
    }

    let confirmation = artifacts.confirmation(&value).expect("confirmation");
    let confirmation_payload = payloads
        .side_effect_confirmation_observed(
            side_effect.clone(),
            &confirmation,
            verifier.clone(),
            None,
        )
        .expect("confirmation payload");
    match &confirmation_payload {
        RunnerEventPayload::SideEffectConfirmationObserved(payload) => {
            assert_eq!(payload.confirmation_hash, confirmation.evidence().digest);
            assert_eq!(
                payload.confirmation_artifact_id,
                confirmation.evidence().artifact_id
            );
            assert_eq!(payload.replay_verifier_id, verifier);
        }
        _ => panic!("expected side-effect confirmation payload"),
    }

    let ambiguity = artifacts
        .ambiguity_evidence(&value)
        .expect("ambiguity evidence");
    let ambiguity_payload = payloads
        .side_effect_ambiguous(
            side_effect.clone(),
            events::AmbiguityCode::new("runner_kit_test").expect("ambiguity code"),
            &ambiguity,
        )
        .expect("ambiguity payload");
    match &ambiguity_payload {
        RunnerEventPayload::SideEffectAmbiguous(payload) => {
            assert_eq!(payload.evidence_hash, ambiguity.evidence().digest);
            assert_eq!(
                payload.evidence_artifact_id,
                ambiguity.evidence().artifact_id
            );
        }
        _ => panic!("expected side-effect ambiguous payload"),
    }

    let failed = payloads.side_effect_failed(
        side_effect.clone(),
        events::side_effect::FailurePhase::BeforeInvocationStarted,
        true,
        events::MfmErrorInfo::new(
            events::ErrorCode::new("runner_kit_failure").expect("error code"),
            events::ErrorCategory::Runtime,
            true,
            "runner kit failure",
        )
        .expect("error info"),
    );
    match &failed {
        RunnerEventPayload::SideEffectFailed(payload) => {
            assert_eq!(payload.ledger_key, ledger_key);
            assert_eq!(
                payload.failure_phase,
                events::side_effect::FailurePhase::BeforeInvocationStarted
            );
            assert!(payload.retryable);
            assert_eq!(payload.error.safe_message, "runner kit failure");
        }
        _ => panic!("expected side-effect failed payload"),
    }

    let role_mismatch = payloads
        .cell_produced(&response)
        .expect_err("fact response cannot produce a cell");
    assert!(matches!(
        role_mismatch,
        RuntimeError::InvalidRunnerOutput(message)
            if message.contains("fact_response")
                && message.contains("state_output")
    ));

    let mut state_output = RunnerOutputBuilder::new(&ctx);
    state_output
        .stage_attempt_artifact(&state)
        .expect("stage state output")
        .retain_runtime_evidence(&state)
        .payload(cell.clone());
    let state_output = state_output.finish();
    assert_eq!(state_output.staged_artifacts.len(), 1);
    assert_eq!(state_output.staged_retention_refs.len(), 1);
    assert_eq!(state_output.payloads, vec![cell]);

    let mut side_effect_output = RunnerOutputBuilder::new(&ctx);
    side_effect_output
        .stage_side_effect_artifact(&intent, ledger_key, 1)
        .expect("stage side-effect artifact")
        .retain_runtime_evidence(&intent)
        .payload(intent_payload);
    let side_effect_output = side_effect_output.finish();
    assert_eq!(side_effect_output.staged_artifacts.len(), 1);
    assert_eq!(side_effect_output.staged_retention_refs.len(), 1);
    assert_eq!(side_effect_output.payloads.len(), 1);
}

#[test]
fn side_effect_evidence_builder_prepares_and_stages_claimed_invocation() {
    let fixture = fixture();
    let ledger_key = events::SideEffectLedgerKey::new("mfm.test.side_effect_builder.ledger")
        .expect("ledger key");
    let owner =
        events::RunnerInvocationId::new("mfm.test.side_effect_builder.owner").expect("claim owner");
    let token = events::side_effect::ClaimFencingToken::new("mfm.test.side_effect_builder.token")
        .expect("token");
    let idempotency_key = events::IdempotencyKeyRef::new("mfm.test.side_effect_builder.idem")
        .expect("idempotency key");
    let capability_binding = RunnerCapabilityBinding {
        capability_kind: fixture.cap_kind.clone(),
        capability_version: fixture.cap_version.clone(),
        adapter_kind: fixture.adapter_kind.clone(),
        adapter_version: fixture.adapter_version.clone(),
    };
    let intent = CertifierValue { amount: 7 };
    let idempotency = CertifierValue { amount: 11 };
    let prepared = serde_json::json!({"prepared": true});

    with_runner_erased_ctx(&fixture, &fixture.cell_a, |ctx| {
        let output = SideEffectEvidenceBuilder::new(&ctx)
            .prepare_invocation_and_start(SideEffectPreparedInvocationEvidence {
                side_effect: RunnerSideEffectBinding {
                    ledger_key: ledger_key.clone(),
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    invocation_epoch: 3,
                },
                claim: RuntimeSideEffectClaimAuthority {
                    claim_owner: owner.clone(),
                    claim_generation: 9,
                    claim_fencing_token: token.clone(),
                    resource_key: None,
                },
                intent: &intent,
                idempotency: &idempotency,
                idempotency_key: idempotency_key.clone(),
                capability_binding,
                prepared_invocation: &prepared,
            })
            .expect("prepare side-effect evidence");

        assert_eq!(output.staged_artifacts.len(), 2);
        assert_eq!(output.staged_retention_refs.len(), 2);
        assert_eq!(output.payloads.len(), 4);
        match &output.payloads[0] {
            RunnerEventPayload::SideEffectIntentPersisted(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(
                    payload.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                );
                assert_eq!(payload.invocation_epoch, 3);
                assert_eq!(payload.idempotency_key, idempotency_key);
                assert_eq!(payload.capability_kind, fixture.cap_kind);
                assert_eq!(payload.adapter_kind, fixture.adapter_kind);
            }
            other => panic!("expected side-effect intent payload: {other:?}"),
        }
        match &output.payloads[1] {
            RunnerEventPayload::SideEffectClaimed(payload) => {
                assert_eq!(payload.claim_owner, owner);
                assert_eq!(payload.claim_generation, 9);
                assert_eq!(payload.claim_fencing_token, token);
            }
            other => panic!("expected side-effect claimed payload: {other:?}"),
        }
        match &output.payloads[2] {
            RunnerEventPayload::SideEffectInvocationPrepared(payload) => {
                assert!(payload.prepared_artifact_id.is_some());
                assert!(payload.prepared_hash.is_some());
                assert_eq!(payload.claim_generation, 9);
                assert_eq!(payload.claim_fencing_token, token);
            }
            other => panic!("expected side-effect prepared payload: {other:?}"),
        }
        match &output.payloads[3] {
            RunnerEventPayload::SideEffectInvocationStarted(payload) => {
                assert_eq!(payload.claim_owner, owner);
                assert_eq!(payload.claim_generation, 9);
                assert_eq!(payload.claim_fencing_token, token);
            }
            other => panic!("expected side-effect started payload: {other:?}"),
        }
    });
}

#[test]
fn side_effect_evidence_builder_builds_progress_evidence_with_replay_and_resources() {
    let fixture = fixture();
    let ledger_key = events::SideEffectLedgerKey::new("mfm.test.side_effect_builder.progress")
        .expect("ledger key");
    let verifier =
        events::ReplayVerifierId::new("mfm.test.side_effect_builder.verifier").expect("verifier");
    let touched_set = events::ResourceTouchedSetEvidence {
        namespace: exact_touched_set_resource_namespace(),
        evidence_schema_id: fixture.seed_ref.schema_id.clone(),
        evidence_hash: content(0xd1),
        evidence_artifact_id: artifact(0xd2),
    };
    let value = CertifierValue { amount: 17 };

    with_runner_erased_ctx(&fixture, &fixture.cell_a, |ctx| {
        let builder = SideEffectEvidenceBuilder::new(&ctx);
        let side_effect = RunnerSideEffectBinding {
            ledger_key: ledger_key.clone(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            invocation_epoch: 4,
        };

        match single_side_effect_payload(
            &builder
                .submission_observed(side_effect.clone(), &value)
                .expect("submission evidence"),
        ) {
            RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(payload.invocation_epoch, 4);
            }
            other => panic!("expected submission observed payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .submission_unknown(side_effect.clone(), &value)
                .expect("submission unknown evidence"),
        ) {
            RunnerEventPayload::SideEffectSubmissionUnknown(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(payload.invocation_epoch, 4);
            }
            other => panic!("expected submission unknown payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .not_submitted_proven(side_effect.clone(), &value)
                .expect("not-submitted evidence"),
        ) {
            RunnerEventPayload::SideEffectNotSubmittedProven(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(payload.invocation_epoch, 4);
            }
            other => panic!("expected not-submitted payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .receipt_observed(
                    side_effect.clone(),
                    &value,
                    SideEffectReplayEvidence {
                        replay_verifier_id: verifier.clone(),
                        resource_touched_set: Some(touched_set.clone()),
                    },
                )
                .expect("receipt evidence"),
        ) {
            RunnerEventPayload::SideEffectReceiptObserved(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(payload.invocation_epoch, 4);
                assert_eq!(payload.replay_verifier_id, verifier);
                assert_eq!(payload.resource_touched_set.as_ref(), Some(&touched_set));
            }
            other => panic!("expected receipt payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .confirmation_observed(
                    side_effect.clone(),
                    &value,
                    SideEffectReplayEvidence {
                        replay_verifier_id: verifier.clone(),
                        resource_touched_set: Some(touched_set.clone()),
                    },
                )
                .expect("confirmation evidence"),
        ) {
            RunnerEventPayload::SideEffectConfirmationObserved(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(payload.invocation_epoch, 4);
                assert_eq!(payload.replay_verifier_id, verifier);
                assert_eq!(payload.resource_touched_set.as_ref(), Some(&touched_set));
            }
            other => panic!("expected confirmation payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .ambiguous(
                    side_effect,
                    events::AmbiguityCode::new("side_effect_builder_test").expect("ambiguity code"),
                    &value,
                )
                .expect("ambiguity evidence"),
        ) {
            RunnerEventPayload::SideEffectAmbiguous(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(payload.invocation_epoch, 4);
            }
            other => panic!("expected ambiguity payload: {other:?}"),
        }
    });
}

#[derive(Clone)]
enum TestSubmissionDecision {
    Observed,
    Unknown,
    NotSubmitted,
    Ambiguous,
}

#[derive(Clone)]
struct TestSideEffectDriverCallbacks {
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    submission_decision: TestSubmissionDecision,
}

impl TestSideEffectDriverCallbacks {
    fn new(fixture: &Fixture) -> Self {
        Self {
            cap_kind: side_effect_capability_kind(),
            cap_version: side_effect_capability_version(),
            adapter_kind: fixture.adapter_kind.clone(),
            adapter_version: fixture.adapter_version.clone(),
            submission_decision: TestSubmissionDecision::Observed,
        }
    }

    fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.submission_decision = decision;
        self
    }

    fn intent_plan_for(
        &self,
        node_id: String,
        attempt_id: String,
    ) -> Result<SideEffectIntentPlan<FixtureSideEffectEvidence, FixtureSideEffectEvidence>> {
        Ok(SideEffectIntentPlan {
            intent: FixtureSideEffectEvidence {
                amount: 21,
                node_id: node_id.clone(),
                attempt_id: attempt_id.clone(),
            },
            idempotency: FixtureSideEffectEvidence {
                amount: 34,
                node_id,
                attempt_id,
            },
            idempotency_key: events::IdempotencyKeyRef::new("mfm.test.driver.idem")
                .expect("idempotency key"),
            capability_binding: RunnerCapabilityBinding {
                capability_kind: self.cap_kind.clone(),
                capability_version: self.cap_version.clone(),
                adapter_kind: self.adapter_kind.clone(),
                adapter_version: self.adapter_version.clone(),
            },
        })
    }
}

impl SideEffectDriverCallbacks for TestSideEffectDriverCallbacks {
    type Intent = FixtureSideEffectEvidence;
    type Idempotency = FixtureSideEffectEvidence;
    type PreparedInvocation = serde_json::Value;
    type Submission = FixtureSideEffectEvidence;
    type SubmissionUnknownEvidence = FixtureSideEffectEvidence;
    type NotSubmittedProof = FixtureSideEffectEvidence;
    type Receipt = FixtureSideEffectEvidence;
    type Confirmation = FixtureSideEffectEvidence;
    type AmbiguityEvidence = FixtureSideEffectEvidence;
    type Output = FixtureOutputValue;

    fn intent_and_idempotency<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
    ) -> SideEffectDriverFuture<'a, SideEffectIntentPlan<Self::Intent, Self::Idempotency>> {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move { self.intent_plan_for(node_id, attempt_id) })
    }

    fn prepare_invocation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _plan: &'a SideEffectIntentPlan<Self::Intent, Self::Idempotency>,
    ) -> SideEffectDriverFuture<'a, SideEffectPreparedInvocationPlan<Self::PreparedInvocation>>
    {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(SideEffectPreparedInvocationPlan::with_prepared_invocation(
                serde_json::json!({
                    "attempt_id": attempt_id,
                    "node_id": node_id,
                    "prepared": true
                }),
            ))
        })
    }

    fn reconstruct_prepared_invocation<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation> {
        let prepared_artifact_id = prepared.artifact_id.clone();
        Box::pin(async move {
            Ok(serde_json::json!({
                "prepared_artifact_id": prepared_artifact_id.as_str()
            }))
        })
    }

    fn submit_or_recover_submission<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _action: SideEffectProtocolAction,
        _prepared: Option<Self::PreparedInvocation>,
    ) -> SideEffectSubmissionDecisionFuture<
        'a,
        Self::Submission,
        Self::SubmissionUnknownEvidence,
        Self::NotSubmittedProof,
        Self::AmbiguityEvidence,
    > {
        let decision = self.submission_decision.clone();
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(match decision {
                TestSubmissionDecision::Observed => {
                    SideEffectSubmissionDecision::Observed(FixtureSideEffectEvidence {
                        amount: 55,
                        node_id: node_id.clone(),
                        attempt_id: attempt_id.clone(),
                    })
                }
                TestSubmissionDecision::Unknown => {
                    SideEffectSubmissionDecision::Unknown(FixtureSideEffectEvidence {
                        amount: 56,
                        node_id: node_id.clone(),
                        attempt_id: attempt_id.clone(),
                    })
                }
                TestSubmissionDecision::NotSubmitted => {
                    SideEffectSubmissionDecision::NotSubmitted(FixtureSideEffectEvidence {
                        amount: 57,
                        node_id: node_id.clone(),
                        attempt_id: attempt_id.clone(),
                    })
                }
                TestSubmissionDecision::Ambiguous => SideEffectSubmissionDecision::Ambiguous {
                    ambiguity_code: events::AmbiguityCode::new("mfm_test_driver_ambiguous")
                        .expect("ambiguity code"),
                    evidence: FixtureSideEffectEvidence {
                        amount: 58,
                        node_id,
                        attempt_id,
                    },
                },
            })
        })
    }

    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(SideEffectObservedEvidence {
                evidence: FixtureSideEffectEvidence {
                    amount: 89,
                    node_id,
                    attempt_id,
                },
                replay: test_driver_replay_evidence(),
            })
        })
    }

    fn build_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(SideEffectObservedEvidence {
                evidence: FixtureSideEffectEvidence {
                    amount: 144,
                    node_id,
                    attempt_id,
                },
                replay: test_driver_replay_evidence(),
            })
        })
    }

    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(FixtureOutputValue {
                amount: 233,
                node_id,
                attempt_id,
            })
        })
    }
}

fn test_driver_replay_evidence() -> SideEffectReplayEvidence {
    SideEffectReplayEvidence {
        replay_verifier_id: events::ReplayVerifierId::new("mfm.test.driver.verifier")
            .expect("verifier"),
        resource_touched_set: None,
    }
}

fn test_driver_resource_key_for_node(node: &spec::NodeSpec) -> Option<events::ResourceKeyEvidence> {
    let Some(spec::SideEffectContractSpec {
        resource_claim:
            spec::ResourceClaimSpec::Exclusive {
                namespace,
                key_schema,
            },
        ..
    }) = &node.side_effect
    else {
        return None;
    };
    Some(events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema.clone(),
        key: events::ResourceKey::new("mfm.test.driver.shared-resource").expect("resource key"),
    })
}

#[tokio::test]
async fn side_effect_driver_prepares_and_starts_one_step() {
    let fixture = fixture_with_first_side_effect_state();
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output = drive_side_effect_driver_empty(&fixture, &fixture.cell_a, &callbacks)
        .await
        .expect("driver output");

    assert_eq!(output.staged_artifacts.len(), 2);
    assert_eq!(output.payloads.len(), 4);
    assert!(matches!(
        output.payloads[0],
        RunnerEventPayload::SideEffectIntentPersisted(_)
    ));
    assert!(matches!(
        output.payloads[1],
        RunnerEventPayload::SideEffectClaimed(_)
    ));
    assert!(matches!(
        output.payloads[2],
        RunnerEventPayload::SideEffectInvocationPrepared(_)
    ));
    assert!(matches!(
        output.payloads[3],
        RunnerEventPayload::SideEffectInvocationStarted(_)
    ));
    match &output.payloads[0] {
        RunnerEventPayload::SideEffectIntentPersisted(payload) => {
            assert_eq!(
                payload.ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            );
            assert_eq!(payload.invocation_epoch, 1);
        }
        other => panic!("expected intent payload: {other:?}"),
    }
}

#[tokio::test]
async fn side_effect_driver_preserves_concrete_exclusive_resource_key_across_runs() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let mut peer = fixture.clone();
    refresh_fixture_run_id_with_distinct(&mut peer, Some(content(0xf6)));
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();

    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start first run");
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .expect("first run claims resource lane"),
        SchedulerStatus::Advanced
    );
    let first_stream = store.load_run_stream(&fixture.run_id);
    let claim_seq = first_stream
        .iter()
        .find_map(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ResourceLaneClaimed(_)
            )
            .then_some(event.seq())
        })
        .expect("first run recorded resource lane claim");
    let prepared_seq = first_stream.iter().find_map(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectInvocationPrepared(_)
        )
        .then_some(event.seq())
    });
    assert!(
        prepared_seq.is_none_or(|prepared_seq| claim_seq < prepared_seq),
        "exclusive invocation prepare must be committed after ResourceLaneClaimed"
    );
    let resource_key = first_stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::ResourceLaneClaimed(payload) => {
                Some(payload.resource_key.clone())
            }
            _ => None,
        })
        .expect("first run recorded resource key");
    assert_eq!(resource_key.key.as_str(), "mfm.test.driver.shared-resource");
    let lane_key = store::ResourceLaneKey::from_evidence(&resource_key);
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());

    start_fixture_run(&scheduler, &mut store, &peer, vec![peer.seed_ref.clone()])
        .await
        .expect("start peer run");
    assert_eq!(
        drive_once(&scheduler, &mut store, &peer.runtime_spec, &peer.run_id)
            .await
            .expect("peer run starts attempt before observing lane block"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        drive_once(&scheduler, &mut store, &peer.runtime_spec, &peer.run_id)
            .await
            .expect("peer run blocks on same resource lane"),
        SchedulerStatus::Blocked
    );
    assert!(
        store
            .load_run_stream(&peer.run_id)
            .iter()
            .all(|event| !matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectInvocationPrepared(_)
            )),
        "peer run must not prepare while the cross-run resource lane is held"
    );
}

#[tokio::test]
async fn exclusive_side_effect_prepare_failure_after_claim_terminalizes_attempt() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        FailingAfterPreclaimRunner::new(&fixture),
    ));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .expect("exclusive run terminalizes failed resource-lane claim"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    let stream = store.load_run_stream(&fixture.run_id);
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ResourceLaneClaimed(_)
        )),
        "failed exclusive attempt records the resource lane claim"
    );
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ResourceLaneReleased(_)
        )),
        "failed exclusive attempt releases the resource lane"
    );
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectFailed(_)
        )),
        "failed exclusive attempt records terminal side-effect evidence"
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
    assert!(
        store
            .projection_snapshot()
            .resource_lanes()
            .next()
            .is_none(),
        "terminal attempt failure releases the exclusive resource lane"
    );
}

#[tokio::test]
async fn side_effect_driver_submits_from_started_projection() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-submit",
        "sidefx-driver-submit",
    );
    append_synthetic_invocation_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-submit-started",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");

    assert_eq!(output.payloads.len(), 1);
    match &output.payloads[0] {
        RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
            assert_eq!(payload.ledger_key, ledger_key);
            assert_eq!(payload.invocation_epoch, 1);
        }
        other => panic!("expected submission payload: {other:?}"),
    }
}

#[tokio::test]
async fn side_effect_driver_persists_submission_recovery_decisions() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-recovery",
        "sidefx-driver-recovery",
    );
    append_synthetic_invocation_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-recovery-started",
    );

    for (decision, expected) in [
        (
            TestSubmissionDecision::Unknown,
            "side_effect.submission_unknown",
        ),
        (
            TestSubmissionDecision::NotSubmitted,
            "side_effect.not_submitted_proven",
        ),
        (TestSubmissionDecision::Ambiguous, "side_effect.ambiguous"),
    ] {
        let callbacks =
            TestSideEffectDriverCallbacks::new(&fixture).with_submission_decision(decision);
        let output =
            drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
                .await
                .expect("driver output");
        let actual = output
            .payloads
            .iter()
            .find_map(|payload| match payload {
                RunnerEventPayload::SideEffectSubmissionUnknown(_) => {
                    Some("side_effect.submission_unknown")
                }
                RunnerEventPayload::SideEffectNotSubmittedProven(_) => {
                    Some("side_effect.not_submitted_proven")
                }
                RunnerEventPayload::SideEffectAmbiguous(_) => Some("side_effect.ambiguous"),
                RunnerEventPayload::ResourceLaneReleaseIntent(_) => None,
                _ => None,
            })
            .unwrap_or_else(|| panic!("unexpected recovery payloads: {:?}", output.payloads));
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn side_effect_driver_maps_confirmation_to_state_output() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-output",
        "sidefx-driver-output",
    );
    append_synthetic_invocation_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-output-started",
    );
    append_synthetic_submission_observed(
        &mut store,
        &fixture,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-output-submission",
    );
    append_synthetic_receipt_observed(
        &mut store,
        &fixture,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-output-receipt",
    );
    append_synthetic_confirmation_observed(
        &mut store,
        &fixture,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-output-confirmation",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");

    assert_eq!(output.staged_artifacts.len(), 1);
    assert_eq!(output.payloads.len(), 1);
    assert!(matches!(
        output.payloads[0],
        RunnerEventPayload::CellProduced(_)
    ));
}

#[tokio::test]
async fn side_effect_driver_idles_on_ambiguous_projection() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-ambiguous",
        "sidefx-driver-ambiguous",
    );
    append_synthetic_invocation_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-ambiguous-started",
    );
    append_synthetic_ambiguous(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-ambiguous-terminal",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");

    assert!(output.payloads.is_empty());
    assert!(output.staged_artifacts.is_empty());
}

#[tokio::test]
async fn side_effect_driver_starts_and_submits_from_prepared_projection() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-unsupported",
        "sidefx-driver-unsupported",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");

    assert_eq!(output.staged_artifacts.len(), 1);
    assert_eq!(output.payloads.len(), 2);
    match &output.payloads[0] {
        RunnerEventPayload::SideEffectInvocationStarted(payload) => {
            assert_eq!(payload.ledger_key, ledger_key);
            assert_eq!(payload.invocation_epoch, 1);
        }
        other => panic!("expected invocation started payload: {other:?}"),
    }
    match &output.payloads[1] {
        RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
            assert_eq!(payload.ledger_key, ledger_key);
            assert_eq!(payload.invocation_epoch, 1);
        }
        other => panic!("expected submission payload: {other:?}"),
    }

    let required_artifacts = output
        .staged_artifacts
        .iter()
        .map(|artifact| artifact.evidence().clone())
        .collect::<Vec<_>>();
    let payloads = output
        .payloads
        .into_iter()
        .map(events::KernelEventPayload::from)
        .collect::<Vec<_>>();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("sidefx-driver-prepared-resume")
                .expect("commit key"),
            payloads: payloads,
            required_artifacts: required_artifacts,
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    ledger_key: ledger_key.clone(),
                    required: store::RequiredSideEffectState::InvocationPrepared,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append prepared recovery output");

    let projection_snapshot = store.projection_snapshot();
    let projection = projection_snapshot
        .side_effect_state_for_run(&fixture.run_id, &ledger_key)
        .expect("side-effect state")
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase(),
        store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::Observed { .. },
            ..
        }
    ));
}

fn with_runner_erased_ctx<R, F>(fixture: &Fixture, cell_id: &CellId, test: F) -> R
where
    F: for<'a> FnOnce(ErasedRunCtx<'a>) -> R,
{
    let node = node_by_output(fixture, cell_id);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let config_artifact = config_artifact(&fixture.runtime_spec, &node.config_ref).evidence;
    let projections = store::ProjectionSnapshot::default();
    let run_stream = Vec::new();
    let invocation = PreparedRunnerInvocation {
        runtime_spec: &fixture.runtime_spec,
        run_id: &fixture.run_id,
        spec_hash: fixture.runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id: &attempt_id,
        attempt_no: 1,
        config_artifact,
        inputs: MaterializedInputs {
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            root: MaterializedInputNode::Unit,
        },
        caps: CertifiedRuntimeCapabilities::new(
            node.node_id.clone(),
            node.capability_bindings.clone(),
        ),
        recorded_facts: RecordedFacts::default(),
        projections: &projections,
        run_stream: &run_stream,
    };
    test(ErasedRunCtx::from_prepared(&invocation))
}

async fn drive_side_effect_driver_empty<C>(
    fixture: &Fixture,
    cell_id: &CellId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectDriverCallbacks + ?Sized,
{
    let node = node_by_output(fixture, cell_id);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let config_artifact = config_artifact(&fixture.runtime_spec, &node.config_ref).evidence;
    let projections = store::ProjectionSnapshot::default();
    let run_stream = Vec::new();
    let invocation = PreparedRunnerInvocation {
        runtime_spec: &fixture.runtime_spec,
        run_id: &fixture.run_id,
        spec_hash: fixture.runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id: &attempt_id,
        attempt_no: 1,
        config_artifact,
        inputs: MaterializedInputs {
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            root: MaterializedInputNode::Unit,
        },
        caps: CertifiedRuntimeCapabilities::new(
            node.node_id.clone(),
            node.capability_bindings.clone(),
        ),
        recorded_facts: RecordedFacts::default(),
        projections: &projections,
        run_stream: &run_stream,
    };
    SideEffectDriver::drive(ErasedRunCtx::from_prepared(&invocation), callbacks).await
}

async fn drive_side_effect_driver_from_store<C>(
    fixture: &Fixture,
    store: &TestTypedRunStore,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectDriverCallbacks + ?Sized,
{
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let config_artifact = config_artifact(&fixture.runtime_spec, &node.config_ref).evidence;
    let projections = store.projection_snapshot().clone();
    let run_stream = store.load_run_stream(&fixture.run_id);
    let invocation = PreparedRunnerInvocation {
        runtime_spec: &fixture.runtime_spec,
        run_id: &fixture.run_id,
        spec_hash: fixture.runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id,
        attempt_no: 1,
        config_artifact,
        inputs: MaterializedInputs {
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            root: MaterializedInputNode::Unit,
        },
        caps: CertifiedRuntimeCapabilities::new(
            node.node_id.clone(),
            node.capability_bindings.clone(),
        ),
        recorded_facts: RecordedFacts::default(),
        projections: &projections,
        run_stream: &run_stream,
    };
    SideEffectDriver::drive(ErasedRunCtx::from_prepared(&invocation), callbacks).await
}

fn single_side_effect_payload(output: &ErasedRunnerOutput) -> &RunnerEventPayload {
    assert_eq!(output.staged_artifacts.len(), 1);
    assert_eq!(output.staged_retention_refs.len(), 1);
    assert_eq!(output.payloads.len(), 1);
    &output.payloads[0]
}

#[test]
fn runner_registration_builder_preserves_explicit_binding_authority() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_b);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let factory_id = events::RunnerFactoryId::new("read").expect("factory");
    let executable = events::ExecutableIdentity {
        factory_id: factory_id.clone(),
        cargo_package_digest: content(0xe1),
        binary_digest: content(0xe2),
        nix_derivation_hash: None,
        nix_output_hash: None,
    };
    let implementation_id = CapabilityImplementationId::new("mfm.test.runner-kit-registration")
        .expect("implementation id");
    let mut registry = ErasedRunnerRegistry::new();

    RunnerRegistrationBuilder::new(&mut registry, implementation_id.clone())
        .register_descriptor(
            node.descriptor_id.clone(),
            &node.capability_bindings,
            factory_id.clone(),
            executable.clone(),
            Arc::new(RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            }),
        )
        .expect("runner registration");

    let binding = registry
        .resolve(node, descriptor)
        .expect("registered runner");
    assert_eq!(binding.factory_id(), &factory_id);
    assert_eq!(binding.executable(), &executable);
    let capabilities = registry
        .resolve_capability_implementations(node)
        .expect("capability implementations");
    assert_eq!(
        capabilities.len(),
        node.capability_bindings.capabilities.len()
    );
    for binding in capabilities {
        assert_eq!(binding.implementation_id(), &implementation_id);
    }

    let wrong_factory = events::RunnerFactoryId::new("pure").expect("factory");
    let mut mismatch_registry = ErasedRunnerRegistry::new();
    let error = match RunnerRegistrationBuilder::new(
        &mut mismatch_registry,
        CapabilityImplementationId::new("mfm.test.runner-kit-registration-mismatch")
            .expect("implementation id"),
    )
    .register_descriptor(
        node.descriptor_id.clone(),
        &node.capability_bindings,
        factory_id,
        events::ExecutableIdentity {
            factory_id: wrong_factory,
            ..executable
        },
        Arc::new(RecordingRunner {
            expected_caps: Vec::new(),
            output_artifact: artifact(0xc1),
            output_digest: content(0xc2),
        }),
    ) {
        Ok(_) => panic!("factory mismatch should be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        RuntimeError::RunnerBinding(message)
            if message.contains("does not match binding factory")
    ));
}

#[tokio::test]
async fn serial_scheduler_runs_nodes_in_certified_topological_order() {
    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("drive a"),
        SchedulerStatus::Advanced
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_none());
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("drive b"),
        SchedulerStatus::Advanced
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_some());
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=0 completed=2 failed=0 interrupted=0 total=2] cells=2 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn scheduler_completes_run_after_public_output_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_until_blocked(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("drive to public output"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.render_cell)
        .is_some());
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Completed attempts[started=0 completed=5 failed=0 interrupted=0 total=5] cells=5 side_effects=0 lanes[run=0 total=0] public_outputs=1 retentions=1"
    );

    let stream = store.load_run_stream(&fixture.run_id);
    let public_output_pos = stream
        .iter()
        .position(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::PublicOutputProduced(_)
            )
        })
        .expect("public output produced");
    let completed_pos = stream
        .iter()
        .position(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
        .expect("run completed");
    assert!(public_output_pos < completed_pos);

    let public_event = &stream[public_output_pos];
    let public_payload = match public_event.payload() {
        events::KernelEventPayload::PublicOutputProduced(payload) => payload,
        _ => unreachable!("checked above"),
    };
    assert_eq!(public_payload.node_id, fixture.render_node);
    assert_eq!(public_payload.receipt_cell_id, fixture.render_cell);
    assert!(public_payload.rendered_artifact_id.is_none());

    let completed_payload = match stream[completed_pos].payload() {
        events::KernelEventPayload::RunCompleted(payload) => payload,
        _ => unreachable!("checked above"),
    };
    assert_eq!(
        completed_payload.outcome,
        events::RunCompletionOutcome::Completed(Box::new(events::PublicOutputCompletionEvidence {
            public_output_schema_id: fixture
                .runtime_spec
                .spec()
                .public_outputs
                .public_schema_id
                .clone(),
            public_output_event_id: public_event.event_id().clone(),
        }))
    );
    assert_eq!(completed_pos, stream.len() - 1);

    let complete_node = certified_complete_run_node(&fixture.runtime_spec).expect("complete node");
    let completion_seq = stream[completed_pos].seq();
    let completion_key = stream[completed_pos].commit_key().clone();
    let completion_commit = stream
        .iter()
        .filter(|event| event.seq() == completion_seq && event.commit_key() == &completion_key)
        .collect::<Vec<_>>();
    assert_eq!(completion_commit.len(), 4);
    let complete_attempt = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == complete_node.node_id && event.seq() < completion_seq =>
            {
                Some(payload.attempt_id.clone())
            }
            _ => None,
        })
        .expect("complete attempt started");
    let complete_receipt = completion_commit
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == complete_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("complete receipt cell");
    assert_eq!(complete_receipt.attempt_id, complete_attempt);
    assert_eq!(complete_receipt.cell_id, complete_node.output_cell);
    assert!(completion_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.node_id == complete_node.node_id
                    && payload.attempt_id == complete_attempt
                    && payload.output_cell_id == complete_node.output_cell
        )
    }));
    assert!(completion_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(&complete_node.node_id)
                    && payload.attempt_id.as_ref() == Some(&complete_attempt)
                    && payload.artifact_ref.artifact_id == complete_receipt.artifact_id
                    && payload.artifact_ref.content_digest == complete_receipt.content_digest
                    && payload.artifact_ref.role == events::ArtifactRole::StateOutput
        )
    }));
    let stream_len = stream.len();
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("drive completed run"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(store.load_run_stream(&fixture.run_id).len(), stream_len);
}

#[tokio::test]
async fn no_second_authority_full_run_stages_and_admits_first_artifact_references() {
    struct InlineRecordingRunner {
        expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for InlineRecordingRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                for (kind, version) in &self.expected_caps {
                    assert!(ctx.caps().contains(kind, version));
                }
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        artifact.artifact_id.clone(),
                        artifact.digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            InlineRecordingRunner {
                expected_caps: Vec::new(),
                output_bytes: br#"{"node":"a"}"#.to_vec(),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            InlineRecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_bytes: br#"{"node":"b"}"#.to_vec(),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry, &fixture),
        Arc::new(RecordingRuntimeArtifactStore {
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }),
    );
    let store = RecordingTypedRunStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start run");

    assert_eq!(
        scheduler
            .drive_until_blocked(&store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive full representative run"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        store
            .projection_snapshot(&fixture.run_id)
            .await
            .run_state(&fixture.run_id),
        store::RunState::Completed
    );

    let stream = store.load_run_stream(&fixture.run_id).await;
    validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("representative stream validates");
    assert_every_certified_node_has_attempt(&fixture.runtime_spec, &stream);
    assert!(node_by_output(&fixture, &fixture.cell_a)
        .framework
        .is_none());
    assert!(matches!(
        &node_by_output(&fixture, &fixture.render_cell).framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    ));
    assert!(matches!(
        &certified_retention_manifest_node(&fixture.runtime_spec)
            .expect("retention node")
            .framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    ));
    assert!(matches!(
        &certified_complete_run_node(&fixture.runtime_spec)
            .expect("complete node")
            .framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    ));

    let mut first_reference_by_artifact = BTreeMap::<ArtifactId, usize>::new();
    let commits = store.commits();
    for (commit_index, commit) in commits.iter().enumerate() {
        let commit_references = commit
            .payloads
            .iter()
            .flat_map(referenced_artifact_ids_for_payload)
            .collect::<BTreeSet<_>>();
        assert!(
            !commit_references.is_empty() || commit.admitted_artifacts.is_empty(),
            "commit {} admitted artifacts without same-commit references",
            commit.commit_key
        );
        for admitted in &commit.admitted_artifacts {
            assert!(
                commit_references.contains(&admitted.artifact_id),
                "commit {} admitted unreferenced artifact {}",
                commit.commit_key,
                admitted.artifact_id
            );
        }
        for artifact_id in commit_references {
            first_reference_by_artifact
                .entry(artifact_id)
                .or_insert(commit_index);
        }
    }

    assert!(
        !first_reference_by_artifact.is_empty(),
        "representative run should reference artifacts"
    );
    for (artifact_id, commit_index) in first_reference_by_artifact {
        let commit = &commits[commit_index];
        assert!(
            commit
                .admitted_artifacts
                .iter()
                .any(|admitted| admitted.artifact_id == artifact_id),
            "artifact {artifact_id} was first referenced by {} at seq {} but not admitted there",
            commit.commit_key,
            commit.seq.as_u64()
        );
    }
}

#[tokio::test]
async fn run_launch_commits_single_admission_root_and_admits_launch_artifacts() {
    let fixture = fixture();
    let scheduler = test_scheduler_with_artifacts(
        registered_fixture_runners(&fixture),
        Arc::new(RecordingRuntimeArtifactStore {
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }),
    );
    let mut store = TestTypedRunStore::new();

    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let stream = store.load_run_stream(&fixture.run_id);
    let first = stream.first().expect("stream event");
    let admission_commit = stream
        .iter()
        .take_while(|event| event.seq() == first.seq() && event.commit_key() == first.commit_key())
        .collect::<Vec<_>>();
    assert_eq!(admission_commit.len(), 1);
    assert_eq!(admission_commit[0].ordinal(), store::CommitOrdinal::new(0));

    let run_admitted = match admission_commit[0].payload() {
        events::KernelEventPayload::RunAdmitted(payload) => payload,
        _ => panic!("first admission event must be RunAdmitted"),
    };
    assert_eq!(
        run_admitted.spec_artifact.role,
        events::ArtifactRole::TypedExecutionSpec
    );
    assert_eq!(
        run_admitted.certificate_artifact.role,
        events::ArtifactRole::TypedSpecCertificate
    );
    assert!(run_admitted
        .config_artifacts
        .iter()
        .all(|artifact| artifact.role == events::ArtifactRole::TypedConfig));
    assert_eq!(
        fixture.seed_ref.seed_artifact.role,
        events::ArtifactRole::SeedInput
    );
    assert!(stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptStarted(_)
                | events::KernelEventPayload::StateAttemptCompleted(_)
                | events::KernelEventPayload::CellProduced(_)
                | events::KernelEventPayload::ArtifactReferenced(_)
                | events::KernelEventPayload::RetentionRefsAppended(_)
        )
    }));
}

#[tokio::test]
async fn scheduler_binds_staged_retention_refs_and_projects_manifest() {
    let fixture = fixture_with_retention_lifecycle_node();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let projection_snapshot = store.projection_snapshot();
    let start_retention = projection_snapshot
        .retention(&fixture.run_id)
        .expect("run-start retention");
    assert!(start_retention
        .refs
        .contains_key(&fixture.seed_ref.seed_artifact.artifact_id));

    let status = drive_until_blocked(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("drive to completion");
    assert_eq!(status, SchedulerStatus::PublicOutputProjected);
    let render_receipt_artifact = match store
        .projection_snapshot()
        .cell_terminal(&fixture.render_cell)
        .expect("render receipt cell")
    {
        store::CellTerminalProjection::Produced { artifact_id, .. } => artifact_id.clone(),
        terminal => panic!("unexpected render terminal: {terminal:?}"),
    };
    assert!(store
        .projection_snapshot()
        .retention(&fixture.run_id)
        .expect("runtime retention")
        .refs
        .contains_key(&render_receipt_artifact));

    let projection_snapshot = store.projection_snapshot();
    let projection = projection_snapshot
        .retention(&fixture.run_id)
        .expect("retention projection");
    let manifest = projection.manifest.as_ref().expect("manifest");
    assert_eq!(manifest.manifest_seq, 1);
    assert!(projection.refs.contains_key(&manifest.manifest_artifact_id));

    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    let stream = store.load_run_stream(&fixture.run_id);
    let manifest_event = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionManifestProjected(payload)
                if payload.manifest_artifact_id == manifest.manifest_artifact_id =>
            {
                Some((event.seq(), payload))
            }
            _ => None,
        })
        .expect("manifest event");
    let retention_seq = manifest_event.0;
    assert_eq!(manifest_event.1.manifest_digest, manifest.manifest_digest);
    let receipt = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if event.seq() == retention_seq && payload.node_id == retention_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("retention receipt cell");
    assert_eq!(receipt.cell_id, retention_node.output_cell);
    assert!(stream.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptStarted(payload)
                if event.seq() < retention_seq
                    && payload.node_id == retention_node.node_id
                    && payload.attempt_id == receipt.attempt_id
        )
    }));
    assert!(stream.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if event.seq() == retention_seq
                    && payload.node_id == retention_node.node_id
                    && payload.attempt_id == receipt.attempt_id
                    && payload.output_cell_id == retention_node.output_cell
        )
    }));
    let manifest_ref = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload)
                if event.seq() == retention_seq
                    && payload.reason == events::RetentionReason::ManifestProjection =>
            {
                payload.refs.first()
            }
            _ => None,
        })
        .expect("manifest retention ref");
    assert_eq!(manifest_ref.artifact_id, manifest.manifest_artifact_id);
    assert_eq!(manifest_ref.content_digest, manifest.manifest_digest);
    assert_eq!(manifest_ref.role, events::ArtifactRole::RetentionManifest);
}

#[tokio::test]
async fn retention_manifest_projection_retry_is_idempotent_after_current_store_advanced() {
    let fixture = fixture_with_retention_lifecycle_node();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;
    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    append_attempt_start(&mut store, &fixture, retention_node, 1);
    let stale_stream = store.load_run_stream(&fixture.run_id);

    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("advance current store with retention projection");
    assert!(store
        .projection_snapshot()
        .retention(&fixture.run_id)
        .and_then(|retention| retention.manifest.as_ref())
        .is_some());

    let stale_store = StaleStreamStore::new(&mut store, stale_stream);
    assert_eq!(
        scheduler
            .drive_once(&stale_store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("idempotent retention retry"),
        SchedulerStatus::Advanced
    );
}

#[tokio::test]
async fn runtime_rejects_standalone_retention_manifest_projection_history() {
    let fixture = fixture_with_retention_lifecycle_node();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;

    let manifest = build_retention_manifest_artifact(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store.load_run_stream(&fixture.run_id),
    )
    .expect("manifest");
    let manifest_evidence = manifest.evidence.clone();
    let request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new("synthetic/standalone-retention-projection")
            .expect("commit key"),
        payloads: retention_manifest_payloads(&fixture.runtime_spec, &fixture.run_id, manifest),
        required_artifacts: vec![manifest_evidence],
        preconditions: store::CommitPreconditions::default(),
    };
    store
        .append_prepared_commit(request)
        .expect("synthetic standalone projection");

    assert!(matches!(
        RuntimeRunView::from_stream(
            &fixture.runtime_spec,
            &fixture.run_id,
            &store.load_run_stream(&fixture.run_id)
        ),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("retention manifest projection was not produced")
    ));
}

#[tokio::test]
async fn runtime_rejects_complete_run_receipt_commit_without_run_completed() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_until_blocked(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("drive to completion");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(payload, events::KernelEventPayload::RunCompleted(_))
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("missing RunCompleted")
    ));
}

#[tokio::test]
async fn public_output_render_failure_resumes_and_completes() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("drive a");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("drive b");
    let render_node = node_by_output(&fixture, &fixture.render_cell);
    let failed_attempt = append_attempt_start(&mut store, &fixture, render_node, 1);
    append_public_output_render_failure(&mut store, &fixture, render_node, &failed_attempt);
    assert!(matches!(
        store
            .projection_snapshot()
            .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id),
        Some(store::PublicOutputProjection::RenderFailed { .. })
    ));

    assert_eq!(
        drive_until_blocked(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("retry render"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &fixture.render_node),
        2
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id),
        Some(store::PublicOutputProjection::Produced { .. })
    ));
}

#[test]
fn certified_runtime_spec_rejects_hash_mismatch() {
    let fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    envelope.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
    assert!(matches!(
        CertifiedRuntimeSpec::from_verified_envelope(envelope),
        Err(RuntimeError::SpecHash(_))
    ));
}

#[test]
fn certified_runtime_spec_accepts_certifier_authority() {
    let (certified, _registry) = certifier_backed_runtime_authority();
    let runtime = CertifiedRuntimeSpec::new(certified).expect("runtime authority");
    assert!(!runtime.topological_order().is_empty());
}

#[test]
fn certified_runtime_spec_accepts_verified_persisted_parts_authority() {
    let (certified, registry) = certifier_backed_runtime_authority();
    let persisted_parts = certified
        .to_persisted_parts()
        .expect("persisted spec/certificate parts");
    let verified = mfm_certify::verify_persisted_spec_certificate(
        persisted_parts.spec_bytes(),
        persisted_parts.certificate_bytes(),
        &registry,
    )
    .expect("verified persisted spec/certificate");
    let runtime = CertifiedRuntimeSpec::new(verified).expect("runtime authority");
    assert!(!runtime.topological_order().is_empty());
}

#[tokio::test]
async fn replay_rejects_run_completed_without_public_output_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-complete-without-public-output")
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunCompleted(
                events::RunCompleted {
                    run_id: fixture.run_id.clone(),
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    outcome: events::RunCompletionOutcome::Completed(Box::new(
                        events::PublicOutputCompletionEvidence {
                            public_output_schema_id: fixture
                                .runtime_spec
                                .spec()
                                .public_outputs
                                .public_schema_id
                                .clone(),
                            public_output_event_id: EventId::from_digest(
                                DigestAlgorithm::Sha256JcsV1,
                                D9,
                            ),
                        },
                    )),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged completion");

    assert!(matches!(
        drive_once(&scheduler, &mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("RunCompleted appeared before PublicOutputProduced")
    ));
}

#[tokio::test]
async fn scheduler_rejects_uncertified_capability_use() {
    struct BadFactRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
    }

    impl ErasedNodeRunner for BadFactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                Ok(ErasedRunnerOutput::new(vec![
                    RunnerEventPayload::FactRecorded(events::FactRecorded {
                        spec_hash: ctx.spec_hash().clone(),
                        node_id: ctx.node().node_id.clone(),
                        attempt_id: ctx.attempt_id().clone(),
                        capability_kind: self.cap_kind.clone(),
                        capability_version: self.cap_version.clone(),
                        adapter_kind: AdapterKind::new(
                            "mfm.test",
                            "adapter",
                            DigestAlgorithm::Sha256JcsV1,
                            D1,
                        )
                        .expect("adapter"),
                        adapter_version: AdapterVersion::new("mfm.adapter.v1")
                            .expect("adapter version"),
                        request_schema_id: ctx.node().config_ref.schema_id.clone(),
                        request_hash: content(0xc1),
                        response_schema_id: ctx.node().config_ref.schema_id.clone(),
                        response_hash: content(0xc2),
                        fact_key: events::FactKey::new("bad-fact").expect("fact key"),
                        artifact_id: artifact(0xc3),
                    }),
                ]))
            })
        }
    }

    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            BadFactRunner {
                cap_kind: fixture.cap_kind.clone(),
                cap_version: fixture.cap_version.clone(),
            },
        ))
        .expect("binding");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize uncertified capability use"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
}

#[tokio::test]
async fn runner_cannot_stage_artifact_with_foreign_producer() {
    struct ForeignProducerArtifactRunner {
        foreign_node_id: NodeId,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for ForeignProducerArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let mut artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                );
                artifact.producer_node_id = Some(self.foreign_node_id.clone());
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let foreign_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            ForeignProducerArtifactRunner {
                foreign_node_id,
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize foreign producer artifact"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
}

#[tokio::test]
async fn runner_cannot_stage_inline_artifact_with_mismatched_bytes() {
    struct BadInlineArtifactRunner {
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for BadInlineArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    b"mismatched".to_vec(),
                    artifact,
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            BadInlineArtifactRunner {
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize mismatched inline artifact"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
}

#[tokio::test]
async fn runner_can_commit_inline_state_output_artifact() {
    struct InlineArtifactRunner {
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for InlineArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        artifact.artifact_id.clone(),
                        artifact.digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let output_bytes = br#"{"inline":true}"#.to_vec();
    let output_digest = digest_for_bytes(&output_bytes);
    let output_artifact =
        ArtifactId::from_digest(output_digest.algorithm(), *output_digest.digest());
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            InlineArtifactRunner { output_bytes },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("drive inline output"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        store.projection_snapshot().cell_terminal(&fixture.cell_a),
        Some(store::CellTerminalProjection::Produced {
            artifact_id,
            content_digest,
            ..
        }) if artifact_id == &output_artifact && content_digest == &output_digest
    ));
}

#[tokio::test]
async fn runner_cannot_stage_reserved_retention_reasons() {
    struct ReservedRetentionReasonRunner {
        output_bytes: Vec<u8>,
        reason: events::RetentionReason,
    }

    impl ErasedNodeRunner for ReservedRetentionReasonRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: vec![StagedRetentionRefs {
                        refs: vec![retention_ref_for_artifact(&artifact)],
                        reason: self.reason,
                    }],
                    payloads: terminal_payloads(
                        &ctx,
                        artifact.artifact_id.clone(),
                        artifact.digest.clone(),
                    ),
                })
            })
        }
    }

    for reason in [
        events::RetentionReason::RunAdmitted,
        events::RetentionReason::ManifestProjection,
        events::RetentionReason::PublicOutput,
    ] {
        let fixture = fixture();
        let node = node_by_output(&fixture, &fixture.cell_a).clone();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                ReservedRetentionReasonRunner {
                    output_bytes: br#"{"reserved":true}"#.to_vec(),
                    reason,
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
        let mut store = TestTypedRunStore::new();
        start_fixture_run(
            &scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await
        .expect("start run");
        let stream_before = store.load_run_stream(&fixture.run_id);

        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("terminalize reserved retention reason"),
            SchedulerStatus::Advanced
        );
        assert_eq!(
            store.load_run_stream(&fixture.run_id).len(),
            stream_before.len() + 4
        );
        assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());
    }
}

#[tokio::test]
async fn runner_output_requires_payload_bound_staged_artifact() {
    struct MissingStagedArtifactRunner {
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for MissingStagedArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                Ok(ErasedRunnerOutput {
                    staged_artifacts: Vec::new(),
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            MissingStagedArtifactRunner {
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize missing staged artifact"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
}

#[tokio::test]
async fn rejected_staged_payload_mismatch_does_not_admit_artifact_evidence() {
    struct MismatchedStagedArtifactRunner {
        staged_artifact: ArtifactId,
        staged_digest: ContentDigest,
        payload_artifact: ArtifactId,
        payload_digest: ContentDigest,
    }

    impl ErasedNodeRunner for MismatchedStagedArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.staged_artifact.clone(),
                    self.staged_digest.clone(),
                );
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.payload_artifact.clone(),
                        self.payload_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    let staged_artifact = artifact(0xa1);
    let staged_digest = content(0xa2);
    let payload_artifact = artifact(0xa3);
    let payload_digest = content(0xa4);
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            MismatchedStagedArtifactRunner {
                staged_artifact: staged_artifact.clone(),
                staged_digest: staged_digest.clone(),
                payload_artifact,
                payload_digest,
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize staged payload mismatch"),
        SchedulerStatus::Advanced
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");

    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(&node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let attempt_logical_key =
        store::LogicalEventKey::new(format!("attempt:{}:{}", node.node_id, attempt_id))
            .expect("attempt key");
    let leaked_artifact_request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new("missing-leaked-staged-artifact").expect("commit key"),
        payloads: vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                cell_id: node.output_cell.clone(),
                scope_id: node.scope_id.clone(),
                attempt_id: attempt_id.clone(),
                semantic_type_id: descriptor.output_semantic_type_id.clone(),
                schema_id: descriptor.output_schema_id.clone(),
                value_lineage: output_cell.value_lineage.clone(),
                artifact_id: staged_artifact.clone(),
                content_digest: staged_digest,
                producer_state_kind: Some(node.state_kind.clone()),
                producer_state_version: Some(node.state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: node.output_cell.clone(),
            }),
        ],
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![attempt_logical_key],
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    };
    let artifacts = store::CommitArtifactEvidenceSet::new(
        leaked_artifact_request.required_artifacts().to_vec(),
        Vec::new(),
    )
    .expect("leak probe artifact evidence set");
    let error =
        store::PreparedCommit::<store::AttemptTerminal>::new(leaked_artifact_request, artifacts)
            .expect_err("missing leak evidence rejects before append");
    assert!(matches!(
        error,
        store::StoreError::InvalidPreparedCommitPurpose { message, .. }
            if message.contains("missing required artifact evidence")
                && message.contains(staged_artifact.as_str())
    ));
}

#[test]
fn materialization_rejects_seed_digest_not_certified() {
    let fixture = fixture();
    let mut seed = fixture.seed_ref.clone();
    seed.digest = content(0xee);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    assert!(matches!(
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![seed],),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_missing_config_artifact_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence.config_artifacts.clear();
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_mismatched_staged_launch_bytes() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let base = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    let mut bad_spec = base.clone();
    bad_spec.spec_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_spec,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_certificate = base.clone();
    bad_certificate.certificate_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_certificate,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_config = base.clone();
    bad_config
        .config_artifacts
        .first_mut()
        .expect("config artifact")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_config,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_seed = base;
    bad_seed
        .seed_cells
        .first_mut()
        .expect("seed cell")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_seed,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));
    assert!(store.load_run_stream(&fixture.run_id).is_empty());
}

#[tokio::test]
async fn run_admission_returns_bound_context_with_capability_and_framework_authority() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let launch =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .expect("prepare launch");

    let authority =
        scheduler_start_run_admitted(&scheduler, &mut store, &fixture.runtime_spec, launch)
            .await
            .expect("admitted run authority");

    assert_eq!(authority.run_id(), &fixture.run_id);
    assert_eq!(authority.spec_hash(), fixture.runtime_spec.spec_hash());
    assert_eq!(
        authority.head_seq(),
        store.expected_next_seq(&fixture.run_id)
    );

    let node = node_by_output(&fixture, &fixture.cell_b);
    let capability = authority
        .bound_context()
        .capability_authority_for(&node.node_id)
        .expect("capability authority");
    assert_eq!(capability.capabilities(), &node.capability_bindings);
    assert_eq!(capability.implementations().len(), 1);
    assert_eq!(
        capability.implementations()[0].descriptor(),
        &node.capability_bindings.capabilities[0]
    );
    assert_eq!(
        capability.implementations()[0].implementation_id().as_str(),
        "mfm.test.capability"
    );

    let render_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("public-output render node");
    let framework = authority
        .bound_context()
        .framework_handler_for(&render_node.node_id)
        .expect("framework handler authority");
    assert_eq!(
        framework.kind(),
        BoundFrameworkHandlerKind::PublicOutputRender
    );

    let run_admitted = store
        .load_run_stream(&fixture.run_id)
        .into_iter()
        .find_map(|event| match event.payload().clone() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(payload),
            _ => None,
        })
        .expect("RunAdmitted payload");
    assert_eq!(
        run_admitted.runner_executables,
        authority.bound_context().runner_executables()
    );
}

#[test]
fn run_start_rejects_missing_capability_implementation() {
    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(registry);
    let store = TestTypedRunStore::new();
    let error =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .err()
            .expect("missing capability implementation must reject launch");
    assert!(
        matches!(error, RuntimeError::RunnerBinding(message) if message.contains("missing capability implementation"))
    );
}

#[test]
fn run_start_rejects_capability_implementation_descriptor_mismatch() {
    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    let implementation_id =
        CapabilityImplementationId::new("mfm.test.capability").expect("capability implementation");
    registry
        .register_capability(CapabilityImplementationBinding::new(
            CapabilityDescriptor::new(
                fixture.cap_kind.clone(),
                fixture.cap_version.clone(),
                CapabilityRole::ReadExternal,
                "wrong-read-db",
            )
            .expect("wrong capability descriptor"),
            implementation_id,
        ))
        .expect("capability implementation");
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(registry);
    let store = TestTypedRunStore::new();
    let error =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .err()
            .expect("mismatched capability implementation must reject launch");
    assert!(
        matches!(error, RuntimeError::RunnerBinding(message) if message.contains("differs from certified descriptor"))
    );
}

#[tokio::test]
async fn resume_rejects_missing_downstream_binding_before_attempt_start() {
    let fixture = fixture();
    let launch_scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &launch_scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let stream_len_before = store.load_run_stream(&fixture.run_id).len();

    let mut partial_registry = ErasedRunnerRegistry::new();
    partial_registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    let resume_scheduler = test_scheduler(partial_registry);
    let error = drive_once(
        &resume_scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect_err("missing descriptor b binding should reject bound context");

    assert!(
        matches!(error, RuntimeError::RunnerBinding(message) if message.contains("missing runner binding"))
    );
    assert_eq!(
        store.load_run_stream(&fixture.run_id).len(),
        stream_len_before
    );
}

#[tokio::test]
async fn resume_rejects_runner_executable_identity_mismatch_before_attempt_start() {
    let fixture = fixture();
    let launch_scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &launch_scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let stream_len_before = store.load_run_stream(&fixture.run_id).len();

    let mut changed_registry = ErasedRunnerRegistry::new();
    let mut changed_a = binding(
        fixture.descriptor_a.clone(),
        "pure",
        RecordingRunner {
            expected_caps: Vec::new(),
            output_artifact: artifact(0xa1),
            output_digest: content(0xa2),
        },
    );
    changed_a.executable.binary_digest = content(0xee);
    changed_registry.register(changed_a).expect("binding a");
    changed_registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let resume_scheduler =
        test_scheduler(register_fixture_capabilities(changed_registry, &fixture));
    let error = drive_once(
        &resume_scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect_err("changed executable identity should reject bound context");

    assert!(matches!(error, RuntimeError::RunnerBinding(message)
            if message.contains("runner executable identities")));
    assert_eq!(
        store.load_run_stream(&fixture.run_id).len(),
        stream_len_before
    );
}

#[tokio::test]
async fn runner_invocation_uses_run_admitted_config_evidence_without_reference_event() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    assert!(valid_stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
        )
    }));
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("drive with RunAdmitted config evidence");
}

#[tokio::test]
async fn runner_invocation_requires_committed_produced_input_artifact_reference() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                    && payload.node_id.as_ref() == Some(&producer_node_id)
        )
    });
    {
        let corrupt_store = StaleStreamStore::new(&mut store, corrupt_stream);
        assert!(matches!(
            scheduler
                .drive_once(&corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InputMaterialization(message))
                if message.contains("is not committed in the run stream")
        ));
    }
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
        1
    );
}

#[tokio::test]
async fn post_start_materialization_failure_terminalizes_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node = node_by_output(&fixture, &fixture.cell_b).clone();
    {
        let corrupt_store = MissingInputArtifactRefStore::new(&mut store, producer_node_id);
        assert_eq!(
            scheduler
                .drive_once(&corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("terminalize materialization failure"),
            SchedulerStatus::Advanced
        );
    }

    assert_node_failed_with_code_and_retryable(
        &store,
        &consumer_node.node_id,
        "input_materialization_failed",
        true,
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&consumer_node.output_cell)
        .is_none());
}

#[tokio::test]
async fn post_start_runtime_validation_failure_terminalizes_attempt() {
    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            ErrorRunner {
                error: RuntimeError::RuntimeValidation(
                    "synthetic post-start validation failure".to_owned(),
                ),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize runtime validation failure"),
        SchedulerStatus::Advanced
    );

    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runtime_validation_failed");
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=0 completed=0 failed=1 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn post_start_invalid_run_stream_failure_does_not_terminalize_attempt() {
    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            ErrorRunner {
                error: RuntimeError::InvalidRunStream(
                    "synthetic corrupt stream authority".to_owned(),
                ),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert!(matches!(
        drive_once(&scheduler, &mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("synthetic corrupt stream authority")
    ));

    let node = node_by_output(&fixture, &fixture.cell_a);
    let projection_snapshot = store.projection_snapshot();
    let attempts = projection_snapshot
        .attempts()
        .filter(|((node_id, _), _)| node_id == &node.node_id)
        .map(|(_, attempt)| attempt)
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 1);
    assert!(matches!(
        attempts[0].status,
        store::AttemptStatus::Started { .. }
    ));
    assert_failure_code_count(&store, "runtime_validation_failed", 0);
    assert_failure_code_count(&store, "runner_output_invalid", 0);
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn replay_rejects_terminal_cell_producer_outside_certified_spec() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let forged_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell != fixture.cell_a)
        .expect("second node")
        .clone();
    let certified_cell = fixture
        .runtime_spec
        .cell(&fixture.cell_a)
        .expect("cell a")
        .clone();
    let forged_attempt = AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xfa; 32]),
    );
    let artifact_id = artifact(0xfa);
    let artifact_digest = content(0xfb);
    let forged_artifact = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: artifact_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(certified_cell.schema_id.clone()),
        semantic_type_id: Some(certified_cell.semantic_type_id.clone()),
        producer_node_id: Some(forged_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-attempt-start").expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    attempt_no: 1,
                    state_kind: forged_node.state_kind.clone(),
                    state_version: forged_node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged attempt start");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-terminal").expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    cell_id: fixture.cell_a.clone(),
                    scope_id: certified_cell.scope_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    semantic_type_id: certified_cell.semantic_type_id.clone(),
                    schema_id: certified_cell.schema_id.clone(),
                    value_lineage: certified_cell.value_lineage.clone(),
                    artifact_id,
                    content_digest: artifact_digest,
                    producer_state_kind: Some(forged_node.state_kind.clone()),
                    producer_state_version: Some(forged_node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt,
                    output_cell_id: fixture.cell_a.clone(),
                }),
            ],
            required_artifacts: vec![forged_artifact],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged terminal");

    assert!(matches!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn store_rejects_fact_without_started_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell == fixture.cell_b)
        .expect("read node")
        .clone();
    let fact_artifact = artifact(0xd1);
    let fact_digest = content(0xd2);
    let fact_schema = node.config_ref.schema_id.clone();
    let fact_evidence = store::ArtifactEvidenceRef {
        artifact_id: fact_artifact.clone(),
        digest: fact_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(fact_schema.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    assert!(store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-fact").expect("commit key"),
            payloads: vec![events::KernelEventPayload::FactRecorded(
                events::FactRecorded {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: AttemptId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        DigestBytes::from_array([0xd3; 32]),
                    ),
                    capability_kind: fixture.cap_kind.clone(),
                    capability_version: fixture.cap_version.clone(),
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                    request_schema_id: fact_schema.clone(),
                    request_hash: content(0xd4),
                    response_schema_id: fact_schema,
                    response_hash: fact_digest,
                    fact_key: events::FactKey::new("forged-fact").expect("fact key"),
                    artifact_id: fact_artifact,
                },
            )],
            required_artifacts: vec![fact_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .is_err());
}

#[tokio::test]
async fn replay_rejects_public_output_without_render_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let non_render_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell == fixture.cell_a)
        .expect("non-render node")
        .clone();
    let public_cell = fixture
        .runtime_spec
        .spec()
        .public_outputs
        .outputs
        .first()
        .expect("public cell")
        .clone();
    let source_artifact = artifact(0xe1);
    let source_digest = content(0xe2);
    let producer_node_id = match &public_cell.producer {
        spec::CellProducer::Node(node_id) => Some(node_id.clone()),
        spec::CellProducer::Seed(_) => None,
    };
    let source_evidence = store::ArtifactEvidenceRef {
        artifact_id: source_artifact.clone(),
        digest: source_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(public_cell.schema_id.clone()),
        semantic_type_id: Some(public_cell.semantic_type_id.clone()),
        producer_node_id,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let forged_attempt = append_attempt_start(&mut store, &fixture, &non_render_node, 1);
    let output_cell = fixture
        .runtime_spec
        .cell(&non_render_node.output_cell)
        .expect("non-render output")
        .clone();
    let receipt_artifact = artifact(0xe3);
    let receipt_digest = content(0xe4);
    let receipt_evidence = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact.clone(),
        digest: receipt_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(output_cell.schema_id.clone()),
        semantic_type_id: Some(output_cell.semantic_type_id.clone()),
        producer_node_id: Some(non_render_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-public-output").expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    cell_id: non_render_node.output_cell.clone(),
                    scope_id: output_cell.scope_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    semantic_type_id: output_cell.semantic_type_id.clone(),
                    schema_id: output_cell.schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    artifact_id: receipt_artifact,
                    content_digest: receipt_digest,
                    producer_state_kind: Some(non_render_node.state_kind.clone()),
                    producer_state_version: Some(non_render_node.state_version.clone()),
                }),
                events::KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    receipt_cell_id: non_render_node.output_cell.clone(),
                    public_schema_id: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .public_schema_id
                        .clone(),
                    output_spec_digest: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .digest()
                        .expect("public digest"),
                    cells: vec![events::NamedTypedCellRef {
                        public_field_path: public_cell.public_field_path.clone(),
                        cell_id: public_cell.cell_id.clone(),
                        producer: public_cell.producer.clone(),
                        scope_id: public_cell.scope_id.clone(),
                        semantic_type_id: public_cell.semantic_type_id.clone(),
                        schema_id: public_cell.schema_id.clone(),
                        value_lineage: public_cell.value_lineage.clone(),
                        content_digest: source_digest,
                        artifact_id: source_artifact,
                    }],
                    rendered_digest: content(0xe5),
                    rendered_artifact_id: None,
                    renderer_descriptor_id: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .renderer_descriptor
                        .descriptor_id
                        .clone(),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    output_cell_id: non_render_node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![source_evidence, receipt_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    non_render_node.node_id, forged_attempt
                ))
                .expect("attempt key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: non_render_node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged public output");
    assert!(matches!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn replay_rejects_public_output_with_forged_rendered_digest() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("drive a");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("drive b");

    let render_node = node_by_output(&fixture, &fixture.render_cell).clone();
    let attempt_id = append_attempt_start(&mut store, &fixture, &render_node, 1);
    let output_cell = fixture
        .runtime_spec
        .cell(&render_node.output_cell)
        .expect("render output")
        .clone();
    let bad_rendered_digest = content(0xf1);
    let bad_receipt_digest = content(0xf2);
    let bad_receipt_artifact =
        ArtifactId::from_digest(bad_receipt_digest.algorithm(), *bad_receipt_digest.digest());
    let bad_receipt_evidence = store::ArtifactEvidenceRef {
        artifact_id: bad_receipt_artifact.clone(),
        digest: bad_receipt_digest.clone(),
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(output_cell.schema_id.clone()),
        semantic_type_id: Some(output_cell.semantic_type_id.clone()),
        producer_node_id: Some(render_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let public_cells = fixture
        .runtime_spec
        .spec()
        .public_outputs
        .outputs
        .iter()
        .map(|public_cell| {
            let projection_snapshot = store.projection_snapshot();
            let Some(store::CellTerminalProjection::Produced {
                artifact_id,
                content_digest,
                ..
            }) = projection_snapshot.cell_terminal(&public_cell.cell_id)
            else {
                panic!("public cell should be produced");
            };
            events::NamedTypedCellRef {
                public_field_path: public_cell.public_field_path.clone(),
                cell_id: public_cell.cell_id.clone(),
                producer: public_cell.producer.clone(),
                scope_id: public_cell.scope_id.clone(),
                semantic_type_id: public_cell.semantic_type_id.clone(),
                schema_id: public_cell.schema_id.clone(),
                value_lineage: public_cell.value_lineage.clone(),
                content_digest: content_digest.clone(),
                artifact_id: artifact_id.clone(),
            }
        })
        .collect::<Vec<_>>();
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &render_node.framework else {
        panic!("expected render node");
    };
    let error = store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-public-output-rendered-digest")
                .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    cell_id: render_node.output_cell.clone(),
                    scope_id: output_cell.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: output_cell.semantic_type_id.clone(),
                    schema_id: output_cell.schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    artifact_id: bad_receipt_artifact,
                    content_digest: bad_receipt_digest,
                    producer_state_kind: Some(render_node.state_kind.clone()),
                    producer_state_version: Some(render_node.state_version.clone()),
                }),
                events::KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    receipt_cell_id: render_node.output_cell.clone(),
                    public_schema_id: render.public_schema_id.clone(),
                    output_spec_digest: render.output_spec_digest.clone(),
                    cells: public_cells,
                    rendered_digest: bad_rendered_digest,
                    rendered_artifact_id: None,
                    renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    output_cell_id: render_node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![bad_receipt_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    render_node.node_id, attempt_id
                ))
                .expect("attempt key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: render_node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect_err("forged public output rejects before replay");
    assert!(matches!(
        error,
        store::StoreError::InvalidPreparedCommitPurpose { message, .. }
            if message.contains("missing required artifact evidence")
    ));
}

#[tokio::test]
async fn recovery_interrupts_started_pure_attempt_before_retrying_fresh_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node = node_by_output(&fixture, &fixture.cell_a);
    let interrupted_attempt_id = append_attempt_start(&mut store, &fixture, node, 1);

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("interrupt pure"),
        SchedulerStatus::Advanced
    );
    let projection_snapshot = store.projection_snapshot();
    let interrupted_attempt = projection_snapshot
        .attempt(&node.node_id, &interrupted_attempt_id)
        .expect("interrupted attempt");
    assert!(matches!(
        interrupted_attempt.status,
        store::AttemptStatus::Interrupted
    ));
    assert!(
        store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none(),
        "interruption must not terminalize the output cell"
    );

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("retry pure"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        2
    );
    let retry_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        2,
    )
    .expect("retry attempt id");
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &retry_attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=0 completed=1 failed=0 interrupted=1 total=2] cells=1 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn recovery_delegates_started_side_effect_attempt_to_side_effect_lifecycle() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, _) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-recovery",
        "sidefx-recovery-open",
    );
    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=1 lanes[run=1 total=1] public_outputs=0 retentions=1"
    );

    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &fixture.runtime_spec,
        &view,
        &BTreeSet::new(),
    )
    .expect("recovery disposition")
    .expect("open side-effect attempt")
    {
        crate::recovery::OpenAttemptDisposition::DelegateSideEffect {
            node: recovered_node,
            attempt_id: recovered_attempt,
            attempt_no,
        } => {
            assert_eq!(recovered_node.node_id, node.node_id);
            assert_eq!(recovered_attempt, attempt_id);
            assert_eq!(attempt_no, 1);
        }
        crate::recovery::OpenAttemptDisposition::Continue { .. } => {
            panic!("side-effect attempt must delegate to side-effect lifecycle")
        }
        crate::recovery::OpenAttemptDisposition::Interrupt { .. }
        | crate::recovery::OpenAttemptDisposition::RetryTerminalization { .. }
        | crate::recovery::OpenAttemptDisposition::OperationalBlock { .. } => {
            panic!("side-effect attempt must delegate to side-effect lifecycle")
        }
    }
}

#[test]
fn side_effect_attempt_view_from_erased_context_is_empty_before_ledger() {
    let fixture = fixture_with_first_side_effect_state();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let config_artifact = config_artifact(&fixture.runtime_spec, &node.config_ref).evidence;
    let projections = store::ProjectionSnapshot::default();
    let run_stream = Vec::new();
    let invocation = PreparedRunnerInvocation {
        runtime_spec: &fixture.runtime_spec,
        run_id: &fixture.run_id,
        spec_hash: fixture.runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id: &attempt_id,
        attempt_no: 1,
        config_artifact,
        inputs: MaterializedInputs {
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            root: MaterializedInputNode::Unit,
        },
        caps: CertifiedRuntimeCapabilities::new(
            node.node_id.clone(),
            node.capability_bindings.clone(),
        ),
        recorded_facts: RecordedFacts::default(),
        projections: &projections,
        run_stream: &run_stream,
    };
    let ctx = ErasedRunCtx::from_prepared(&invocation);
    let view = SideEffectAttemptView::from_erased_context(&ctx).expect("side-effect view");

    assert!(view.is_empty());
    assert!(view.projection().is_none());
    assert!(view.ledger_state().is_none());
    assert!(view.phase().is_none());
    assert!(view.ledger_key().is_none());
    assert!(view.ledger_purpose().is_none());
}

#[tokio::test]
async fn side_effect_attempt_view_from_verified_context_exposes_ledger_state() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-view",
        "sidefx-view-open",
    );
    let loader = crate::history::VerifiedRunContextLoader::new(
        crate::binding::BoundRuntimeContextLoader::new(registered_side_effect_fixture_runners(
            &fixture,
        )),
    );
    let context = loader
        .load_async(&fixture.runtime_spec, &fixture.run_id, &store)
        .await
        .expect("verified context");

    let view = SideEffectAttemptView::from_verified_context(&context, node, &attempt_id)
        .expect("side-effect view");
    assert!(!view.is_empty());
    assert_eq!(view.ledger_key(), Some(&ledger_key));
    assert_eq!(
        view.ledger_purpose(),
        Some(&events::SideEffectLedgerPurpose::Forward)
    );
    assert_eq!(
        view.projection().expect("projection").intent.attempt_id,
        attempt_id
    );
    match view.phase().expect("ledger phase") {
        store::SideEffectLedgerPhase::Prepared {
            claim,
            resource_key,
            ..
        } => {
            assert_eq!(claim.attempt_id, attempt_id);
            assert_eq!(claim.invocation_epoch, 1);
            assert!(resource_key.is_some());
        }
        other => panic!("unexpected side-effect phase: {other:?}"),
    }
}

#[tokio::test]
async fn recovery_sweep_includes_open_remediation_attempts() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let scheduler = compensated_saga_scheduler(&fixture);
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[fixture.cell_a.clone(), fixture.cell_b.clone()],
        "drive forward side-effect phase",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_some());

    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let remediation = fixture
        .runtime_spec
        .remediation_for_forward_node(&forward_b.node_id)
        .expect("remediation for forward b");
    let remediation_attempt = append_attempt_start(&mut store, &fixture, remediation, 1);

    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &fixture.runtime_spec,
        &view,
        &BTreeSet::new(),
    )
    .expect("recovery disposition")
    .expect("open remediation attempt")
    {
        crate::recovery::OpenAttemptDisposition::Continue {
            node,
            attempt_id,
            attempt_no,
        } => {
            assert_eq!(node.node_id, remediation.node_id);
            assert_eq!(attempt_id, remediation_attempt);
            assert_eq!(attempt_no, 1);
        }
        crate::recovery::OpenAttemptDisposition::DelegateSideEffect { .. }
        | crate::recovery::OpenAttemptDisposition::Interrupt { .. }
        | crate::recovery::OpenAttemptDisposition::RetryTerminalization { .. }
        | crate::recovery::OpenAttemptDisposition::OperationalBlock { .. } => {
            panic!("remediation attempt before ledger should continue through attempt lifecycle")
        }
    }
}

#[tokio::test]
async fn recovery_interrupts_side_effect_attempt_after_intent_before_prepare() {
    assert_prepared_boundary_side_effect_recovery_interrupts(false).await;
}

#[tokio::test]
async fn recovery_interrupts_side_effect_attempt_after_claim_before_prepare() {
    assert_prepared_boundary_side_effect_recovery_interrupts(true).await;
}

async fn assert_prepared_boundary_side_effect_recovery_interrupts(emit_claim: bool) {
    let fixture = fixture_with_first_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        PrePreparedSideEffectRunner { emit_claim },
    ));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node = node_by_output(&fixture, &fixture.cell_a).clone();

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("append pre-prepared side-effect evidence"),
        SchedulerStatus::Advanced
    );
    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &fixture.runtime_spec,
        &view,
        &BTreeSet::new(),
    )
    .expect("recovery disposition")
    .expect("open side-effect attempt")
    {
        crate::recovery::OpenAttemptDisposition::Interrupt {
            node: recovered_node,
            attempt_id: recovered_attempt,
            attempt_no,
        } => {
            assert_eq!(recovered_node.node_id, node.node_id);
            assert_eq!(recovered_attempt, attempt_id);
            assert_eq!(attempt_no, 1);
        }
        _ => panic!("pre-prepared side-effect attempt must interrupt"),
    }

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("interrupt pre-prepared side-effect attempt"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .attempt(&node.node_id, &attempt_id)
            .expect("attempt projection")
            .status,
        store::AttemptStatus::Interrupted
    ));
}

#[tokio::test]
async fn recovery_rejects_split_terminal_cell_and_attempt_completion() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("produce first cell");
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.output_cell_id == fixture.cell_a
        )
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::Store(message))
            if message.contains("terminal cell requires matching attempt completion")
    ));
}

#[tokio::test]
async fn recovery_rejects_attempt_started_before_inputs_were_terminal() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let node_a = node_by_output(&fixture, &fixture.cell_a);
    let node_b = node_by_output(&fixture, &fixture.cell_b);
    append_attempt_start(&mut store, &fixture, node_b, 1);
    let attempt_a = append_attempt_start(&mut store, &fixture, node_a, 1);
    append_terminal(
        &mut store,
        &fixture,
        node_a,
        &attempt_a,
        artifact(0xa1),
        content(0xa2),
    );

    assert!(matches!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn recovery_reuses_committed_read_facts_for_same_attempt() {
    struct FactReuseRunner {
        fact_key: events::FactKey,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for FactReuseRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let fact = ctx
                    .recorded_facts()
                    .get(&self.fact_key)
                    .expect("recorded fact");
                assert_eq!(fact.fact_key, self.fact_key);
                assert_eq!(fact.request_schema_id, ctx.node().config_ref.schema_id);
                assert_eq!(ctx.recorded_facts().iter().count(), 1);
                let artifact = store::ArtifactEvidenceRef {
                    artifact_id: self.output_artifact.clone(),
                    digest: self.output_digest.clone(),
                    byte_len: 17,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                    semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                };
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let fact_key = events::FactKey::new("reused-fact").expect("fact key");
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            FactReuseRunner {
                fact_key: fact_key.clone(),
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(
        &mut store,
        &fixture,
        node,
        &attempt_id,
        fact_key.clone(),
        artifact(0xd1),
        content(0xd2),
    );

    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("resume read");
    assert_eq!(fact_recorded_count(&store), 1);
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}

#[tokio::test]
async fn recovery_rejects_new_fact_after_same_attempt_fact_exists() {
    struct NewFactRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    }

    impl ErasedNodeRunner for NewFactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                Ok(ErasedRunnerOutput::new(vec![
                    RunnerEventPayload::FactRecorded(events::FactRecorded {
                        spec_hash: ctx.spec_hash().clone(),
                        node_id: ctx.node().node_id.clone(),
                        attempt_id: ctx.attempt_id().clone(),
                        capability_kind: self.cap_kind.clone(),
                        capability_version: self.cap_version.clone(),
                        adapter_kind: self.adapter_kind.clone(),
                        adapter_version: self.adapter_version.clone(),
                        request_schema_id: ctx.node().config_ref.schema_id.clone(),
                        request_hash: content(0xe1),
                        response_schema_id: ctx.node().config_ref.schema_id.clone(),
                        response_hash: content(0xe2),
                        fact_key: events::FactKey::new("new-fact").expect("fact key"),
                        artifact_id: artifact(0xe3),
                    }),
                ]))
            })
        }
    }

    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            NewFactRunner {
                cap_kind: fixture.cap_kind.clone(),
                cap_version: fixture.cap_version.clone(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(
        &mut store,
        &fixture,
        node,
        &attempt_id,
        events::FactKey::new("existing-fact").expect("fact key"),
        artifact(0xd1),
        content(0xd2),
    );

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize duplicate fact output"),
        SchedulerStatus::Advanced
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
    assert_eq!(store.projection_snapshot().facts().count(), 1);
}

#[tokio::test]
async fn recovery_allows_managed_write_artifact_restage_before_terminal_commit() {
    let fixture = fixture_with_first_managed_write_state();
    let output_artifact = artifact(0xa1);
    let output_digest = content(0xa2);
    let node = node_by_output(&fixture, &fixture.cell_a);
    let expected_caps = node
        .capability_bindings
        .capabilities
        .iter()
        .map(|capability| (capability.kind.clone(), capability.version.clone()))
        .collect();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "managed-write",
            RecordingRunner {
                expected_caps,
                output_artifact: output_artifact.clone(),
                output_digest: output_digest.clone(),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());

    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("resume managed write");
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}

#[tokio::test]
async fn scheduler_reloads_and_redecides_after_stale_expected_sequence_on_terminal_append() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = StaleOnceTypedRunStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive after injected stale terminal append"),
        SchedulerStatus::Advanced
    );
    assert!(
        store
            .projection_snapshot(&fixture.run_id)
            .await
            .cell_terminal(&fixture.cell_a)
            .is_some(),
        "scheduler must reload and observe the concurrently advanced terminal projection"
    );
}

#[tokio::test]
async fn side_effect_scheduler_commits_durable_ledger_phases_before_output() {
    let fixture = fixture_with_first_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let mut confirmed_before_output = false;
    for _ in 0..8 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("drive side effect phase"),
            SchedulerStatus::Advanced
        );
        let projection_snapshot = store.projection_snapshot();
        let projection =
            side_effect_projection_for_attempt(&projection_snapshot, node, &attempt_id)
                .expect("projection lookup")
                .expect("side-effect projection");
        if matches!(
            projection.phase,
            store::SideEffectPhase::ConfirmationObserved { .. }
        ) && projection_snapshot.cell_terminal(&fixture.cell_a).is_none()
        {
            confirmed_before_output = true;
            break;
        }
    }
    assert!(confirmed_before_output);
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("materialize side-effect output"),
        SchedulerStatus::Advanced
    );

    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(&projection_snapshot, node, &attempt_id)
        .expect("projection lookup")
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ));
    assert!(store
        .load_run_stream(&fixture.run_id)
        .iter()
        .any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectInvocationStarted(_)
        )));
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
}

#[tokio::test]
async fn runtime_rejects_exact_touched_set_receipt_without_evidence() {
    let fixture = fixture_with_first_exact_touched_set_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..2 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("advance before receipt"),
            SchedulerStatus::Advanced
        );
    }
    assert!(matches!(
        drive_once(&scheduler, &mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("without touched-set evidence")
    ));
}

#[tokio::test]
async fn runtime_rejects_exact_touched_set_confirmation_without_evidence() {
    let fixture = fixture_with_first_exact_touched_set_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        TouchedSetSideEffectRunner::with_receipt(&fixture),
    ));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..3 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("advance through receipt"),
            SchedulerStatus::Advanced
        );
    }
    assert!(matches!(
        drive_once(&scheduler, &mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("without touched-set evidence")
    ));
}

#[tokio::test]
async fn runtime_rejects_touched_set_confirmation_without_exact_claim() {
    let fixture = fixture_with_first_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        TouchedSetSideEffectRunner::with_confirmation(&fixture),
    ));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..3 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("advance through receipt"),
            SchedulerStatus::Advanced
        );
    }
    assert!(matches!(
        drive_once(&scheduler, &mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("without an exact-touched-set certified resource claim")
    ));
}

#[tokio::test]
async fn runtime_fails_pre_boundary_forward_attempt_before_saga_terminal() {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let failing_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        FailActiveSideEffectAfterSagaRunner::before_invocation_started(&fixture),
    ));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let forward_attempt = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &forward_node.node_id,
        1,
    )
    .expect("forward attempt id");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("prepare forward side effect"),
        SchedulerStatus::Advanced
    );
    let projection_snapshot = store.projection_snapshot();
    assert!(matches!(
        side_effect_projection_for_attempt(&projection_snapshot, &forward_node, &forward_attempt)
            .expect("side-effect lookup")
            .expect("side-effect projection")
            .phase,
        store::SideEffectPhase::InvocationPrepared { .. }
    ));

    let failing_attempt = append_attempt_start(&mut store, &fixture, &failing_node, 1);
    append_attempt_failure(&mut store, &fixture, &failing_node, &failing_attempt, false);
    assert_eq!(
        store
            .projection_snapshot()
            .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga)
            .run_mode,
        store::RunMode::FailedWithoutAcdcClaim
    );

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("fail active pre-boundary forward attempt"),
        SchedulerStatus::Advanced
    );
    let projection_snapshot = store.projection_snapshot();
    assert!(matches!(
        side_effect_projection_for_attempt(&projection_snapshot, &forward_node, &forward_attempt)
            .expect("side-effect lookup")
            .expect("side-effect projection")
            .phase,
        store::SideEffectPhase::Failed {
            failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
            ..
        }
    ));
    assert!(matches!(
        store
            .projection_snapshot()
            .attempt(&forward_node.node_id, &forward_attempt)
            .expect("forward attempt")
            .status,
        store::AttemptStatus::Failed { .. }
    ));
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("resolve saga terminal after active attempt closed"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_fails_not_submitted_forward_attempt_before_saga_terminal() {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let failing_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        FailActiveSideEffectAfterSagaRunner::new(&fixture)
            .with_submission_decision(TestSubmissionDecision::NotSubmitted),
    ));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let forward_attempt = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &forward_node.node_id,
        1,
    )
    .expect("forward attempt id");

    for _ in 0..2 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("advance forward side effect before not-submitted proof"),
            SchedulerStatus::Advanced
        );
    }
    let projection_snapshot = store.projection_snapshot();
    assert!(matches!(
        side_effect_projection_for_attempt(&projection_snapshot, &forward_node, &forward_attempt)
            .expect("side-effect lookup")
            .expect("side-effect projection")
            .phase,
        store::SideEffectPhase::NotSubmittedProven { .. }
    ));

    let failing_attempt = append_attempt_start(&mut store, &fixture, &failing_node, 1);
    append_attempt_failure(&mut store, &fixture, &failing_node, &failing_attempt, false);

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("fail active not-submitted forward attempt"),
        SchedulerStatus::Advanced
    );
    let projection_snapshot = store.projection_snapshot();
    assert!(matches!(
        side_effect_projection_for_attempt(&projection_snapshot, &forward_node, &forward_attempt)
            .expect("side-effect lookup")
            .expect("side-effect projection")
            .phase,
        store::SideEffectPhase::Failed {
            failure_phase: events::side_effect::FailurePhase::AfterNotSubmittedProven,
            ..
        }
    ));
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("resolve saga terminal after not-submitted attempt closed"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_remediates_confirmed_forward_ledgers_in_reverse_confirmation_order() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding forward a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding forward b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "fail",
            BlockingRunner,
        ))
        .expect("binding failure node");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[fixture.cell_a.clone(), fixture.cell_b.clone()],
        "drive forward side-effect phase",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_some());
    let forward_a_ledger =
        forward_ledger_for_node(&store.projection_snapshot(), &forward_a.node_id);
    let forward_b_ledger =
        forward_ledger_for_node(&store.projection_snapshot(), &forward_b.node_id);

    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::Remediating);
    assert_eq!(saga.obligations.len(), 2);

    for _ in 0..12 {
        let status = drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .expect("drive remediation phase");
        assert!(matches!(
            status,
            SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
        ));
        if store
            .projection_snapshot()
            .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga)
            .run_mode
            == store::RunMode::Compensated
        {
            break;
        }
    }
    let remediation_order = remediation_intent_forward_links(&store, &fixture.run_id);
    assert_eq!(
        remediation_order,
        vec![forward_b_ledger.clone(), forward_a_ledger.clone()]
    );
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::Compensated);
    for forward_ledger in [forward_a_ledger, forward_b_ledger] {
        let obligation = saga
            .obligations
            .get(&forward_ledger)
            .expect("forward obligation");
        assert_eq!(
            obligation.classification,
            store::ForwardLedgerClassification::Owed
        );
        assert!(
            obligation
                .remediation
                .as_ref()
                .expect("remediation ledger")
                .closed
        );
    }
    for _ in 0..8 {
        if store.projection_snapshot().run_state(&fixture.run_id) == store::RunState::Completed {
            break;
        }
        let status = drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .expect("resolve compensated terminal");
        assert!(matches!(
            status,
            SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
        ));
    }
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::Compensated
    ));
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("completed compensated run"),
        SchedulerStatus::PublicOutputProjected
    );
}

#[tokio::test]
async fn runtime_compensated_saga_resume_boundaries_do_not_duplicate_mutations() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let remediation_a = fixture
        .runtime_spec
        .spec()
        .remediations
        .get(&forward_a.node_id)
        .expect("remediation a")
        .clone();
    let remediation_b = fixture
        .runtime_spec
        .spec()
        .remediations
        .get(&forward_b.node_id)
        .expect("remediation b")
        .clone();
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut scheduler = compensated_saga_scheduler(&fixture);
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    drive_until_side_effect_confirmation_without_output(
        &scheduler,
        &mut store,
        &fixture,
        &forward_a,
        &fixture.cell_a,
        "forward a confirmation before output",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[fixture.cell_a.clone(), fixture.cell_b.clone()],
        "forward outputs",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);
    let forward_a_ledger =
        forward_ledger_for_node(&store.projection_snapshot(), &forward_a.node_id);
    let forward_b_ledger =
        forward_ledger_for_node(&store.projection_snapshot(), &forward_b.node_id);

    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::Remediating);
    assert_eq!(saga.obligations.len(), 2);

    scheduler = compensated_saga_scheduler(&fixture);
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);
    drive_until_remediation_phase(
        &scheduler,
        &mut store,
        &fixture,
        &forward_b_ledger,
        RemediationPhaseCheckpoint::SubmissionObserved,
        "first remedial submission",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_remediation_phase(
        &scheduler,
        &mut store,
        &fixture,
        &forward_b_ledger,
        RemediationPhaseCheckpoint::ConfirmationObserved,
        "first remedial confirmation",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&remediation_b.output_cell)
        .is_none());
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        std::slice::from_ref(&remediation_b.output_cell),
        "first remedial output",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    drive_until_compensated_before_terminal(&scheduler, &mut store, &fixture).await;
    assert!(matches!(
        remediation_projection_for_forward_ledger(store.projection_snapshot(), &forward_a_ledger)
            .expect("second remediation projection")
            .phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ));
    assert!(store
        .projection_snapshot()
        .cell_terminal(&remediation_a.output_cell)
        .is_none());
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        std::slice::from_ref(&remediation_a.output_cell),
        "second remedial output",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&remediation_a.output_cell)
        .is_some());
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);
    assert_eq!(
        store
            .projection_snapshot()
            .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga)
            .run_mode,
        store::RunMode::Compensated
    );
    assert_ne!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );

    scheduler = compensated_saga_scheduler(&fixture);
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("resolve compensated terminal after resume"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::Compensated
    ));
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("completed compensated run"),
        SchedulerStatus::PublicOutputProjected
    );

    assert_eq!(
        remediation_intent_forward_links(&store, &fixture.run_id),
        vec![forward_b_ledger.clone(), forward_a_ledger.clone()]
    );
    for forward_ledger in [&forward_a_ledger, &forward_b_ledger] {
        assert_eq!(
            side_effect_submission_count_for_ledger(&store, &fixture.run_id, forward_ledger),
            1
        );
        assert_eq!(
            remediation_submission_count_for_forward_ledger(
                &store,
                &fixture.run_id,
                forward_ledger
            ),
            1
        );
    }
}

#[tokio::test]
async fn runtime_resolves_clean_failure_without_acdc_claim() {
    let fixture = fixture();
    let failure_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::FailedWithoutAcdcClaim);
    assert!(saga.obligations.is_empty());

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("resolve clean failure terminal"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_materializes_confirmed_forward_output_before_failed_without_claim_terminal() {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let failure_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding side effect");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding failure node");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let forward_attempt = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &forward_node.node_id,
        1,
    )
    .expect("attempt id");
    for _ in 0..8 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("drive forward side-effect to confirmation"),
            SchedulerStatus::Advanced
        );
        let projection_snapshot = store.projection_snapshot();
        let projection = side_effect_projection_for_attempt(
            &projection_snapshot,
            &forward_node,
            &forward_attempt,
        )
        .expect("side-effect projection lookup")
        .expect("side-effect projection");
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());
        if matches!(
            projection.phase,
            store::SideEffectPhase::ConfirmationObserved { .. }
        ) {
            break;
        }
    }
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());
    let projection_snapshot = store.projection_snapshot();
    let projection =
        side_effect_projection_for_attempt(&projection_snapshot, &forward_node, &forward_attempt)
            .expect("side-effect projection lookup")
            .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ));

    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::FailedWithoutAcdcClaim);

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("materialize confirmed forward output"),
        SchedulerStatus::Advanced
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("resolve failed-without-claim terminal"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_resolves_manual_resolution_terminal() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture)
                .with_submission_decision(TestSubmissionDecision::Ambiguous),
        ))
        .expect("binding side effect");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding read");
    let mut store = TestTypedRunStore::new();
    let artifact_store = Arc::new(store.clone());
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry.clone(), &fixture),
        artifact_store.clone(),
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..2 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("advance to ambiguity"),
            SchedulerStatus::Advanced
        );
    }
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::ManualBlocked);
    assert_eq!(
        saga.manual_block_reason,
        Some(store::ManualBlockReason::PolicyManualResolution)
    );

    append_manual_resolution(
        &scheduler,
        &mut store,
        &fixture,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::ManuallyResolved);

    let fresh_scheduler = SerialTypedScheduler::new(
        register_fixture_capabilities(registry, &fixture),
        artifact_store,
    );
    assert_eq!(
        drive_once(
            &fresh_scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("resolve manual terminal"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::ManuallyResolved
    ));
}

#[tokio::test]
async fn runtime_rejects_manual_resolution_prefix_with_open_attempt() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture)
                .with_submission_decision(TestSubmissionDecision::Ambiguous),
        ))
        .expect("binding side effect");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding read");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..2 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("advance to manual block"),
            SchedulerStatus::Advanced
        );
    }
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::ManualBlocked);

    let node_b = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("node b")
        .clone();
    append_attempt_start(&mut store, &fixture, &node_b, 2);

    let spec::SagaPolicySpec::ManualResolution { manual } = &fixture.runtime_spec.spec().saga
    else {
        panic!("manual resolution fixture policy");
    };
    let error = build_manual_resolution_prefix_authority_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store,
        manual.clone(),
    )
    .expect_err("manual prefix rejects open attempt");
    assert!(
        matches!(&error, RuntimeError::InvalidRunStream(message) if message.contains("requires no open semantic attempts")),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_missing_manual_terminal_authorization_artifact_leaves_open_attempt() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture)
                .with_submission_decision(TestSubmissionDecision::Ambiguous),
        ))
        .expect("binding side effect");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding read");
    let mut store = TestTypedRunStore::new();
    let artifact_store = Arc::new(FilteringRuntimeArtifactStore::new(store.clone()));
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry, &fixture),
        artifact_store.clone(),
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..2 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("advance to ambiguity"),
            SchedulerStatus::Advanced
        );
    }
    append_manual_resolution(
        &scheduler,
        &mut store,
        &fixture,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    let manual = store
        .load_run_stream(&fixture.run_id)
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::ManualResolutionRecorded(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("manual resolution recorded");
    artifact_store.hide_artifact(manual.authorization_artifact_id.clone());
    let stream_len_before = store.load_run_stream(&fixture.run_id).len();

    let resolve_node = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .expect("resolve saga terminal node");
    assert!(matches!(
        drive_once(&scheduler, &mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::Store(message)) if message.contains("missing artifact")
    ));
    assert_eq!(
        store.load_run_stream(&fixture.run_id).len(),
        stream_len_before + 1
    );
    let resolve_attempt = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &resolve_node.node_id,
        1,
    )
    .expect("resolve attempt id");
    assert!(matches!(
        store
            .projection_snapshot()
            .attempt(&resolve_node.node_id, &resolve_attempt)
            .expect("open resolve attempt")
            .status,
        store::AttemptStatus::Started { .. }
    ));
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());
}

#[tokio::test]
async fn runtime_rejects_manual_resolution_before_manual_blocked() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture)
                .with_submission_decision(TestSubmissionDecision::Ambiguous),
        ))
        .expect("binding side effect");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding read");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let evidence_bytes = br#"{"operator_note":"too_early"}"#.to_vec();

    let error = record_manual_resolution(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
        ManualResolutionRequest {
            outcome: events::ManualResolutionOutcome::ConfirmRemediated,
            evidence_artifact: ManualResolutionEvidenceArtifact {
                bytes: evidence_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            proof_bytes: br#"{}"#.to_vec(),
            note: None,
        },
    )
    .await
    .expect_err("manual resolution before block rejects");

    assert!(
        matches!(error, RuntimeError::InvalidRunStream(_)),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_rejects_forward_node_emitting_remediation_ledger_purpose() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            ForwardEmitsRemediationPurposeRunner,
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize wrong forward ledger purpose"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
}

#[tokio::test]
async fn runtime_rejects_remediation_node_emitting_forward_ledger_purpose() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            RemediationEmitsForwardPurposeRunner::new(&fixture),
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "sidefx",
            RemediationEmitsForwardPurposeRunner::new(&fixture),
        ))
        .expect("binding b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "fail",
            BlockingRunner,
        ))
        .expect("binding failure node");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[fixture.cell_a.clone(), fixture.cell_b.clone()],
        "drive forward side-effect phase",
    )
    .await;
    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize wrong remediation ledger purpose"),
        SchedulerStatus::Advanced
    );
    assert_failure_code_count(&store, "runner_output_invalid", 1);
}

#[tokio::test]
async fn side_effect_not_submitted_resume_claims_next_epoch() {
    let fixture = fixture_with_first_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("prepare and start side effect");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    append_not_submitted_proven(&mut store, &fixture, node, &attempt_id, 1);

    drive_once(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
    .await
    .expect("resume not-submitted");
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(&projection_snapshot, node, &attempt_id)
        .expect("projection lookup")
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::InvocationStarted {
            invocation_epoch: 2,
            claim_generation: 2,
            ..
        }
    ));
}

#[tokio::test]
async fn side_effect_staged_artifact_must_match_payload_ledger_binding() {
    struct WrongLedgerStagedSideEffectRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    }

    impl ErasedNodeRunner for WrongLedgerStagedSideEffectRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let ledger = side_effect_ledger_key_for_ctx(&ctx);
                let staged_ledger =
                    events::SideEffectLedgerKey::new("wrong-ledger").expect("ledger key");
                let intent_bytes = br#"{"intent":"wrong-ledger"}"#.to_vec();
                let intent_hash = digest_for_bytes(&intent_bytes);
                let intent_artifact_id =
                    ArtifactId::from_digest(intent_hash.algorithm(), *intent_hash.digest());
                let evidence = store::ArtifactEvidenceRef {
                    artifact_id: intent_artifact_id.clone(),
                    digest: intent_hash.clone(),
                    byte_len: intent_bytes.len() as u64,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.node().config_ref.schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::SideEffectIntent,
                };
                let staged_artifact = StagedArtifact::inline_side_effect_artifact(
                    &ctx,
                    intent_bytes,
                    evidence,
                    staged_ledger,
                    1,
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: vec![
                        RunnerEventPayload::SideEffectIntentPersisted(
                            events::side_effect::IntentPersisted {
                                spec_hash: ctx.spec_hash().clone(),
                                node_id: ctx.node().node_id.clone(),
                                scope_id: ctx.node().scope_id.clone(),
                                attempt_id: ctx.attempt_id().clone(),
                                ledger_key: ledger.clone(),
                                ledger_purpose: side_effect_ledger_purpose_for_ctx(&ctx),
                                invocation_epoch: 1,
                                intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                                intent_hash,
                                intent_artifact_id,
                                idempotency_input_schema_id: ctx
                                    .node()
                                    .config_ref
                                    .schema_id
                                    .clone(),
                                idempotency_input_hash: content(0xc3),
                                idempotency_key: events::IdempotencyKeyRef::new("idem-1")
                                    .expect("idempotency key"),
                                capability_kind: self.cap_kind.clone(),
                                capability_version: self.cap_version.clone(),
                                adapter_kind: self.adapter_kind.clone(),
                                adapter_version: self.adapter_version.clone(),
                            },
                        ),
                        side_effect_claimed(&ctx, ledger.clone(), 1, 1),
                        side_effect_prepared(&ctx, ledger, 1, 1),
                    ],
                })
            })
        }
    }

    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            WrongLedgerStagedSideEffectRunner {
                cap_kind: side_effect_capability_kind(),
                cap_version: side_effect_capability_version(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize side-effect artifact binding mismatch"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
}

#[tokio::test]
async fn side_effect_ambiguous_phase_blocks_resume() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture)
                .with_submission_decision(TestSubmissionDecision::Ambiguous),
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..3 {
        assert_eq!(
            drive_once(
                &scheduler,
                &mut store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("advance to ambiguity"),
            SchedulerStatus::Advanced
        );
    }
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("ambiguous side effect resolves terminal"),
        SchedulerStatus::PublicOutputProjected
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn side_effect_ambiguity_blocks_independent_ready_nodes() {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(&fixture)
                .with_submission_decision(TestSubmissionDecision::Ambiguous),
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..3 {
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .expect("advance to ambiguity");
    }
    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("ambiguity resolves terminal"),
        SchedulerStatus::PublicOutputProjected
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_none());
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn side_effect_output_before_confirmation_is_rejected() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            PrematureSideEffectOutputRunner {
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("terminalize premature side-effect output"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
}

#[test]
fn side_effect_failure_derives_attempt_failure_payload() {
    let fixture = fixture_with_first_side_effect_state();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let payloads = runner_payloads_with_derived_lifecycle(
        &fixture.runtime_spec,
        node,
        &attempt_id,
        vec![RunnerEventPayload::SideEffectFailed(
            events::side_effect::Failed {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: side_effect_ledger_key(1),
                ledger_purpose: side_effect_ledger_purpose(),
                invocation_epoch: 1,
                failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                retryable: false,
                error: side_effect_error(false),
            },
        )],
    );
    let payloads = payloads.expect("derive lifecycle");
    assert!(
        payloads.iter().any(|payload| {
            matches!(
                payload,
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    retryable: false,
                    ..
                })
            )
        }),
        "side-effect failure payloads should include middleware-derived StateAttemptFailed"
    );
}

#[test]
fn side_effect_ambiguity_derives_attempt_failure_payload() {
    let fixture = fixture_with_first_side_effect_state();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let payloads = runner_payloads_with_derived_lifecycle(
        &fixture.runtime_spec,
        node,
        &attempt_id,
        vec![RunnerEventPayload::SideEffectAmbiguous(
            events::side_effect::Ambiguous {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: side_effect_ledger_key(1),
                ledger_purpose: side_effect_ledger_purpose(),
                invocation_epoch: 1,
                ambiguity_code: events::AmbiguityCode::new("unknown_submission")
                    .expect("ambiguity code"),
                evidence_schema_id: node.config_ref.schema_id.clone(),
                evidence_hash: content(0xca),
                evidence_artifact_id: artifact(0xcb),
            },
        )],
    );
    let payloads = payloads.expect("derive lifecycle");
    let failure = payloads
        .iter()
        .find_map(|payload| match payload {
            events::KernelEventPayload::StateAttemptFailed(payload) => Some(payload),
            _ => None,
        })
        .expect("derived attempt failure");
    assert!(!failure.retryable);
    assert_eq!(failure.error.code.as_str(), "side_effect_ambiguous");
}

#[derive(Clone)]
struct DriverSideEffectRunner {
    callbacks: TestSideEffectDriverCallbacks,
}

impl DriverSideEffectRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
        }
    }

    fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.callbacks = self.callbacks.with_submission_decision(decision);
        self
    }
}

impl ErasedNodeRunner for DriverSideEffectRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let Some(resource_key) = test_driver_resource_key_for_node(ctx.node()) else {
                return Ok(ErasedRunnerOutput::new(Vec::new()));
            };
            let plan = self.callbacks.intent_plan_for(
                ctx.node().node_id.as_str().to_owned(),
                ctx.attempt_id().as_str().to_owned(),
            )?;
            SideEffectLanePreclaimBuilder::new(ctx).claim_resource_lane(
                &plan.intent,
                &plan.idempotency,
                plan.idempotency_key,
                plan.capability_binding,
                resource_key,
            )
        })
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { SideEffectDriver::drive(ctx, &self.callbacks).await })
    }
}

#[derive(Clone)]
struct FailingAfterPreclaimRunner {
    callbacks: TestSideEffectDriverCallbacks,
}

impl FailingAfterPreclaimRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
        }
    }
}

impl ErasedNodeRunner for FailingAfterPreclaimRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let Some(resource_key) = test_driver_resource_key_for_node(ctx.node()) else {
                return Ok(ErasedRunnerOutput::new(Vec::new()));
            };
            let plan = self.callbacks.intent_plan_for(
                ctx.node().node_id.as_str().to_owned(),
                ctx.attempt_id().as_str().to_owned(),
            )?;
            SideEffectLanePreclaimBuilder::new(ctx).claim_resource_lane(
                &plan.intent,
                &plan.idempotency,
                plan.idempotency_key,
                plan.capability_binding,
                resource_key,
            )
        })
    }

    fn run_erased<'a>(&'a self, _ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            Err(RuntimeError::InvalidRunnerOutput(
                "prepare invocation failed after exclusive resource claim".to_owned(),
            ))
        })
    }
}

struct PrePreparedSideEffectRunner {
    emit_claim: bool,
}

impl ErasedNodeRunner for PrePreparedSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            if side_effect_projection_for_attempt(ctx.projections(), ctx.node(), ctx.attempt_id())?
                .is_some()
            {
                return Err(RuntimeError::Blocked(
                    "pre-prepared side-effect runner should not resume".to_owned(),
                ));
            }
            let ledger = side_effect_ledger_key_for_ctx(&ctx);
            let (intent_artifact_id, intent_hash) =
                side_effect_fixture_artifact_pair(&ctx, "intent");
            let staged_artifact = staged_side_effect_artifact(
                &ctx,
                side_effect_artifact(
                    &ctx,
                    intent_artifact_id.clone(),
                    intent_hash.clone(),
                    events::ArtifactRole::SideEffectIntent,
                ),
                ledger.clone(),
                1,
            )?;
            let mut payloads = vec![RunnerEventPayload::SideEffectIntentPersisted(
                events::side_effect::IntentPersisted {
                    spec_hash: ctx.spec_hash().clone(),
                    node_id: ctx.node().node_id.clone(),
                    scope_id: ctx.node().scope_id.clone(),
                    attempt_id: ctx.attempt_id().clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: side_effect_ledger_purpose_for_ctx(&ctx),
                    invocation_epoch: 1,
                    intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                    intent_hash,
                    intent_artifact_id,
                    idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
                    idempotency_input_hash: side_effect_fixture_digest(&ctx, "idempotency"),
                    idempotency_key: events::IdempotencyKeyRef::new("idem-1")
                        .expect("idempotency key"),
                    capability_kind: side_effect_capability_kind(),
                    capability_version: side_effect_capability_version(),
                    adapter_kind: ctx
                        .node()
                        .adapter_bindings
                        .first()
                        .expect("side-effect adapter")
                        .adapter_kind
                        .clone(),
                    adapter_version: ctx
                        .node()
                        .adapter_bindings
                        .first()
                        .expect("side-effect adapter")
                        .adapter_version
                        .clone(),
                },
            )];
            if self.emit_claim {
                payloads.push(side_effect_claimed(&ctx, ledger, 1, 1));
            }
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: Vec::new(),
                payloads,
            })
        })
    }
}

struct TouchedSetSideEffectRunner {
    inner: DriverSideEffectRunner,
    receipt: TouchedSetEmission,
    confirmation: TouchedSetEmission,
}

#[derive(Clone, Copy)]
enum TouchedSetEmission {
    None,
    MatchPayloadSchema,
}

impl TouchedSetSideEffectRunner {
    fn with_receipt(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
            receipt: TouchedSetEmission::MatchPayloadSchema,
            confirmation: TouchedSetEmission::None,
        }
    }

    fn with_confirmation(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
            receipt: TouchedSetEmission::None,
            confirmation: TouchedSetEmission::MatchPayloadSchema,
        }
    }
}

impl ErasedNodeRunner for TouchedSetSideEffectRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        self.inner.preclaim_resource_lane(ctx)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let mut output = self.inner.run_erased(ctx).await?;
            for payload in &mut output.payloads {
                match payload {
                    RunnerEventPayload::SideEffectReceiptObserved(payload) => {
                        payload.resource_touched_set = touched_set_for_emission(
                            self.receipt,
                            &payload.receipt_schema_id,
                            &payload.receipt_hash,
                            &payload.receipt_artifact_id,
                        );
                    }
                    RunnerEventPayload::SideEffectConfirmationObserved(payload) => {
                        payload.resource_touched_set = touched_set_for_emission(
                            self.confirmation,
                            &payload.confirmation_schema_id,
                            &payload.confirmation_hash,
                            &payload.confirmation_artifact_id,
                        );
                    }
                    _ => {}
                }
            }
            Ok(output)
        })
    }
}

struct FailActiveSideEffectAfterSagaRunner {
    inner: DriverSideEffectRunner,
    stop_before_invocation_started: bool,
}

impl FailActiveSideEffectAfterSagaRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
            stop_before_invocation_started: false,
        }
    }

    fn before_invocation_started(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
            stop_before_invocation_started: true,
        }
    }

    fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.inner = self.inner.with_submission_decision(decision);
        self
    }
}

impl ErasedNodeRunner for FailActiveSideEffectAfterSagaRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        self.inner.preclaim_resource_lane(ctx)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let saga = ctx
                .projections()
                .derive_saga_projection(ctx.run_id(), &ctx.runtime_spec().spec().saga);
            let projected = side_effect_projection_for_attempt(
                ctx.projections(),
                ctx.node(),
                ctx.attempt_id(),
            )?
            .map(|projection| (projection.ledger_key.clone(), projection.phase.clone()));
            if saga.engagement.is_some() {
                if let Some((ledger, phase)) = &projected {
                    if let Some((invocation_epoch, failure_phase)) =
                        saga_closure_failure_for_phase(phase)
                    {
                        return Ok(ErasedRunnerOutput::new(vec![side_effect_failed(
                            &ctx,
                            ledger.clone(),
                            invocation_epoch,
                            failure_phase,
                            false,
                        )]));
                    }
                }
            }
            if self.stop_before_invocation_started && projected.is_none() {
                return prepared_boundary_side_effect_output(ctx);
            }
            self.inner.run_erased(ctx).await
        })
    }
}

fn prepared_boundary_side_effect_output(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let ledger = side_effect_ledger_key_for_ctx(&ctx);
    let (intent_artifact_id, intent_hash) = side_effect_fixture_artifact_pair(&ctx, "intent");
    let staged_artifact = staged_side_effect_artifact(
        &ctx,
        side_effect_artifact(
            &ctx,
            intent_artifact_id.clone(),
            intent_hash.clone(),
            events::ArtifactRole::SideEffectIntent,
        ),
        ledger.clone(),
        1,
    )?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: Vec::new(),
        payloads: vec![
            RunnerEventPayload::SideEffectIntentPersisted(events::side_effect::IntentPersisted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                scope_id: ctx.node().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger.clone(),
                ledger_purpose: side_effect_ledger_purpose_for_ctx(&ctx),
                invocation_epoch: 1,
                intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                intent_hash,
                intent_artifact_id,
                idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
                idempotency_input_hash: side_effect_fixture_digest(&ctx, "idempotency"),
                idempotency_key: events::IdempotencyKeyRef::new("idem-1").expect("idempotency key"),
                capability_kind: side_effect_capability_kind(),
                capability_version: side_effect_capability_version(),
                adapter_kind: ctx
                    .node()
                    .adapter_bindings
                    .first()
                    .expect("side-effect adapter")
                    .adapter_kind
                    .clone(),
                adapter_version: ctx
                    .node()
                    .adapter_bindings
                    .first()
                    .expect("side-effect adapter")
                    .adapter_version
                    .clone(),
            }),
            side_effect_claimed(&ctx, ledger.clone(), 1, 1),
            side_effect_prepared(&ctx, ledger, 1, 1),
        ],
    })
}

fn saga_closure_failure_for_phase(
    phase: &store::SideEffectPhase,
) -> Option<(u32, events::side_effect::FailurePhase)> {
    match phase {
        store::SideEffectPhase::IntentPersisted { invocation_epoch }
        | store::SideEffectPhase::Claimed {
            invocation_epoch, ..
        }
        | store::SideEffectPhase::InvocationPrepared {
            invocation_epoch, ..
        } => Some((
            *invocation_epoch,
            events::side_effect::FailurePhase::BeforeInvocationStarted,
        )),
        store::SideEffectPhase::NotSubmittedProven { invocation_epoch } => Some((
            *invocation_epoch,
            events::side_effect::FailurePhase::AfterNotSubmittedProven,
        )),
        _ => None,
    }
}

fn touched_set_for_emission(
    emission: TouchedSetEmission,
    evidence_schema_id: &SchemaId,
    evidence_hash: &ContentDigest,
    evidence_artifact_id: &ArtifactId,
) -> Option<events::ResourceTouchedSetEvidence> {
    match emission {
        TouchedSetEmission::None => None,
        TouchedSetEmission::MatchPayloadSchema => Some(events::ResourceTouchedSetEvidence {
            namespace: exact_touched_set_resource_namespace(),
            evidence_schema_id: evidence_schema_id.clone(),
            evidence_hash: evidence_hash.clone(),
            evidence_artifact_id: evidence_artifact_id.clone(),
        }),
    }
}

struct ForwardEmitsRemediationPurposeRunner;

impl ErasedNodeRunner for ForwardEmitsRemediationPurposeRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let forward_ledger_key =
                events::SideEffectLedgerKey::new("foreign-forward").expect("forward ledger key");
            side_effect_intent_with_purpose(
                ctx,
                events::SideEffectLedgerPurpose::Remediation { forward_ledger_key },
            )
        })
    }
}

struct RemediationEmitsForwardPurposeRunner {
    inner: DriverSideEffectRunner,
}

impl RemediationEmitsForwardPurposeRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
        }
    }
}

impl ErasedNodeRunner for RemediationEmitsForwardPurposeRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        self.inner.preclaim_resource_lane(ctx)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            if ctx
                .runtime_spec()
                .forward_node_for_remediation(&ctx.node().node_id)
                .is_some()
            {
                side_effect_intent_with_purpose(ctx, events::SideEffectLedgerPurpose::Forward)
            } else {
                self.inner.run_erased(ctx).await
            }
        })
    }
}

fn side_effect_intent_with_purpose(
    ctx: ErasedRunCtx<'_>,
    ledger_purpose: events::SideEffectLedgerPurpose,
) -> Result<ErasedRunnerOutput> {
    let ledger = side_effect_ledger_key_for_ctx(&ctx);
    let (intent_artifact_id, intent_hash) = side_effect_fixture_artifact_pair(&ctx, "intent");
    let staged_artifact = staged_side_effect_artifact(
        &ctx,
        side_effect_artifact(
            &ctx,
            intent_artifact_id.clone(),
            intent_hash.clone(),
            events::ArtifactRole::SideEffectIntent,
        ),
        ledger.clone(),
        1,
    )?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: Vec::new(),
        payloads: vec![RunnerEventPayload::SideEffectIntentPersisted(
            events::side_effect::IntentPersisted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                scope_id: ctx.node().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger,
                ledger_purpose,
                invocation_epoch: 1,
                intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                intent_hash,
                intent_artifact_id,
                idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
                idempotency_input_hash: side_effect_fixture_digest(&ctx, "idempotency"),
                idempotency_key: events::IdempotencyKeyRef::new("idem-1").expect("idempotency key"),
                capability_kind: side_effect_capability_kind(),
                capability_version: side_effect_capability_version(),
                adapter_kind: ctx
                    .node()
                    .adapter_bindings
                    .first()
                    .expect("side-effect adapter")
                    .adapter_kind
                    .clone(),
                adapter_version: ctx
                    .node()
                    .adapter_bindings
                    .first()
                    .expect("side-effect adapter")
                    .adapter_version
                    .clone(),
            },
        )],
    })
}

struct PrematureSideEffectOutputRunner {
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
}

impl ErasedNodeRunner for PrematureSideEffectOutputRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let artifact = state_output_artifact(
                ctx.node(),
                ctx.descriptor(),
                self.output_artifact.clone(),
                self.output_digest.clone(),
            );
            let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: Vec::new(),
                payloads: terminal_payloads(
                    &ctx,
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                ),
            })
        })
    }
}

struct StaleStreamStore<'a> {
    inner: RefCell<&'a mut TestTypedRunStore>,
    stream: Vec<store::KernelEventEnvelope>,
}

impl<'a> StaleStreamStore<'a> {
    fn new(inner: &'a mut TestTypedRunStore, stream: Vec<store::KernelEventEnvelope>) -> Self {
        Self {
            inner: RefCell::new(inner),
            stream,
        }
    }
}

impl store::RunEventStore for StaleStreamStore<'_> {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let result = block_on_ready(
            self.inner
                .borrow_mut()
                .append_prepared_commit_bundle(bundle),
        );
        Box::pin(std::future::ready(result))
    }

    fn load_run_stream<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        Box::pin(std::future::ready(Ok(self.stream.clone())))
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        let result = Ok(self.inner.borrow().expected_next_seq(run_id));
        Box::pin(std::future::ready(result))
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }
}

struct MissingInputArtifactRefStore<'a> {
    inner: RefCell<&'a mut TestTypedRunStore>,
    producer_node_id: NodeId,
}

impl<'a> MissingInputArtifactRefStore<'a> {
    fn new(inner: &'a mut TestTypedRunStore, producer_node_id: NodeId) -> Self {
        Self {
            inner: RefCell::new(inner),
            producer_node_id,
        }
    }
}

impl store::RunEventStore for MissingInputArtifactRefStore<'_> {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let result = block_on_ready(
            self.inner
                .borrow_mut()
                .append_prepared_commit_bundle(bundle),
        );
        Box::pin(std::future::ready(result))
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        let result = rewrite_stream_without_payloads(
            &self.inner.borrow().load_run_stream(run_id),
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::ArtifactReferenced(payload)
                        if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                            && payload.node_id.as_ref() == Some(&self.producer_node_id)
                )
            },
        );
        Box::pin(std::future::ready(Ok(result)))
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        let result = Ok(self.inner.borrow().expected_next_seq(run_id));
        Box::pin(std::future::ready(result))
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }
}

fn rewrite_envelope(
    event: &store::KernelEventEnvelope,
    seq: store::StreamSeq,
    ordinal: store::CommitOrdinal,
    commit_key: store::CommitKey,
) -> store::KernelEventEnvelope {
    store::KernelEventEnvelope::from_persisted_record(store::PersistedKernelEventRecord {
        event_id: event_id_for(event, seq, ordinal),
        event_schema_id: event.event_schema_id().clone(),
        run_id: event.run_id().clone(),
        seq,
        ordinal,
        spec_hash: event.spec_hash().clone(),
        commit_key,
        logical_key: event.logical_key().clone(),
        payload_hash: event.payload_hash().clone(),
        payload: event.payload().clone(),
    })
    .expect("rewritten envelope")
}

fn rewrite_stream_without_payloads<F>(
    stream: &[store::KernelEventEnvelope],
    mut should_remove: F,
) -> Vec<store::KernelEventEnvelope>
where
    F: FnMut(&events::KernelEventPayload) -> bool,
{
    let mut rewritten = Vec::with_capacity(stream.len());
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        let payloads = commit
            .iter()
            .filter(|event| !should_remove(event.payload()))
            .map(|event| event.payload().clone())
            .collect::<Vec<_>>();
        assert!(
            !payloads.is_empty(),
            "test corruption helper must not remove an entire commit"
        );
        if payloads.len() == commit.len() {
            rewritten.extend(commit.iter().cloned());
        } else {
            let mut ordinal = 0;
            for event in commit {
                if should_remove(event.payload()) {
                    continue;
                }
                rewritten.push(rewrite_envelope(
                    event,
                    seq,
                    store::CommitOrdinal::new(ordinal),
                    commit_key.clone(),
                ));
                ordinal += 1;
            }
        }
        index = end;
    }
    rewritten
}

fn event_id_for(
    event: &store::KernelEventEnvelope,
    seq: store::StreamSeq,
    ordinal: store::CommitOrdinal,
) -> EventId {
    let canonical = canonical_json(serde_json::json!({
        "event_schema_id": event.event_schema_id().as_str(),
        "ordinal": ordinal.as_u32(),
        "payload_hash": event.payload_hash().as_str(),
        "run_id": event.run_id().as_str(),
        "seq": seq.as_u64(),
    }))
    .expect("event id canonical");
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, canonical.digest_bytes())
}

fn assert_every_certified_node_has_attempt(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) {
    let mut started = BTreeSet::new();
    let mut completed = BTreeSet::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                started.insert(payload.node_id.clone());
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                completed.insert(payload.node_id.clone());
            }
            _ => {}
        }
    }
    for node in &runtime_spec.spec().nodes {
        if matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        ) {
            continue;
        }
        assert!(
            started.contains(&node.node_id),
            "node {} has no StateAttemptStarted",
            node.node_id
        );
        assert!(
            completed.contains(&node.node_id),
            "node {} has no StateAttemptCompleted",
            node.node_id
        );
    }
}

fn referenced_artifact_ids_for_payload(payload: &events::KernelEventPayload) -> Vec<ArtifactId> {
    let mut artifacts = Vec::new();
    match payload {
        events::KernelEventPayload::RunAdmitted(payload) => {
            artifacts.push(payload.spec_artifact.artifact_id.clone());
            artifacts.push(payload.certificate_artifact.artifact_id.clone());
            artifacts.extend(
                payload
                    .config_artifacts
                    .iter()
                    .map(|artifact| artifact.artifact_id.clone()),
            );
            artifacts.extend(
                payload
                    .seed_cells
                    .iter()
                    .map(|seed| seed.seed_artifact.artifact_id.clone()),
            );
        }
        events::KernelEventPayload::FactRecorded(payload) => {
            artifacts.push(payload.artifact_id.clone());
        }
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            artifacts.push(payload.artifact_ref.artifact_id.clone());
        }
        events::KernelEventPayload::CellProduced(payload) => {
            artifacts.push(payload.artifact_id.clone());
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            artifacts.extend(payload.cells.iter().map(|cell| cell.artifact_id.clone()));
            if let Some(artifact_id) = &payload.rendered_artifact_id {
                artifacts.push(artifact_id.clone());
            }
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            if let Some(evidence) = &payload.error.diagnostic_ref {
                artifacts.push(evidence.artifact_id.clone());
            }
        }
        events::KernelEventPayload::StateAttemptFailed(payload) => {
            if let Some(evidence) = &payload.error.diagnostic_ref {
                artifacts.push(evidence.artifact_id.clone());
            }
        }
        events::KernelEventPayload::ManualResolutionRecorded(payload) => {
            artifacts.push(payload.evidence_artifact_id.clone());
            artifacts.push(payload.authorization_artifact_id.clone());
        }
        events::KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
            events::RunCompletionOutcome::Completed(_) => {}
            events::RunCompletionOutcome::Compensated
            | events::RunCompletionOutcome::ManuallyResolved
            | events::RunCompletionOutcome::FailedWithoutAcdcClaim => {}
        },
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            artifacts.push(payload.intent_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            if let Some(artifact_id) = &payload.prepared_artifact_id {
                artifacts.push(artifact_id.clone());
            }
        }
        events::KernelEventPayload::ResourceLaneClaimed(_)
        | events::KernelEventPayload::ResourceLaneClaimIntent(_)
        | events::KernelEventPayload::ResourceLaneReleased(_)
        | events::KernelEventPayload::ResourceLaneReleaseIntent(_) => {}
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            artifacts.push(payload.proof_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            artifacts.push(payload.submission_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            artifacts.push(payload.evidence_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
            artifacts.push(payload.receipt_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            artifacts.push(payload.confirmation_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectAmbiguous(payload) => {
            artifacts.push(payload.evidence_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectFailed(payload) => {
            if let Some(evidence) = &payload.error.diagnostic_ref {
                artifacts.push(evidence.artifact_id.clone());
            }
        }
        events::KernelEventPayload::RetentionRefsAppended(payload) => {
            artifacts.extend(
                payload
                    .refs
                    .iter()
                    .map(|reference| reference.artifact_id.clone()),
            );
        }
        events::KernelEventPayload::RetentionManifestProjected(payload) => {
            artifacts.push(payload.manifest_artifact_id.clone());
        }
        events::KernelEventPayload::StateAttemptStarted(_)
        | events::KernelEventPayload::StateAttemptInterrupted(_)
        | events::KernelEventPayload::CellSkipped(_)
        | events::KernelEventPayload::SideEffectClaimed(_)
        | events::KernelEventPayload::SideEffectClaimTakenOver(_)
        | events::KernelEventPayload::SideEffectInvocationStarted(_)
        | events::KernelEventPayload::StateAttemptCompleted(_) => {}
    }
    artifacts
}

fn prepare_fixture_launch(
    scheduler: &SerialTypedScheduler,
    store: &TestTypedRunStore,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<PreparedRunLaunch> {
    scheduler.prepare_run_launch(
        &fixture.runtime_spec,
        fixture_run_identity_material(fixture),
        run_start_evidence(fixture, seed_cells),
        store.expected_next_seq(&fixture.run_id),
    )
}

async fn start_fixture_run(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<store::CommitOutcome> {
    let launch = prepare_fixture_launch(scheduler, store, fixture, seed_cells)?;
    scheduler_start_run(scheduler, store, launch).await
}

async fn start_fixture_run_async_store<S: store::RunEventStore + ?Sized>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<store::CommitOutcome> {
    let expected_next_seq = store
        .expected_next_seq(&fixture.run_id)
        .await
        .map_err(crate::error::async_store_error)?;
    let launch = scheduler.prepare_run_launch(
        &fixture.runtime_spec,
        fixture_run_identity_material(fixture),
        run_start_evidence(fixture, seed_cells),
        expected_next_seq,
    )?;
    scheduler.start_run(store, launch).await
}

async fn scheduler_start_run(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    launch: PreparedRunLaunch,
) -> Result<store::CommitOutcome> {
    scheduler.start_run(&*store, launch).await
}

async fn scheduler_start_run_admitted(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    launch: PreparedRunLaunch,
) -> Result<RunAdmissionAuthority> {
    scheduler
        .start_run_admitted(&*store, runtime_spec, launch)
        .await
}

async fn drive_once(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus> {
    scheduler.drive_once(&*store, runtime_spec, run_id).await
}

async fn drive_until_blocked(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus> {
    scheduler
        .drive_until_blocked(&*store, runtime_spec, run_id)
        .await
}

async fn record_manual_resolution(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    request: ManualResolutionRequest,
) -> Result<store::CommitOutcome> {
    scheduler
        .record_manual_resolution(&*store, runtime_spec, run_id, request)
        .await
}

fn build_manual_resolution_prefix_authority_for_tests(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    store: &TestTypedRunStore,
    manual: spec::ManualResolutionEvidenceSpec,
) -> Result<mfm_manual_auth::ManualResolutionPrefixAuthority> {
    let projection_snapshot = store.projection_snapshot();
    crate::manual_resolution::build_manual_resolution_prefix_authority_from_parts(
        runtime_spec,
        run_id,
        manual,
        &store.load_run_stream(run_id),
        store.expected_next_seq(run_id),
        &projection_snapshot,
    )
}

fn run_start_evidence(
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> RunLaunchEvidence {
    RunLaunchEvidence {
        entry_point: entry_point_launch_evidence(),
        spec_artifact: spec_artifact(&fixture.runtime_spec),
        certificate_artifact: certificate_artifact(&fixture.runtime_spec),
        config_artifacts: fixture
            .runtime_spec
            .spec()
            .config_refs
            .iter()
            .map(|config| config_artifact(&fixture.runtime_spec, config))
            .collect(),
        adapter_executables: Vec::new(),
        seed_cells: seed_cells.into_iter().map(seed_launch_cell).collect(),
    }
}

fn entry_point_launch_evidence() -> events::EntryPointLaunchEvidence {
    events::EntryPointLaunchEvidence {
        resolved_op_id: events::EntryPointOpId::new("mfm.test:portfolio_snapshot:1")
            .expect("entry-point op id"),
        entry_point_registry_digest: digest_for_bytes(b"entry-point-registry"),
    }
}

fn spec_artifact(runtime_spec: &CertifiedRuntimeSpec) -> RunLaunchArtifact {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .expect("canonical spec");
    let digest = canonical.content_digest();
    RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: runtime_spec.spec().media_type.clone(),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedExecutionSpec,
        },
    }
}

fn certificate_artifact(runtime_spec: &CertifiedRuntimeSpec) -> RunLaunchArtifact {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .expect("canonical certificate");
    let digest = canonical.content_digest();
    RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
                .expect("certificate media type"),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedSpecCertificate,
        },
    }
}

fn config_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    config: &spec::ConfigRef,
) -> RunLaunchArtifact {
    let bytes = runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.config_ref == *config && node.framework.is_some())
        .and_then(|node| {
            node.framework.as_ref().map(|framework| {
                spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
                    .expect("framework config")
                    .to_vec()
            })
        })
        .unwrap_or_else(|| TEST_CONFIG_BYTES.to_vec());
    assert_eq!(digest_for_bytes(&bytes), config.digest);
    RunLaunchArtifact {
        bytes,
        evidence: store::ArtifactEvidenceRef {
            artifact_id: config.artifact_id.clone(),
            digest: config.digest.clone(),
            byte_len: config.byte_len,
            media_type: config.media_type.clone(),
            schema_id: Some(config.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedConfig,
        },
    }
}

fn seed_launch_cell(seed: events::SeedCellRef) -> RunLaunchSeedCell {
    RunLaunchSeedCell {
        bytes: TEST_SEED_BYTES.to_vec(),
        cell: seed,
    }
}

fn terminal_payloads(
    ctx: &ErasedRunCtx<'_>,
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
) -> Vec<RunnerEventPayload> {
    vec![RunnerEventPayload::CellProduced(events::CellProduced {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        cell_id: ctx.node().output_cell.clone(),
        scope_id: ctx.node().scope_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
        schema_id: ctx.descriptor().output_schema_id.clone(),
        value_lineage: ctx.output_cell().value_lineage.clone(),
        artifact_id: output_artifact,
        content_digest: output_digest,
        producer_state_kind: Some(ctx.node().state_kind.clone()),
        producer_state_version: Some(ctx.node().state_version.clone()),
    })]
}

fn state_output_artifact(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(descriptor.output_schema_id.clone()),
        semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    }
}

fn state_output_artifact_for_bytes(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    bytes: &[u8],
) -> store::ArtifactEvidenceRef {
    let digest = digest_for_bytes(bytes);
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(descriptor.output_schema_id.clone()),
        semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    }
}

fn digest_for_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn runtime_staging_class(role: events::ArtifactRole) -> &'static str {
    match role.contract().staging {
        events::ArtifactStagingClass::AttemptStateOutput
        | events::ArtifactStagingClass::AttemptFactResponse
        | events::ArtifactStagingClass::AttemptPublicOutput
        | events::ArtifactStagingClass::AttemptRedactedDiagnostic => {
            assert!(staged_artifact_binding_kind(role).is_some());
            assert!(staged_side_effect_artifact_phase(role).is_none());
        }
        events::ArtifactStagingClass::SideEffectIntent
        | events::ArtifactStagingClass::SideEffectPreparedInvocation
        | events::ArtifactStagingClass::SideEffectNotSubmittedProof
        | events::ArtifactStagingClass::SideEffectSubmission
        | events::ArtifactStagingClass::SideEffectSubmissionUnknown
        | events::ArtifactStagingClass::SideEffectReceipt
        | events::ArtifactStagingClass::SideEffectConfirmation
        | events::ArtifactStagingClass::SideEffectAmbiguity => {
            assert!(staged_artifact_binding_kind(role).is_none());
            assert!(staged_side_effect_artifact_phase(role).is_some());
        }
        events::ArtifactStagingClass::RunAdmission
        | events::ArtifactStagingClass::ManualResolution
        | events::ArtifactStagingClass::MiddlewareRetentionManifest => {
            assert!(staged_artifact_binding_kind(role).is_none());
            assert!(staged_side_effect_artifact_phase(role).is_none());
        }
    }
    role.contract().staging.as_str()
}

#[test]
fn artifact_role_contract_runtime_staging_matches_current_helpers() {
    let rows = events::ArtifactRole::ALL
        .iter()
        .copied()
        .map(|role| format!("{role:?} -> {}", runtime_staging_class(role)))
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        rows,
        "TypedExecutionSpec -> run_admission\n\
TypedSpecCertificate -> run_admission\n\
TypedConfig -> run_admission\n\
SeedInput -> run_admission\n\
StateOutput -> attempt_state_output\n\
FactResponse -> attempt_fact_response\n\
SideEffectIntent -> side_effect_intent\n\
PreparedInvocation -> side_effect_prepared_invocation\n\
NotSubmittedProof -> side_effect_not_submitted_proof\n\
Submission -> side_effect_submission\n\
SubmissionUnknownEvidence -> side_effect_submission_unknown\n\
Receipt -> side_effect_receipt\n\
Confirmation -> side_effect_confirmation\n\
AmbiguityEvidence -> side_effect_ambiguity\n\
ManualResolutionEvidence -> manual_resolution\n\
ManualResolutionAuthorization -> manual_resolution\n\
PublicOutput -> attempt_public_output\n\
RedactedDiagnostic -> attempt_redacted_diagnostic\n\
RetentionManifest -> middleware_retention_manifest"
    );
}

fn staged_attempt_artifact(
    ctx: &ErasedRunCtx<'_>,
    evidence: store::ArtifactEvidenceRef,
) -> Result<StagedArtifact> {
    let binding = staged_artifact_binding_kind(evidence.artifact_role).expect("staged role");
    StagedArtifact::finalized_attempt_artifact_for_tests(ctx, evidence, binding)
}

fn staged_side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    evidence: store::ArtifactEvidenceRef,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> Result<StagedArtifact> {
    let phase =
        staged_side_effect_artifact_phase(evidence.artifact_role).expect("staged side-effect role");
    StagedArtifact::finalized_attempt_artifact_for_tests(
        ctx,
        evidence,
        StagedArtifactBindingKind::SideEffectEvidence {
            ledger_key,
            invocation_epoch,
            phase,
        },
    )
}

fn node_by_output<'a>(fixture: &'a Fixture, cell_id: &CellId) -> &'a spec::NodeSpec {
    fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| &node.output_cell == cell_id)
        .expect("node by output")
}

fn append_attempt_start(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_no: u32,
) -> AttemptId {
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        attempt_no,
    )
    .expect("attempt id");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-attempt-start:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append attempt start");
    attempt_id
}

fn append_attempt_failure(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    retryable: bool,
) {
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-attempt-failure:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptFailed(
                events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable,
                    error: events::MfmErrorInfo {
                        retryable,
                        ..public_output_error()
                    },
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append attempt failure");
}

fn append_synthetic_exclusive_prepare(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    key: &str,
    commit_key: &str,
) -> (AttemptId, events::SideEffectLedgerKey) {
    let attempt_id =
        attempt_id(run_id, fixture.runtime_spec.spec_hash(), &node.node_id, 1).expect("attempt id");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(format!("{commit_key}-attempt-start"))
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no: 1,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic attempt start");

    let ledger =
        events::SideEffectLedgerKey::new(format!("holder-{commit_key}")).expect("holder ledger");
    let intent_hash = content_digest_json(serde_json::json!({
        "key": key,
        "ledger": ledger.as_str(),
        "run": run_id.as_str(),
    }))
    .expect("intent digest");
    let resource_key = exclusive_resource_key(fixture, key);
    let resource_lane_requirement_digest = content_digest_json(serde_json::json!({
        "acquisition": "pre_state_invocation",
        "hold": "until_side_effect_terminal",
        "key_schema_id": resource_key.key_schema_id.as_str(),
        "mode": "exclusive",
        "namespace": resource_key.namespace.as_str(),
    }))
    .expect("resource lane requirement digest");
    let intent_artifact_id =
        ArtifactId::from_digest(intent_hash.algorithm(), *intent_hash.digest());
    let intent_artifact = store::ArtifactEvidenceRef {
        artifact_id: intent_artifact_id.clone(),
        digest: intent_hash.clone(),
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::SideEffectIntent,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::SideEffectIntentPersisted(
                    events::side_effect::IntentPersisted {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        scope_id: node.scope_id.clone(),
                        attempt_id: attempt_id.clone(),
                        ledger_key: ledger.clone(),
                        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                        invocation_epoch: 1,
                        intent_schema_id: node.config_ref.schema_id.clone(),
                        intent_hash: intent_hash.clone(),
                        intent_artifact_id,
                        idempotency_input_schema_id: node.config_ref.schema_id.clone(),
                        idempotency_input_hash: content(0xc3),
                        idempotency_key: events::IdempotencyKeyRef::new(format!(
                            "idem-{commit_key}"
                        ))
                        .expect("idempotency key"),
                        capability_kind: side_effect_capability_kind(),
                        capability_version: side_effect_capability_version(),
                        adapter_kind: fixture.adapter_kind.clone(),
                        adapter_version: fixture.adapter_version.clone(),
                    },
                ),
                events::KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
                    invocation_epoch: 1,
                    claim_generation: 1,
                    claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                        .expect("fencing token"),
                }),
                events::KernelEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    invocation_epoch: 1,
                    resource_key: resource_key.clone(),
                    requirement_digest: resource_lane_requirement_digest,
                    resolved_by_capability_impl: events::RunnerFactoryId::new(
                        "mfm.test.side_effect_driver",
                    )
                    .expect("runner factory"),
                }),
                events::KernelEventPayload::SideEffectInvocationPrepared(
                    events::side_effect::InvocationPrepared {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        ledger_key: ledger.clone(),
                        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                        invocation_epoch: 1,
                        claim_generation: 1,
                        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                            .expect("fencing token"),
                        resource_key: Some(resource_key),
                        prepared_artifact_id: None,
                        prepared_hash: None,
                    },
                ),
            ],
            required_artifacts: vec![intent_artifact],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    ledger_key: ledger.clone(),
                    required: store::RequiredSideEffectState::Absent,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic side-effect prepare");
    (attempt_id, ledger)
}

fn append_synthetic_invocation_started(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![events::KernelEventPayload::SideEffectInvocationStarted(
                events::side_effect::InvocationStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    invocation_epoch: 1,
                    claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
                    claim_generation: 1,
                    claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                        .expect("fencing token"),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    ledger_key: ledger.clone(),
                    required: store::RequiredSideEffectState::InvocationPrepared,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic invocation started");
}

fn append_synthetic_submission_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xd7);
    let digest = content(0xd8);
    let evidence = side_effect_evidence(
        node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Submission,
    );
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![events::KernelEventPayload::SideEffectSubmissionObserved(
                events::side_effect::SubmissionObserved {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    invocation_epoch: 1,
                    submission_schema_id: node.config_ref.schema_id.clone(),
                    submission_hash: digest,
                    submission_artifact_id: artifact_id,
                },
            )],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    ledger_key: ledger.clone(),
                    required: store::RequiredSideEffectState::InvocationStarted,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic submission observed");
}

fn append_synthetic_receipt_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xd9);
    let digest = content(0xda);
    let evidence = side_effect_evidence(
        node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Receipt,
    );
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![events::KernelEventPayload::SideEffectReceiptObserved(
                events::side_effect::ReceiptObserved {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    invocation_epoch: 1,
                    receipt_schema_id: node.config_ref.schema_id.clone(),
                    receipt_hash: digest,
                    receipt_artifact_id: artifact_id,
                    replay_verifier_id: events::ReplayVerifierId::new("mfm.test.driver.replay")
                        .expect("replay verifier"),
                    resource_touched_set: None,
                },
            )],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    ledger_key: ledger.clone(),
                    required: store::RequiredSideEffectState::SubmissionResult,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic receipt observed");
}

fn synthetic_resource_lane_release(
    store: &TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    reason: &str,
) -> Option<events::KernelEventPayload> {
    let holder = store::SideEffectLedgerRef::new(run_id.clone(), ledger.clone());
    let snapshot = store.projection_snapshot();
    let (_, lane) = snapshot
        .resource_lanes()
        .find(|(_, projection)| projection.holder == holder)?;
    Some(events::KernelEventPayload::ResourceLaneReleaseIntent(
        events::ResourceLaneReleaseIntent {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            ledger_key: ledger.clone(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            invocation_epoch: lane.invocation_epoch,
            claim_id: lane.claim_id.clone(),
            release_reason: events::ResourceLaneReleaseReason::new(reason).expect("release reason"),
        },
    ))
}

fn append_synthetic_confirmation_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xdb);
    let digest = content(0xdc);
    let evidence = side_effect_evidence(
        node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Confirmation,
    );
    let release = synthetic_resource_lane_release(
        store,
        fixture,
        &fixture.run_id,
        node,
        attempt_id,
        ledger,
        "side_effect.confirmed",
    );
    let mut payloads = Vec::new();
    if let Some(release) = release {
        payloads.push(release);
    }
    payloads.push(events::KernelEventPayload::SideEffectConfirmationObserved(
        events::side_effect::ConfirmationObserved {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            ledger_key: ledger.clone(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            invocation_epoch: 1,
            confirmation_schema_id: node.config_ref.schema_id.clone(),
            confirmation_hash: digest,
            confirmation_artifact_id: artifact_id,
            replay_verifier_id: events::ReplayVerifierId::new("mfm.test.driver.replay")
                .expect("replay verifier"),
            resource_touched_set: None,
        },
    ));
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: payloads,
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    ledger_key: ledger.clone(),
                    required: store::RequiredSideEffectState::ReceiptObserved,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic confirmation observed");
}

fn side_effect_evidence(
    node: &spec::NodeSpec,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn append_synthetic_ambiguous(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let evidence_hash = content(0xd5);
    let evidence_artifact_id = artifact(0xd6);
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: evidence_artifact_id.clone(),
        digest: evidence_hash.clone(),
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::AmbiguityEvidence,
    };
    let release = synthetic_resource_lane_release(
        store,
        fixture,
        run_id,
        node,
        attempt_id,
        ledger,
        "side_effect.ambiguous",
    );
    let mut payloads = Vec::new();
    if let Some(release) = release {
        payloads.push(release);
    }
    payloads.push(events::KernelEventPayload::SideEffectAmbiguous(
        events::side_effect::Ambiguous {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            ledger_key: ledger.clone(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            invocation_epoch: 1,
            ambiguity_code: events::AmbiguityCode::new("unknown").expect("ambiguity"),
            evidence_schema_id: node.config_ref.schema_id.clone(),
            evidence_hash,
            evidence_artifact_id,
        },
    ));
    payloads.push(events::KernelEventPayload::StateAttemptFailed(
        events::StateAttemptFailed {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            retryable: false,
            error: side_effect_error(false),
        },
    ));
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: payloads,
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    ledger_key: ledger.clone(),
                    required: store::RequiredSideEffectState::InvocationStarted,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic ambiguity");
}

async fn append_manual_resolution(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    outcome: events::ManualResolutionOutcome,
) {
    let manual = match &fixture.runtime_spec.spec().saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual,
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => manual,
        _ => panic!("fixture does not carry manual resolution schemas"),
    };
    let evidence_bytes = br#"{"operator_note":"reviewed"}"#.to_vec();
    let evidence_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&evidence_bytes),
    );
    let evidence_artifact_id =
        ArtifactId::from_digest(evidence_hash.algorithm(), *evidence_hash.digest());
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        content_hash: evidence_hash,
        artifact_id: evidence_artifact_id,
    };
    let prefix = build_manual_resolution_prefix_authority_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        store,
        manual.clone(),
    )
    .expect("manual prefix authority");
    let claim = prefix
        .authorization_claim(outcome, evidence)
        .expect("manual claim");
    let operator = manual.authorization.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("claim digest");
    let proof = ManualResolutionAuthorizationProof {
        verifier_id: manual.authorization.verifier_id.clone(),
        signing_scheme: manual.authorization.signing_scheme.clone(),
        claim: claim.clone(),
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("signature"),
        }],
    };
    let proof_bytes = proof
        .canonical_json()
        .expect("canonical manual proof")
        .to_vec();
    record_manual_resolution(
        scheduler,
        store,
        &fixture.runtime_spec,
        &fixture.run_id,
        ManualResolutionRequest {
            outcome,
            evidence_artifact: ManualResolutionEvidenceArtifact {
                bytes: evidence_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            proof_bytes,
            note: None,
        },
    )
    .await
    .expect("append manual resolution");
}

fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &k256::ecdsa::SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}

fn append_fact(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    fact_key: events::FactKey,
    artifact_id: ArtifactId,
    response_hash: ContentDigest,
) {
    let response_schema_id = node.config_ref.schema_id.clone();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: response_hash.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(response_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-fact:{}:{}:{}",
                node.node_id, attempt_id, fact_key
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::FactRecorded(
                events::FactRecorded {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    capability_kind: fixture.cap_kind.clone(),
                    capability_version: fixture.cap_version.clone(),
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                    request_schema_id: node.config_ref.schema_id.clone(),
                    request_hash: content(0xd4),
                    response_schema_id,
                    response_hash,
                    fact_key,
                    artifact_id,
                },
            )],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append fact");
}

fn append_terminal(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    artifact_id: ArtifactId,
    output_digest: ContentDigest,
) {
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let evidence =
        state_output_artifact(node, descriptor, artifact_id.clone(), output_digest.clone());
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-terminal:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    cell_id: node.output_cell.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: descriptor.output_semantic_type_id.clone(),
                    schema_id: descriptor.output_schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    artifact_id,
                    content_digest: output_digest,
                    producer_state_kind: Some(node.state_kind.clone()),
                    producer_state_version: Some(node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    output_cell_id: node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append terminal");
}

fn append_public_output_render_failure(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        panic!("expected public-output render node");
    };
    let error = public_output_error();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-public-output-failure:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::PublicOutputRenderFailed(
                    events::PublicOutputRenderFailed {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        public_schema_id: render.public_schema_id.clone(),
                        renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
                        error: error.clone(),
                    },
                ),
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable: true,
                    error,
                }),
            ],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append public output failure");
}

fn append_not_submitted_proven(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
) {
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(&projection_snapshot, node, attempt_id)
        .expect("side-effect projection lookup")
        .expect("side-effect projection");
    let ledger_key = projection.ledger_key.clone();
    let ledger_purpose = projection.ledger_purpose.clone();
    let proof_artifact = artifact(0xd5);
    let proof_hash = content(0xd6);
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: proof_artifact.clone(),
        digest: proof_hash.clone(),
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::NotSubmittedProof,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-not-submitted:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::SideEffectNotSubmittedProven(
                events::side_effect::NotSubmittedProven {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key,
                    ledger_purpose,
                    invocation_epoch,
                    proof_schema_id: node.config_ref.schema_id.clone(),
                    proof_hash,
                    proof_artifact_id: proof_artifact,
                },
            )],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append not-submitted proof");
}

fn attempt_started_count(store: &TestTypedRunStore, run_id: &RunId, node_id: &NodeId) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if &payload.node_id == node_id
            )
        })
        .count()
}

fn runtime_lifecycle_summary(store: &TestTypedRunStore, run_id: &RunId) -> String {
    let projections = store.projection_snapshot();
    let mut started = 0;
    let mut completed = 0;
    let mut failed = 0;
    let mut interrupted = 0;
    for (_, attempt) in projections
        .attempts()
        .filter(|(_, attempt)| &attempt.run_id == run_id)
    {
        match attempt.status {
            store::AttemptStatus::Started { .. } => started += 1,
            store::AttemptStatus::Completed { .. } => completed += 1,
            store::AttemptStatus::Failed { .. } => failed += 1,
            store::AttemptStatus::Interrupted => interrupted += 1,
        }
    }
    let total = started + completed + failed + interrupted;
    let run_attempts = projections
        .attempts()
        .filter(|(_, attempt)| &attempt.run_id == run_id)
        .map(|((node_id, attempt_id), _)| (node_id.clone(), attempt_id.clone()))
        .collect::<BTreeSet<_>>();
    let cells = projections
        .cells()
        .filter(|(_, terminal)| match terminal {
            store::CellTerminalProjection::Produced {
                node_id,
                attempt_id,
                ..
            }
            | store::CellTerminalProjection::Skipped {
                node_id,
                attempt_id,
                ..
            } => run_attempts.contains(&(node_id.clone(), attempt_id.clone())),
        })
        .count();
    let side_effects = projections
        .side_effects()
        .filter(|(_, side_effect)| &side_effect.run_id == run_id)
        .count();
    let lanes_total = projections.resource_lanes().count();
    let lanes = projections
        .resource_lanes()
        .filter(|(_, lane)| &lane.holder.run_id == run_id)
        .count();
    let public_outputs = projections.public_outputs().count();
    let retentions = projections
        .retentions()
        .filter(|(retention_run_id, _)| *retention_run_id == run_id)
        .count();
    format!(
        "run={:?} attempts[started={started} completed={completed} failed={failed} interrupted={interrupted} total={total}] cells={cells} side_effects={side_effects} lanes[run={lanes} total={lanes_total}] public_outputs={public_outputs} retentions={retentions}",
        projections.run_state(run_id)
    )
}

fn assert_node_failed_with_code(
    store: &TestTypedRunStore,
    node_id: &NodeId,
    code: &str,
) -> AttemptId {
    assert_node_failed_with_code_and_retryable(store, node_id, code, false)
}

fn assert_node_failed_with_code_and_retryable(
    store: &TestTypedRunStore,
    node_id: &NodeId,
    code: &str,
    expected_retryable: bool,
) -> AttemptId {
    let failures = store
        .projection_snapshot()
        .attempts()
        .filter_map(|((attempt_node_id, attempt_id), attempt)| {
            if attempt_node_id != node_id {
                return None;
            }
            let store::AttemptStatus::Failed { retryable, error } = &attempt.status else {
                return None;
            };
            Some((attempt_id.clone(), *retryable, error.as_ref().clone()))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        failures.len(),
        1,
        "expected one failed attempt for node {node_id}"
    );
    let (attempt_id, retryable, error) = failures.into_iter().next().expect("failure");
    assert_eq!(
        retryable, expected_retryable,
        "failure-safe retryability for {code}"
    );
    assert_eq!(error.code.as_str(), code);
    assert_eq!(error.retryable, retryable);
    let diagnostic = error
        .diagnostic_ref
        .as_ref()
        .expect("failure-safe terminalization records redacted diagnostic evidence");
    assert_eq!(diagnostic.role, events::ArtifactRole::RedactedDiagnostic);
    assert_eq!(diagnostic.semantic_type_id, None);
    assert_eq!(
        diagnostic.media_type,
        spec::MediaType::new("application/json").expect("media type")
    );
    assert!(
        diagnostic
            .schema_id
            .as_str()
            .contains("schema:mfm.runtime.redacted_attempt_failure_diagnostic:1:sha256-jcs-v1:"),
        "diagnostic schema id should identify the runtime redacted failure diagnostic schema"
    );
    let retained = store
        .projection_snapshot()
        .retentions()
        .any(|(_, retention)| {
            retention
                .refs
                .get(&diagnostic.artifact_id)
                .is_some_and(|retention_ref| {
                    retention_ref.role == events::ArtifactRole::RedactedDiagnostic
                        && retention_ref.content_digest == diagnostic.content_digest
                })
        });
    assert!(
        retained,
        "failure-safe diagnostic artifact should be retained as runtime evidence"
    );
    attempt_id
}

fn assert_failure_code_count(store: &TestTypedRunStore, code: &str, expected: usize) {
    let count = store
        .projection_snapshot()
        .attempts()
        .filter(|(_, attempt)| {
            matches!(
                &attempt.status,
                store::AttemptStatus::Failed { error, .. } if error.code.as_str() == code
            )
        })
        .count();
    assert_eq!(count, expected, "failure code count for {code}");
}

fn fact_recorded_count(store: &TestTypedRunStore) -> usize {
    store
        .projection_snapshot()
        .facts()
        .filter(|(_, fact)| fact.fact_key.as_str() == "reused-fact")
        .count()
}

#[test]
fn scheduler_does_not_match_open_attempt_disposition() {
    let scheduler_source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/scheduler.rs"))
            .expect("scheduler source");
    assert!(
        !scheduler_source.contains("OpenAttemptDisposition"),
        "scheduler facade must not match recovery dispositions directly"
    );
    let transition_source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/transition.rs"))
            .expect("transition source");
    assert!(
        !transition_source.contains("TransitionDecision::PublicOutputProjected")
            && !transition_source.contains("PublicOutputProjected,"),
        "public-output projected is a scheduler status, not a transition decision variant"
    );
}

#[test]
fn runtime_order_is_deterministic_for_reordered_spec_nodes() {
    let fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    envelope.spec.nodes.reverse();
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    let runtime = CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime");
    assert_eq!(
        runtime.topological_order(),
        fixture.runtime_spec.topological_order()
    );
}

fn registered_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    registry
}

fn registered_side_effect_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    registry
}

fn registered_first_side_effect_runners_with<R: ErasedNodeRunner + 'static>(
    fixture: &Fixture,
    runner: R,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(fixture.descriptor_a.clone(), "sidefx", runner))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    registry
}

fn compensated_saga_scheduler(fixture: &Fixture) -> SerialTypedScheduler {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding forward a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "sidefx",
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding forward b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "fail",
            BlockingRunner,
        ))
        .expect("binding failure node");
    test_scheduler(registry)
}

fn binding<R: ErasedNodeRunner + 'static>(
    descriptor_id: DescriptorId,
    factory: &str,
    runner: R,
) -> ErasedRunnerBinding {
    let factory_id = events::RunnerFactoryId::new(factory).expect("factory");
    ErasedRunnerBinding::new(
        descriptor_id,
        factory_id.clone(),
        events::ExecutableIdentity {
            factory_id,
            cargo_package_digest: content(0xe1),
            binary_digest: content(0xe2),
            nix_derivation_hash: None,
            nix_output_hash: None,
        },
        Arc::new(runner),
    )
    .expect("runner binding")
}

fn runtime_retention_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
    typed
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        })
        .expect("retention node")
        .output_cell
        .clone()
}

fn append_runtime_retention_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
    public_output_receipt_cell: CellId,
    valid_ordering: bool,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf1; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf2; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf3; 32]),
    );
    let config_ref = spec::framework_config_ref("project_retention_manifest", &node_id)
        .expect("retention config ref");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == public_output_receipt_cell)
        .expect("input cell")
        .clone();
    let input_binding = spec::framework_lifecycle_receipt_input_binding(
        "project_retention_manifest",
        "public_output_receipt",
        &input_cell,
    )
    .expect("input binding");
    let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
    let receipt_schema =
        spec::retention_manifest_receipt_schema_id().expect("retention receipt schema");
    let receipt_semantic =
        spec::retention_manifest_receipt_semantic_type_id().expect("retention receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "project_retention_manifest",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf9; 32]),
    )
    .expect("state kind");
    let state_version = StateVersion::new("mfm.framework.state.project_retention_manifest.v1")
        .expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.project_retention_manifest".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed.kind.clone(),
                effect_class: managed.class.as_str().to_owned(),
                effect_name: managed.name.to_owned(),
                effect_version: managed.version,
                capabilities: no_caps.clone(),
                runner: "managed_platform_write".to_owned(),
                side_effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: input_cell.scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xfa),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
    let predecessors = match &input_cell.producer {
        spec::CellProducer::Node(producer) => vec![producer.clone()],
        spec::CellProducer::Seed(_) => Vec::new(),
    };
    let framework_receipt = if valid_ordering {
        public_output_receipt_cell
    } else {
        input_cell.cell_id.clone()
    };
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/project-retention-manifest")
            .expect("stable key"),
        scope_id: input_cell.scope_id,
        state_kind,
        state_version,
        descriptor_id,
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: managed.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(
            spec::ProjectRetentionManifestNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
                public_output_receipt_cell: framework_receipt,
            },
        )),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: predecessors,
    });
    node_id
}

fn append_runtime_complete_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
    retention_manifest_receipt_cell: CellId,
    valid_ordering: bool,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb2; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb3; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb4; 32]),
    );
    let config_ref =
        spec::framework_config_ref("complete_run", &node_id).expect("complete config ref");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == retention_manifest_receipt_cell)
        .expect("input cell")
        .clone();
    let input_binding = spec::framework_lifecycle_receipt_input_binding(
        "complete_run",
        "retention_manifest_receipt",
        &input_cell,
    )
    .expect("input binding");
    let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
    let receipt_schema = spec::complete_run_receipt_schema_id().expect("complete receipt schema");
    let receipt_semantic =
        spec::complete_run_receipt_semantic_type_id().expect("complete receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "complete_run",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb8; 32]),
    )
    .expect("state kind");
    let state_version =
        StateVersion::new("mfm.framework.state.complete_run.v1").expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.complete_run".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed.kind.clone(),
                effect_class: managed.class.as_str().to_owned(),
                effect_name: managed.name.to_owned(),
                effect_version: managed.version,
                capabilities: no_caps.clone(),
                runner: "managed_platform_write".to_owned(),
                side_effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: input_cell.scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xbb),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
    let predecessors = match &input_cell.producer {
        spec::CellProducer::Node(producer) => vec![producer.clone()],
        spec::CellProducer::Seed(_) => Vec::new(),
    };
    let framework_receipt = if valid_ordering {
        retention_manifest_receipt_cell
    } else {
        typed
            .public_outputs
            .outputs
            .first()
            .expect("public output")
            .cell_id
            .clone()
    };
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/complete-run").expect("stable key"),
        scope_id: input_cell.scope_id,
        state_kind,
        state_version,
        descriptor_id,
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: managed.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::CompleteRun(
            spec::CompleteRunNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
                retention_manifest_receipt_cell: framework_receipt,
            },
        )),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: predecessors,
    });
    node_id
}

fn append_runtime_resolve_saga_terminal_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc2; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc3; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc4; 32]),
    );
    let config_ref =
        spec::framework_config_ref("resolve_saga_terminal", &node_id).expect("resolve config ref");
    let input_binding = spec::framework_lifecycle_unit_input_binding("resolve_saga_terminal")
        .expect("input binding");
    let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
    let receipt_schema =
        spec::resolve_saga_terminal_receipt_schema_id().expect("resolve receipt schema");
    let receipt_semantic =
        spec::resolve_saga_terminal_receipt_semantic_type_id().expect("resolve receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "resolve_saga_terminal",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc8; 32]),
    )
    .expect("state kind");
    let state_version =
        StateVersion::new("mfm.framework.state.resolve_saga_terminal.v1").expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.resolve_saga_terminal".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed.kind.clone(),
                effect_class: managed.class.as_str().to_owned(),
                effect_name: managed.name.to_owned(),
                effect_version: managed.version,
                capabilities: no_caps.clone(),
                runner: "managed_platform_write".to_owned(),
                side_effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    let scope_id = typed.scopes.first().expect("root scope").scope_id.clone();
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xcb),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/resolve-saga-terminal")
            .expect("stable key"),
        scope_id,
        state_kind,
        state_version,
        descriptor_id,
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: managed.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(
            spec::ResolveSagaTerminalNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
            },
        )),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    });
    node_id
}

fn runtime_predecessors_for_inputs(
    typed: &spec::TypedExecutionSpec,
    input_cells: &[CellId],
) -> Vec<NodeId> {
    let mut predecessors = BTreeSet::new();
    for input_cell in input_cells {
        let cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == *input_cell)
            .expect("input cell");
        if let spec::CellProducer::Node(node_id) = &cell.producer {
            predecessors.insert(node_id.clone());
        }
    }
    predecessors.into_iter().collect()
}

fn fixture() -> Fixture {
    let scope = ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, D0);
    let seed_id = SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, D1);
    let seed_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2);
    let node_a = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D3);
    let cell_a = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D4);
    let node_b = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D5);
    let cell_b = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D6);
    let descriptor_a = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D7);
    let descriptor_b = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
    let semantic = fixture_value_semantic_id();
    let value_schema = fixture_value_schema_id();
    let input_schema = SchemaId::new("mfm.test.input", "1", DigestAlgorithm::Sha256JcsV1, DB)
        .expect("input schema");
    let config_schema = SchemaId::new("mfm.test.config", "1", DigestAlgorithm::Sha256JcsV1, DC)
        .expect("config schema");
    let public_schema = SchemaId::new("mfm.test.public", "1", DigestAlgorithm::Sha256JcsV1, DD)
        .expect("public schema");
    let effect_kind =
        EffectKind::new("mfm.test", "pure", DigestAlgorithm::Sha256JcsV1, DE).expect("effect");
    let read_effect =
        EffectKind::new("mfm.test", "read", DigestAlgorithm::Sha256JcsV1, DF).expect("read effect");
    let cap_kind = CapabilityKind::new("mfm.test", "read-db", DigestAlgorithm::Sha256JcsV1, D0)
        .expect("cap kind");
    let cap_version = CapabilityVersion::new("mfm.cap.read_db.v1").expect("cap version");
    let adapter_kind = AdapterKind::new("mfm.test", "adapter", DigestAlgorithm::Sha256JcsV1, D1)
        .expect("adapter kind");
    let adapter_version = AdapterVersion::new("mfm.adapter.v1").expect("adapter version");
    let read_cap = CapabilityDescriptor::new(
        cap_kind.clone(),
        cap_version.clone(),
        CapabilityRole::ReadExternal,
        "read-db",
    )
    .expect("capability");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let read_caps = CapabilitySetDescriptor::new(vec![read_cap]).expect("read caps");
    let config_digest = digest_for_bytes(TEST_CONFIG_BYTES);
    let config_ref = spec::ConfigRef {
        schema_id: config_schema.clone(),
        artifact_id: ArtifactId::from_digest(config_digest.algorithm(), *config_digest.digest()),
        digest: config_digest,
        byte_len: TEST_CONFIG_BYTES.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
    };
    let lineage_seed = spec::ValueLineageRef {
        lineage_digest: content(0x41),
    };
    let lineage_a = spec::ValueLineageRef {
        lineage_digest: content(0x42),
    };
    let lineage_b = spec::ValueLineageRef {
        lineage_digest: content(0x43),
    };
    let planning = spec::PlanningLineage {
        active_operation_instances: Vec::new(),
        completed_operation_frames: Vec::new(),
        lineage_digest: content(0x44),
    };
    let seed_digest = digest_for_bytes(TEST_SEED_BYTES);
    let seed_ref = events::SeedCellRef {
        seed_id: seed_id.clone(),
        cell_id: seed_cell.clone(),
        scope_id: scope.clone(),
        semantic_type_id: semantic.clone(),
        schema_id: value_schema.clone(),
        digest: seed_digest.clone(),
        seed_artifact: events::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(seed_digest.algorithm(), *seed_digest.digest()),
            role: events::ArtifactRole::SeedInput,
            schema_id: value_schema.clone(),
            semantic_type_id: Some(semantic.clone()),
            content_digest: seed_digest.clone(),
            byte_len: TEST_SEED_BYTES.len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media"),
        },
    };
    let renderer = spec::RendererDescriptorIdentity {
        descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D1),
        renderer_kind: spec::RendererKind::new("public-output/json").expect("renderer"),
        renderer_version: spec::RendererVersion::new("mfm.renderer.test.v1")
            .expect("renderer version"),
        public_schema_id: public_schema.clone(),
        canonicalizer_identity: spec::CanonicalizerIdentity::new("sha256-jcs-v1")
            .expect("canonicalizer"),
    };
    let public_output_cell = spec::PublicOutputCell {
        public_field_path: spec::PublicFieldPath::new("result").expect("field"),
        cell_id: cell_b.clone(),
        producer: spec::CellProducer::Node(node_b.clone()),
        scope_id: scope.clone(),
        semantic_type_id: semantic.clone(),
        schema_id: value_schema.clone(),
        value_lineage: lineage_b.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
    };
    let public_outputs = spec::PublicOutputSpec {
        public_schema_id: public_schema.clone(),
        outputs: vec![public_output_cell.clone()],
        renderer_descriptor: renderer.clone(),
    };
    let output_spec_digest = public_outputs.digest().expect("public output digest");
    let render_node = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
    let render_config_ref = spec::framework_config_ref("public_output_render", &render_node)
        .expect("render config ref");
    let render_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
    let render_descriptor = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, DA);
    let receipt_schema = spec::public_output_receipt_schema_id().expect("receipt schema");
    let receipt_semantic =
        spec::public_output_receipt_semantic_type_id().expect("receipt semantic");
    let render_lineage = spec::ValueLineageRef {
        lineage_digest: content(0x45),
    };
    let render_input_root = spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
        field_path: public_output_cell.public_field_path.clone(),
        node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: public_output_cell.public_field_path.clone(),
            cell_id: public_output_cell.cell_id.clone(),
            semantic_type_id: public_output_cell.semantic_type_id.clone(),
            schema_id: public_output_cell.schema_id.clone(),
            required_terminal: public_output_cell.required_terminal,
            value_lineage: public_output_cell.value_lineage.clone(),
        })),
    }]);
    let render_input_binding = spec::InputBindingSpec {
        input_schema_id: public_schema.clone(),
        input_descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, DC),
        digest: content_digest_json(input_node_json(&render_input_root))
            .expect("render input digest"),
        root: render_input_root,
    };
    let managed_effect = ManagedPlatformWrite::descriptor().expect("managed effect");
    let render_state_kind = StateKind::new(
        "mfm.framework.state",
        "render_public_outputs",
        DigestAlgorithm::Sha256JcsV1,
        DD,
    )
    .expect("render state kind");
    let render_state_version = StateVersion::new("mfm.framework.state.render_public_outputs.v1")
        .expect("render state version");
    let node_a_spec = node_spec(NodeSpecFixture {
        node_id: node_a.clone(),
        descriptor_id: descriptor_a.clone(),
        scope_id: scope.clone(),
        state_name: "mfm.test.state.a",
        state_kind: StateKind::new("mfm.test", "a", DigestAlgorithm::Sha256JcsV1, D2)
            .expect("state a"),
        state_version: StateVersion::new("mfm.test.state.a.v1").expect("state version"),
        effect_kind: effect_kind.clone(),
        config_ref: config_ref.clone(),
        input_schema: input_schema.clone(),
        input_cell: seed_cell.clone(),
        input_lineage: lineage_seed.clone(),
        output_cell: cell_a.clone(),
        output_schema: value_schema.clone(),
        semantic: semantic.clone(),
        caps: no_caps.clone(),
        predecessors: Vec::new(),
        adapter_bindings: Vec::new(),
        planning: planning.clone(),
    });
    let node_b_spec = node_spec(NodeSpecFixture {
        node_id: node_b.clone(),
        descriptor_id: descriptor_b.clone(),
        scope_id: scope.clone(),
        state_name: "mfm.test.state.b",
        state_kind: StateKind::new("mfm.test", "b", DigestAlgorithm::Sha256JcsV1, D3)
            .expect("state b"),
        state_version: StateVersion::new("mfm.test.state.b.v1").expect("state version"),
        effect_kind: read_effect.clone(),
        config_ref: config_ref.clone(),
        input_schema: input_schema.clone(),
        input_cell: cell_a.clone(),
        input_lineage: lineage_a.clone(),
        output_cell: cell_b.clone(),
        output_schema: value_schema.clone(),
        semantic: semantic.clone(),
        caps: read_caps.clone(),
        predecessors: vec![node_a.clone()],
        adapter_bindings: vec![spec::AdapterBinding {
            adapter_kind: adapter_kind.clone(),
            adapter_version: adapter_version.clone(),
            binding_digest: None,
        }],
        planning: planning.clone(),
    });
    let render_node_spec = spec::NodeSpec {
        node_id: render_node.clone(),
        stable_key: spec::StableAuthorKey::new("public-output").expect("render key"),
        scope_id: scope.clone(),
        state_kind: render_state_kind.clone(),
        state_version: render_state_version.clone(),
        descriptor_id: render_descriptor.clone(),
        config_ref: render_config_ref.clone(),
        input_bindings: render_input_binding,
        output_cell: render_cell.clone(),
        effect_kind: managed_effect.kind.clone(),
        capability_bindings: no_caps.clone(),
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
            spec::PublicOutputRenderNodeSpec {
                public_schema_id: public_schema.clone(),
                output_spec_digest: output_spec_digest.clone(),
                renderer_descriptor: renderer.clone(),
                required_cells: public_outputs.outputs.clone(),
            },
        )),
        planning_lineage: planning.clone(),
        deterministic_predecessors: vec![node_b.clone()],
    };
    let mut spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
        authoring: spec::AuthoringProvenance::StateComposition {
            descriptor: spec::CompositionDescriptor {
                descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D4),
                name: "mfm.test.composition".to_owned(),
                version: "mfm.test.composition.v1".to_owned(),
            },
            config_hash: content(0x60),
        },
        saga: spec::SagaPolicySpec::NoSideEffects,
        scopes: vec![spec::ScopeSpec {
            scope_id: scope.clone(),
            parent_scope_id: None,
            stable_key: spec::StableAuthorKey::new("root").expect("stable key"),
            planning_lineage: planning.clone(),
        }],
        seeds: vec![spec::SeedSpec {
            seed_id: seed_id.clone(),
            seed_key: spec::StableAuthorKey::new("launch").expect("seed key"),
            cell_id: seed_cell.clone(),
            scope_id: scope.clone(),
            semantic_type_id: semantic.clone(),
            schema_id: value_schema.clone(),
            required_digest: Some(seed_digest),
        }],
        descriptor_identities: vec![
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &node_a_spec,
                descriptor_a.clone(),
                "mfm.test.state.a",
                effect_kind,
                no_caps.clone(),
                "pure",
            ))),
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &node_b_spec,
                descriptor_b.clone(),
                "mfm.test.state.b",
                read_effect,
                read_caps,
                "read",
            ))),
            spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
                descriptor_id: render_descriptor.clone(),
                name: "mfm.framework.render_public_outputs".to_owned(),
                state_kind: render_state_kind,
                state_version: render_state_version,
                config_schema_id: render_config_ref.schema_id.clone(),
                input_schema_id: public_schema.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed_effect.kind,
                effect_class: managed_effect.class.as_str().to_owned(),
                effect_name: managed_effect.name.to_owned(),
                effect_version: managed_effect.version,
                capabilities: no_caps,
                runner: "managed_platform_write".to_owned(),
                side_effect_contract_digest: None,
            })),
            spec::DescriptorIdentity::Renderer(Box::new(renderer.clone())),
        ],
        config_refs: vec![config_ref.clone(), render_config_ref],
        nodes: vec![
            render_node_spec.clone(),
            node_b_spec.clone(),
            node_a_spec.clone(),
        ],
        remediations: BTreeMap::new(),
        cells: vec![
            spec::CellSpec {
                cell_id: seed_cell,
                producer: spec::CellProducer::Seed(seed_id),
                scope_id: scope.clone(),
                semantic_type_id: semantic.clone(),
                schema_id: value_schema.clone(),
                value_lineage: lineage_seed.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            },
            spec::CellSpec {
                cell_id: cell_a.clone(),
                producer: spec::CellProducer::Node(node_a.clone()),
                scope_id: scope.clone(),
                semantic_type_id: semantic.clone(),
                schema_id: value_schema.clone(),
                value_lineage: lineage_a.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            },
            spec::CellSpec {
                cell_id: cell_b.clone(),
                producer: spec::CellProducer::Node(node_b.clone()),
                scope_id: scope.clone(),
                semantic_type_id: semantic.clone(),
                schema_id: value_schema.clone(),
                value_lineage: lineage_b.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            },
            spec::CellSpec {
                cell_id: render_cell.clone(),
                producer: spec::CellProducer::Node(render_node.clone()),
                scope_id: scope.clone(),
                semantic_type_id: receipt_semantic,
                schema_id: receipt_schema,
                value_lineage: render_lineage.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::PublicOutputArtifact,
                redaction_policy: spec::RedactionPolicy::Public,
            },
        ],
        value_lineages: vec![
            spec::ValueLineage {
                lineage_ref: lineage_seed.clone(),
                scope_id: scope.clone(),
                producer: spec::CellProducer::Seed(SeedId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    D1,
                )),
                input_cells: Vec::new(),
                config_ref_digest: None,
                planning_lineage: planning.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::Source,
            },
            spec::ValueLineage {
                lineage_ref: lineage_a.clone(),
                scope_id: scope.clone(),
                producer: spec::CellProducer::Node(node_a.clone()),
                input_cells: vec![CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2)],
                config_ref_digest: Some(config_ref.digest.clone()),
                planning_lineage: planning.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            },
            spec::ValueLineage {
                lineage_ref: lineage_b.clone(),
                scope_id: scope.clone(),
                producer: spec::CellProducer::Node(node_b.clone()),
                input_cells: vec![cell_a.clone()],
                config_ref_digest: Some(config_ref.digest.clone()),
                planning_lineage: planning.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            },
            spec::ValueLineage {
                lineage_ref: render_lineage,
                scope_id: scope,
                producer: spec::CellProducer::Node(render_node.clone()),
                input_cells: vec![cell_b.clone()],
                config_ref_digest: Some(config_ref.digest.clone()),
                planning_lineage: planning,
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            },
        ],
        planning_lineage: Vec::new(),
        public_outputs,
    })
    .expect("typed spec");
    append_runtime_retention_lifecycle_node(&mut spec, render_cell.clone(), true);
    let retention_receipt = runtime_retention_receipt_cell(&spec);
    append_runtime_complete_lifecycle_node(&mut spec, retention_receipt, true);
    append_runtime_resolve_saga_terminal_lifecycle_node(&mut spec);
    let envelope = spec::HashedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
        .expect("envelope");
    let runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    let identity_material = run_identity_material(&runtime_spec);
    let run_id = identity_material.derive_run_id().expect("run id");
    Fixture {
        runtime_spec,
        run_id,
        distinct_run_key_digest: None,
        seed_ref,
        descriptor_a,
        descriptor_b,
        descriptor_c: None,
        render_node,
        render_cell,
        cell_a,
        cell_b,
        cell_c: None,
        cap_kind,
        cap_version,
        adapter_kind,
        adapter_version,
    }
}

fn fixture_with_retention_lifecycle_node() -> Fixture {
    fixture()
}

async fn drive_until_public_output_produced(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) {
    for _ in 0..8 {
        drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive until public output");
        let projections = store.projection_snapshot();
        if projections.run_state(&fixture.run_id) == store::RunState::Started
            && matches!(
                projections
                    .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id),
                Some(store::PublicOutputProjection::Produced { .. })
            )
        {
            return;
        }
    }
    panic!("public output was not produced");
}

fn fixture_with_first_managed_write_state() -> Fixture {
    let mut fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let managed_effect = EffectKind::new(
        "mfm.test",
        "managed-write",
        DigestAlgorithm::Sha256JcsV1,
        D8,
    )
    .expect("managed effect");
    let managed_cap = CapabilityDescriptor::new(
        CapabilityKind::new(
            "mfm.test",
            "managed-store",
            DigestAlgorithm::Sha256JcsV1,
            D9,
        )
        .expect("managed cap kind"),
        CapabilityVersion::new("mfm.cap.managed_store.v1").expect("managed cap version"),
        CapabilityRole::ManagedPlatformWrite,
        "managed-store",
    )
    .expect("managed cap");
    let managed_caps = CapabilitySetDescriptor::new(vec![managed_cap]).expect("managed caps");
    for node in &mut envelope.spec.nodes {
        if node.descriptor_id == fixture.descriptor_a {
            node.effect_kind = managed_effect.clone();
            node.capability_bindings = managed_caps.clone();
        }
    }
    for descriptor in &mut envelope.spec.descriptor_identities {
        if let spec::DescriptorIdentity::State(identity) = descriptor {
            if identity.descriptor_id == fixture.descriptor_a {
                identity.effect_kind = managed_effect.clone();
                identity.effect_class = "managed-write".to_owned();
                identity.effect_name = "managed-write".to_owned();
                identity.capabilities = managed_caps.clone();
                identity.runner = "managed-write".to_owned();
            }
        }
    }
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn fixture_with_first_side_effect_state() -> Fixture {
    let mut fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let side_effect = EffectKind::new("mfm.test", "side-effect", DigestAlgorithm::Sha256JcsV1, D8)
        .expect("side-effect");
    let side_effect_cap = CapabilityDescriptor::new(
        side_effect_capability_kind(),
        side_effect_capability_version(),
        CapabilityRole::ExternalMutationAuthority,
        "external-mutation",
    )
    .expect("side-effect cap");
    let side_effect_caps =
        CapabilitySetDescriptor::new(vec![side_effect_cap]).expect("side-effect caps");
    let contract_digest = content(0x88);
    for node in &mut envelope.spec.nodes {
        if node.descriptor_id == fixture.descriptor_a {
            node.effect_kind = side_effect.clone();
            node.capability_bindings = side_effect_caps.clone();
            node.adapter_bindings = vec![spec::AdapterBinding {
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
                binding_digest: None,
            }];
            node.side_effect = Some(spec::SideEffectContractSpec {
                contract_digest: contract_digest.clone(),
                resource_claim: spec::ResourceClaimSpec::ManualOnly,
            });
        }
    }
    for descriptor in &mut envelope.spec.descriptor_identities {
        if let spec::DescriptorIdentity::State(identity) = descriptor {
            if identity.descriptor_id == fixture.descriptor_a {
                identity.effect_kind = side_effect.clone();
                identity.effect_class = "sidefx".to_owned();
                identity.effect_name = "sidefx".to_owned();
                identity.capabilities = side_effect_caps.clone();
                identity.runner = "sidefx".to_owned();
                identity.side_effect_contract_digest = Some(contract_digest.clone());
            }
        }
    }
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn fixture_with_first_exclusive_side_effect_state() -> Fixture {
    let fixture = fixture_with_first_side_effect_state();
    let descriptors = vec![fixture.descriptor_a.clone()];
    with_exclusive_resource_claims(fixture, &descriptors)
}

fn fixture_with_first_exact_touched_set_side_effect_state() -> Fixture {
    let fixture = fixture_with_first_side_effect_state();
    let descriptors = vec![fixture.descriptor_a.clone()];
    with_exact_touched_set_resource_claims(fixture, &descriptors)
}

fn fixture_with_manual_resolution_side_effect_state() -> Fixture {
    let mut fixture = fixture_with_first_side_effect_state();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let manual = spec::ManualResolutionEvidenceSpec {
        evidence_schema: fixture.seed_ref.schema_id.clone(),
        authorization: manual_authorization(0xe0),
    };
    envelope.spec.saga = spec::SagaPolicySpec::ManualResolution { manual };
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn manual_authorization(byte: u8) -> spec::ManualResolutionAuthorizationSpec {
    spec::ManualResolutionAuthorizationSpec {
        verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
            "mfm.test.manual.verifier.{byte}"
        ))
        .expect("verifier id"),
        signing_scheme: spec::ManualSigningSchemeSpec::new(
            "mfm.manual_resolution.digest_signature.v1",
        )
        .expect("signing scheme"),
        authority: spec::OperatorAuthoritySnapshotSpec {
            authority_id: spec::OperatorAuthorityId::new(format!(
                "mfm.test.manual.authority.{byte}"
            ))
            .expect("authority id"),
            operators: vec![spec::OperatorAuthorityMemberSpec {
                operator_id: spec::OperatorId::new(format!("operator.{byte}"))
                    .expect("operator id"),
                public_identity: spec::OperatorPublicIdentity::new(
                    "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                )
                .expect("operator public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

fn fixture_with_independent_second_node_and_first_side_effect_state() -> Fixture {
    let mut fixture = fixture_with_first_side_effect_state();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let seed_cell = envelope
        .spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == fixture.seed_ref.cell_id)
        .expect("seed cell")
        .clone();
    let node_b_id = envelope
        .spec
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("node b")
        .node_id
        .clone();
    let cell_a = envelope
        .spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == fixture.cell_a)
        .expect("cell a")
        .clone();
    let cell_a_public_output = spec::PublicOutputCell {
        public_field_path: spec::PublicFieldPath::new("side_effect").expect("field"),
        cell_id: cell_a.cell_id.clone(),
        producer: cell_a.producer.clone(),
        scope_id: cell_a.scope_id.clone(),
        semantic_type_id: cell_a.semantic_type_id.clone(),
        schema_id: cell_a.schema_id.clone(),
        value_lineage: cell_a.value_lineage.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
    };
    envelope
        .spec
        .public_outputs
        .outputs
        .push(cell_a_public_output);
    let public_output_cells = envelope.spec.public_outputs.outputs.clone();
    let output_spec_digest = envelope
        .spec
        .public_outputs
        .digest()
        .expect("public output digest");
    let render_input_root = spec::InputBindingNodeSpec::Struct(
        public_output_cells
            .iter()
            .map(|public_output| spec::NamedInputBindingSpec {
                field_path: public_output.public_field_path.clone(),
                node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                    field_path: public_output.public_field_path.clone(),
                    cell_id: public_output.cell_id.clone(),
                    semantic_type_id: public_output.semantic_type_id.clone(),
                    schema_id: public_output.schema_id.clone(),
                    required_terminal: public_output.required_terminal,
                    value_lineage: public_output.value_lineage.clone(),
                })),
            })
            .collect(),
    );
    let render_predecessors = runtime_predecessors_for_inputs(
        &envelope.spec,
        &public_output_cells
            .iter()
            .map(|public_output| public_output.cell_id.clone())
            .collect::<Vec<_>>(),
    );
    for node in &mut envelope.spec.nodes {
        if node.descriptor_id == fixture.descriptor_b {
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                    field_path: spec::PublicFieldPath::new("input").expect("field"),
                    cell_id: seed_cell.cell_id.clone(),
                    semantic_type_id: seed_cell.semantic_type_id.clone(),
                    schema_id: seed_cell.schema_id.clone(),
                    required_terminal: spec::RequiredTerminal::ProducedOnly,
                    value_lineage: seed_cell.value_lineage.clone(),
                }));
            node.deterministic_predecessors.clear();
        }
        if node.node_id == fixture.render_node {
            if let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &mut node.framework {
                render.output_spec_digest = output_spec_digest.clone();
                render.required_cells = public_output_cells.clone();
            }
            node.input_bindings.root = render_input_root.clone();
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("render input digest");
            node.deterministic_predecessors = render_predecessors.clone();
        }
    }
    for lineage in &mut envelope.spec.value_lineages {
        if lineage.producer == spec::CellProducer::Node(node_b_id.clone()) {
            lineage.input_cells = vec![seed_cell.cell_id.clone()];
        }
        if lineage.producer == spec::CellProducer::Node(fixture.render_node.clone()) {
            lineage.input_cells = public_output_cells
                .iter()
                .map(|public_output| public_output.cell_id.clone())
                .collect();
        }
    }
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn fixture_with_two_side_effects_and_failing_tail() -> Fixture {
    let mut fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let side_effect = EffectKind::new("mfm.test", "side-effect", DigestAlgorithm::Sha256JcsV1, D8)
        .expect("side-effect");
    let side_effect_cap = CapabilityDescriptor::new(
        side_effect_capability_kind(),
        side_effect_capability_version(),
        CapabilityRole::ExternalMutationAuthority,
        "external-mutation",
    )
    .expect("side-effect cap");
    let side_effect_caps =
        CapabilitySetDescriptor::new(vec![side_effect_cap]).expect("side-effect caps");
    let contract_digest = content(0x88);
    for node in &mut envelope.spec.nodes {
        if node.descriptor_id == fixture.descriptor_a || node.descriptor_id == fixture.descriptor_b
        {
            node.effect_kind = side_effect.clone();
            node.capability_bindings = side_effect_caps.clone();
            node.adapter_bindings = vec![spec::AdapterBinding {
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
                binding_digest: None,
            }];
            node.side_effect = Some(spec::SideEffectContractSpec {
                contract_digest: contract_digest.clone(),
                resource_claim: spec::ResourceClaimSpec::ManualOnly,
            });
        }
    }
    for descriptor in &mut envelope.spec.descriptor_identities {
        if let spec::DescriptorIdentity::State(identity) = descriptor {
            if identity.descriptor_id == fixture.descriptor_a
                || identity.descriptor_id == fixture.descriptor_b
            {
                identity.effect_kind = side_effect.clone();
                identity.effect_class = "sidefx".to_owned();
                identity.effect_name = "sidefx".to_owned();
                identity.capabilities = side_effect_caps.clone();
                identity.runner = "sidefx".to_owned();
                identity.side_effect_contract_digest = Some(contract_digest.clone());
            }
        }
    }

    let scope = envelope.spec.scopes[0].scope_id.clone();
    let planning = envelope.spec.scopes[0].planning_lineage.clone();
    let node_b = envelope
        .spec
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("node b")
        .clone();
    let cell_b = envelope
        .spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == fixture.cell_b)
        .expect("cell b")
        .clone();
    let config_ref = node_b.config_ref.clone();
    let node_c = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x70; 32]),
    );
    let cell_c = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x71; 32]),
    );
    let descriptor_c = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x72; 32]),
    );
    let lineage_c = spec::ValueLineageRef {
        lineage_digest: content(0x74),
    };
    let failure_effect = EffectKind::new(
        "mfm.test",
        "nonretryable-failure",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x75; 32]),
    )
    .expect("failure effect");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let node_c_input_root =
        spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: spec::PublicFieldPath::new("input").expect("field"),
            cell_id: cell_b.cell_id.clone(),
            semantic_type_id: cell_b.semantic_type_id.clone(),
            schema_id: cell_b.schema_id.clone(),
            required_terminal: spec::RequiredTerminal::ProducedOnly,
            value_lineage: cell_b.value_lineage.clone(),
        }));
    let node_c_spec = spec::NodeSpec {
        node_id: node_c.clone(),
        stable_key: spec::StableAuthorKey::new("c").expect("stable key"),
        scope_id: scope.clone(),
        state_kind: StateKind::new(
            "mfm.test",
            "c",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x76; 32]),
        )
        .expect("state c"),
        state_version: StateVersion::new("mfm.test.state.c.v1").expect("state version"),
        descriptor_id: descriptor_c.clone(),
        config_ref: config_ref.clone(),
        input_bindings: spec::InputBindingSpec {
            input_schema_id: cell_b.schema_id.clone(),
            input_descriptor_id: DescriptorId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x77; 32]),
            ),
            digest: content_digest_json(input_node_json(&node_c_input_root))
                .expect("node c input digest"),
            root: node_c_input_root,
        },
        output_cell: cell_c.clone(),
        effect_kind: failure_effect.clone(),
        capability_bindings: no_caps.clone(),
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: None,
        planning_lineage: planning.clone(),
        deterministic_predecessors: vec![node_b.node_id.clone()],
    };
    envelope
        .spec
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_c.clone(),
                name: "mfm.test.state.c".to_owned(),
                state_kind: node_c_spec.state_kind.clone(),
                state_version: node_c_spec.state_version.clone(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: node_c_spec.input_bindings.input_schema_id.clone(),
                output_schema_id: cell_b.schema_id.clone(),
                output_semantic_type_id: cell_b.semantic_type_id.clone(),
                effect_kind: failure_effect.clone(),
                effect_class: "fail".to_owned(),
                effect_name: "fail".to_owned(),
                effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
                capabilities: no_caps.clone(),
                runner: "fail".to_owned(),
                side_effect_contract_digest: None,
            },
        )));
    envelope.spec.cells.push(spec::CellSpec {
        cell_id: cell_c.clone(),
        producer: spec::CellProducer::Node(node_c.clone()),
        scope_id: scope.clone(),
        semantic_type_id: cell_b.semantic_type_id.clone(),
        schema_id: cell_b.schema_id.clone(),
        value_lineage: lineage_c.clone(),
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
    envelope.spec.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage_c.clone(),
        scope_id: scope.clone(),
        producer: spec::CellProducer::Node(node_c.clone()),
        input_cells: vec![cell_b.cell_id.clone()],
        config_ref_digest: Some(config_ref.digest.clone()),
        planning_lineage: planning.clone(),
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    envelope.spec.nodes.push(node_c_spec);

    let public_output_cell = spec::PublicOutputCell {
        public_field_path: spec::PublicFieldPath::new("result").expect("field"),
        cell_id: cell_c.clone(),
        producer: spec::CellProducer::Node(node_c.clone()),
        scope_id: scope.clone(),
        semantic_type_id: cell_b.semantic_type_id.clone(),
        schema_id: cell_b.schema_id.clone(),
        value_lineage: lineage_c,
        required_terminal: spec::RequiredTerminal::ProducedOnly,
    };
    envelope.spec.public_outputs.outputs = vec![public_output_cell.clone()];
    let public_output_digest = envelope
        .spec
        .public_outputs
        .digest()
        .expect("public output digest");
    let render_input_root = spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
        field_path: public_output_cell.public_field_path.clone(),
        node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: public_output_cell.public_field_path.clone(),
            cell_id: public_output_cell.cell_id.clone(),
            semantic_type_id: public_output_cell.semantic_type_id.clone(),
            schema_id: public_output_cell.schema_id.clone(),
            required_terminal: public_output_cell.required_terminal,
            value_lineage: public_output_cell.value_lineage.clone(),
        })),
    }]);
    for node in &mut envelope.spec.nodes {
        if node.node_id == fixture.render_node {
            if let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &mut node.framework {
                render.output_spec_digest = public_output_digest.clone();
                render.required_cells = vec![public_output_cell.clone()];
            }
            node.input_bindings.root = render_input_root.clone();
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("render input digest");
            node.deterministic_predecessors = vec![node_c.clone()];
        }
    }
    for lineage in &mut envelope.spec.value_lineages {
        if lineage.producer == spec::CellProducer::Node(fixture.render_node.clone()) {
            lineage.input_cells = vec![cell_c.clone()];
        }
    }

    let forward_a = envelope
        .spec
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.descriptor_a)
        .expect("node a")
        .clone();
    let forward_b = envelope
        .spec
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("node b")
        .clone();
    let cell_a = envelope
        .spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == fixture.cell_a)
        .expect("cell a")
        .clone();
    let remediation_a = remediation_node_for_forward(
        &forward_a,
        &cell_a,
        DigestBytes::from_array([0x80; 32]),
        DigestBytes::from_array([0x81; 32]),
        DigestBytes::from_array([0x82; 32]),
    );
    let remediation_b = remediation_node_for_forward(
        &forward_b,
        &cell_b,
        DigestBytes::from_array([0x83; 32]),
        DigestBytes::from_array([0x84; 32]),
        DigestBytes::from_array([0x85; 32]),
    );
    append_remediation_output_cell(&mut envelope.spec, &remediation_a, content(0x86));
    append_remediation_output_cell(&mut envelope.spec, &remediation_b, content(0x87));
    envelope
        .spec
        .remediations
        .insert(forward_a.node_id.clone(), remediation_a);
    envelope
        .spec
        .remediations
        .insert(forward_b.node_id.clone(), remediation_b);
    envelope.spec.saga = spec::SagaPolicySpec::CompensateCompleted {
        on_remediation_unresolved: spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim,
    };

    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture.descriptor_c = Some(descriptor_c);
    fixture.cell_c = Some(cell_c);
    fixture
}

fn with_exclusive_resource_claims(mut fixture: Fixture, descriptors: &[DescriptorId]) -> Fixture {
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let side_effect = EffectKind::new("mfm.test", "side-effect", DigestAlgorithm::Sha256JcsV1, D8)
        .expect("side-effect");
    let side_effect_cap = CapabilityDescriptor::new(
        side_effect_capability_kind(),
        side_effect_capability_version(),
        CapabilityRole::ExternalMutationAuthority,
        "external-mutation",
    )
    .expect("side-effect cap");
    let side_effect_caps =
        CapabilitySetDescriptor::new(vec![side_effect_cap]).expect("side-effect caps");
    let contract_digest = content(0x88);
    let resource_claim = spec::ResourceClaimSpec::Exclusive {
        namespace: exclusive_resource_namespace(),
        key_schema: fixture.seed_ref.schema_id.clone(),
    };
    for node in envelope
        .spec
        .nodes
        .iter_mut()
        .chain(envelope.spec.remediations.values_mut())
    {
        if descriptors
            .iter()
            .any(|descriptor| descriptor == &node.descriptor_id)
        {
            node.effect_kind = side_effect.clone();
            node.capability_bindings = side_effect_caps.clone();
            node.adapter_bindings = vec![spec::AdapterBinding {
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
                binding_digest: None,
            }];
            node.side_effect = Some(spec::SideEffectContractSpec {
                contract_digest: contract_digest.clone(),
                resource_claim: resource_claim.clone(),
            });
        }
    }
    for descriptor in &mut envelope.spec.descriptor_identities {
        if let spec::DescriptorIdentity::State(identity) = descriptor {
            if descriptors
                .iter()
                .any(|descriptor| descriptor == &identity.descriptor_id)
            {
                identity.effect_kind = side_effect.clone();
                identity.effect_class = "sidefx".to_owned();
                identity.effect_name = "sidefx".to_owned();
                identity.capabilities = side_effect_caps.clone();
                identity.runner = "sidefx".to_owned();
                identity.side_effect_contract_digest = Some(contract_digest.clone());
            }
        }
    }
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn with_exact_touched_set_resource_claims(
    mut fixture: Fixture,
    descriptors: &[DescriptorId],
) -> Fixture {
    let mut envelope = fixture.runtime_spec.envelope().clone();
    for node in envelope
        .spec
        .nodes
        .iter_mut()
        .chain(envelope.spec.remediations.values_mut())
    {
        if descriptors
            .iter()
            .any(|descriptor| descriptor == &node.descriptor_id)
        {
            let side_effect = node
                .side_effect
                .as_mut()
                .expect("exact touched-set descriptor is a side-effect node");
            side_effect.resource_claim = spec::ResourceClaimSpec::ExactTouchedSet {
                namespace: exact_touched_set_resource_namespace(),
                evidence_schema: <FixtureSideEffectEvidence as mfm_values::MfmValue>::schema_id()
                    .expect("fixture side-effect evidence schema"),
            };
        }
    }
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn remediation_node_for_forward(
    forward: &spec::NodeSpec,
    forward_output: &spec::CellSpec,
    node_digest: DigestBytes,
    cell_digest: DigestBytes,
    input_descriptor_digest: DigestBytes,
) -> spec::NodeSpec {
    let mut remediation = forward.clone();
    remediation.node_id = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, node_digest);
    remediation.stable_key =
        spec::StableAuthorKey::new(format!("remediation/{}", forward.stable_key.as_str()))
            .expect("remediation stable key");
    remediation.output_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, cell_digest);
    remediation.input_bindings.root =
        spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: spec::PublicFieldPath::new("input").expect("field"),
            cell_id: forward.output_cell.clone(),
            semantic_type_id: forward_output.semantic_type_id.clone(),
            schema_id: forward_output.schema_id.clone(),
            required_terminal: spec::RequiredTerminal::ProducedOnly,
            value_lineage: forward_output.value_lineage.clone(),
        }));
    remediation.input_bindings.input_descriptor_id =
        DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, input_descriptor_digest);
    remediation.input_bindings.digest =
        content_digest_json(input_node_json(&remediation.input_bindings.root))
            .expect("remediation input digest");
    remediation.deterministic_predecessors = Vec::new();
    remediation
}

fn append_remediation_output_cell(
    typed: &mut spec::TypedExecutionSpec,
    remediation: &spec::NodeSpec,
    lineage_digest: ContentDigest,
) {
    let descriptor = typed
        .descriptor_identities
        .iter()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == remediation.descriptor_id =>
            {
                Some(state)
            }
            _ => None,
        })
        .expect("remediation descriptor");
    typed.cells.push(spec::CellSpec {
        cell_id: remediation.output_cell.clone(),
        producer: spec::CellProducer::Node(remediation.node_id.clone()),
        scope_id: remediation.scope_id.clone(),
        semantic_type_id: descriptor.output_semantic_type_id.clone(),
        schema_id: descriptor.output_schema_id.clone(),
        value_lineage: spec::ValueLineageRef { lineage_digest },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
}

struct NodeSpecFixture {
    node_id: NodeId,
    descriptor_id: DescriptorId,
    scope_id: ScopeId,
    state_name: &'static str,
    state_kind: StateKind,
    state_version: StateVersion,
    effect_kind: EffectKind,
    config_ref: spec::ConfigRef,
    input_schema: SchemaId,
    input_cell: CellId,
    input_lineage: spec::ValueLineageRef,
    output_cell: CellId,
    output_schema: SchemaId,
    semantic: SemanticTypeId,
    caps: CapabilitySetDescriptor,
    predecessors: Vec<NodeId>,
    adapter_bindings: Vec<spec::AdapterBinding>,
    planning: spec::PlanningLineage,
}

fn node_spec(fixture: NodeSpecFixture) -> spec::NodeSpec {
    spec::NodeSpec {
        node_id: fixture.node_id,
        stable_key: spec::StableAuthorKey::new(
            fixture.state_name.rsplit('.').next().expect("state key"),
        )
        .expect("stable key"),
        scope_id: fixture.scope_id,
        state_kind: fixture.state_kind,
        state_version: fixture.state_version,
        descriptor_id: fixture.descriptor_id,
        config_ref: fixture.config_ref,
        input_bindings: spec::InputBindingSpec {
            input_schema_id: fixture.input_schema,
            input_descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D6),
            root: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                field_path: spec::PublicFieldPath::new("input").expect("field"),
                cell_id: fixture.input_cell,
                semantic_type_id: fixture.semantic.clone(),
                schema_id: fixture.output_schema.clone(),
                required_terminal: spec::RequiredTerminal::ProducedOnly,
                value_lineage: fixture.input_lineage,
            })),
            digest: content(0x73),
        },
        output_cell: fixture.output_cell,
        effect_kind: fixture.effect_kind,
        capability_bindings: fixture.caps,
        adapter_bindings: fixture.adapter_bindings,
        side_effect: None,
        framework: None,
        planning_lineage: fixture.planning,
        deterministic_predecessors: fixture.predecessors,
    }
}

fn state_descriptor(
    node: &spec::NodeSpec,
    descriptor_id: DescriptorId,
    name: &str,
    effect_kind: EffectKind,
    capabilities: CapabilitySetDescriptor,
    runner: &str,
) -> spec::StateDescriptorIdentity {
    spec::StateDescriptorIdentity {
        descriptor_id,
        name: name.to_owned(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        config_schema_id: node.config_ref.schema_id.clone(),
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        output_schema_id: node
            .input_bindings
            .root
            .clone()
            .first_schema_or(node.config_ref.schema_id.clone()),
        output_semantic_type_id: match &node.input_bindings.root {
            spec::InputBindingNodeSpec::Cell(cell) => cell.semantic_type_id.clone(),
            _ => panic!("test input"),
        },
        effect_kind,
        effect_class: runner.to_owned(),
        effect_name: runner.to_owned(),
        effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
        capabilities,
        runner: runner.to_owned(),
        side_effect_contract_digest: None,
    }
}

trait FirstSchema {
    fn first_schema_or(&self, fallback: SchemaId) -> SchemaId;
}

impl FirstSchema for spec::InputBindingNodeSpec {
    fn first_schema_or(&self, fallback: SchemaId) -> SchemaId {
        match self {
            spec::InputBindingNodeSpec::Cell(cell) => cell.schema_id.clone(),
            _ => fallback,
        }
    }
}

fn content(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn input_node_json(node: &spec::InputBindingNodeSpec) -> serde_json::Value {
    match node {
        spec::InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
        spec::InputBindingNodeSpec::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": match cell.required_terminal {
                spec::RequiredTerminal::ProducedOnly => "produced_only",
                spec::RequiredTerminal::MaybeSkipped => "maybe_skipped",
            },
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.lineage_digest.as_str(),
        }),
        spec::InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        spec::InputBindingNodeSpec::Struct(fields) => serde_json::json!({
            "fields": fields.iter().map(|field| {
                serde_json::json!({
                    "field_path": field.field_path.as_str(),
                    "node": input_node_json(&field.node),
                })
            }).collect::<Vec<_>>(),
            "kind": "struct",
        }),
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering_json(*ordering),
        }),
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering_json(*ordering),
        }),
    }
}

fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
        spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
    }
}

fn stable_domain_key_ref_json(key: &spec::StableDomainKeyRef) -> serde_json::Value {
    serde_json::json!({
        "content_digest": key.content_digest.as_str(),
        "schema_id": key.schema_id.as_str(),
    })
}

fn artifact(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn side_effect_capability_kind() -> CapabilityKind {
    CapabilityKind::new(
        "mfm.test",
        "external-mutation",
        DigestAlgorithm::Sha256JcsV1,
        D9,
    )
    .expect("side-effect cap kind")
}

fn side_effect_capability_version() -> CapabilityVersion {
    CapabilityVersion::new("mfm.cap.external_mutation.v1").expect("side-effect cap version")
}

fn exclusive_resource_namespace() -> spec::ResourceNamespace {
    spec::ResourceNamespace::new("mfm.test.wallet_nonce").expect("resource namespace")
}

fn exact_touched_set_resource_namespace() -> spec::ResourceNamespace {
    spec::ResourceNamespace::new("mfm.test.wallet_nonce").expect("resource namespace")
}

fn exclusive_resource_key(fixture: &Fixture, value: &str) -> events::ResourceKeyEvidence {
    resource_key_in_namespace(fixture, exclusive_resource_namespace(), value)
}

fn resource_key_in_namespace(
    fixture: &Fixture,
    namespace: spec::ResourceNamespace,
    value: &str,
) -> events::ResourceKeyEvidence {
    events::ResourceKeyEvidence {
        namespace,
        key_schema_id: fixture.seed_ref.schema_id.clone(),
        key: events::ResourceKey::new(value).expect("resource key"),
    }
}

fn side_effect_ledger_key(attempt_no: u32) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!("ledger-{attempt_no}")).expect("ledger key")
}

fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Forward
}

fn forward_ledger_for_node(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
) -> events::SideEffectLedgerKey {
    let mut found = None;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == *node_id
            && matches!(
                projection.ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            )
        {
            assert!(
                found.replace(projection.ledger_key.clone()).is_none(),
                "node {node_id} has multiple forward ledgers"
            );
        }
    }
    found.expect("forward ledger for node")
}

fn side_effect_projection_for_run_node<P: std::borrow::Borrow<store::ProjectionSnapshot>>(
    projections: P,
    run_id: &RunId,
    node_id: &NodeId,
) -> Option<store::SideEffectProjection> {
    let projections = projections.borrow();
    projections.side_effects().find_map(|(_, projection)| {
        (projection.run_id == *run_id && projection.intent.node_id == *node_id)
            .then(|| projection.clone())
    })
}

async fn drive_until_side_effect_confirmation_without_output(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    output_cell: &CellId,
    context: &str,
) {
    for _ in 0..8 {
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
        let projection_snapshot = store.projection_snapshot();
        if side_effect_projection_for_run_node(&projection_snapshot, &fixture.run_id, &node.node_id)
            .is_some_and(|projection| {
                matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                ) && projection_snapshot.cell_terminal(output_cell).is_none()
            })
        {
            return;
        }
    }
    panic!("{context} was not reached");
}

async fn drive_until_cells_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    cells: &[CellId],
    context: &str,
) {
    for _ in 0..24 {
        if cells
            .iter()
            .all(|cell| store.projection_snapshot().cell_terminal(cell).is_some())
        {
            return;
        }
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
    }
    panic!("{context} did not become terminal");
}

#[derive(Clone, Copy)]
enum RemediationPhaseCheckpoint {
    SubmissionObserved,
    ConfirmationObserved,
}

async fn drive_until_remediation_phase(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    forward_ledger_key: &events::SideEffectLedgerKey,
    checkpoint: RemediationPhaseCheckpoint,
    context: &str,
) {
    for _ in 0..8 {
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
        let projection_snapshot = store.projection_snapshot();
        if remediation_projection_for_forward_ledger(&projection_snapshot, forward_ledger_key)
            .is_some_and(|projection| match checkpoint {
                RemediationPhaseCheckpoint::SubmissionObserved => {
                    matches!(
                        projection.phase,
                        store::SideEffectPhase::SubmissionObserved { .. }
                    )
                }
                RemediationPhaseCheckpoint::ConfirmationObserved => {
                    matches!(
                        projection.phase,
                        store::SideEffectPhase::ConfirmationObserved { .. }
                    )
                }
            })
        {
            return;
        }
    }
    panic!("{context} was not reached");
}

async fn drive_until_compensated_before_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) {
    for _ in 0..24 {
        let saga = store
            .projection_snapshot()
            .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
        if saga.run_mode == store::RunMode::Compensated
            && store.projection_snapshot().run_state(&fixture.run_id) != store::RunState::Completed
        {
            return;
        }
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive until compensated before terminal"),
            SchedulerStatus::Advanced
        );
    }
    panic!("compensated pre-terminal boundary was not reached");
}

fn remediation_intent_forward_links(
    store: &TestTypedRunStore,
    run_id: &RunId,
) -> Vec<events::SideEffectLedgerKey> {
    store
        .load_run_stream(run_id)
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                match &payload.ledger_purpose {
                    events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } => {
                        Some(forward_ledger_key.clone())
                    }
                    events::SideEffectLedgerPurpose::Forward => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn remediation_projection_for_forward_ledger<P: std::borrow::Borrow<store::ProjectionSnapshot>>(
    projections: P,
    forward_ledger_key: &events::SideEffectLedgerKey,
) -> Option<store::SideEffectProjection> {
    let projections = projections.borrow();
    projections.side_effects().find_map(|(_, projection)| {
        matches!(
            &projection.ledger_purpose,
            events::SideEffectLedgerPurpose::Remediation {
                forward_ledger_key: linked
            } if linked == forward_ledger_key
        )
        .then(|| projection.clone())
    })
}

fn assert_no_duplicate_side_effect_submissions(store: &TestTypedRunStore, run_id: &RunId) {
    let mut by_ledger = BTreeMap::<events::SideEffectLedgerKey, usize>::new();
    let mut forward_by_node = BTreeMap::<NodeId, usize>::new();
    let mut remediation_by_forward = BTreeMap::<events::SideEffectLedgerKey, usize>::new();
    for event in store.load_run_stream(run_id) {
        if let events::KernelEventPayload::SideEffectSubmissionObserved(payload) = event.payload() {
            *by_ledger.entry(payload.ledger_key.clone()).or_default() += 1;
            match &payload.ledger_purpose {
                events::SideEffectLedgerPurpose::Forward => {
                    *forward_by_node.entry(payload.node_id.clone()).or_default() += 1;
                }
                events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } => {
                    *remediation_by_forward
                        .entry(forward_ledger_key.clone())
                        .or_default() += 1;
                }
            }
        }
    }
    for (ledger, count) in by_ledger {
        assert_eq!(count, 1, "duplicate submission for ledger {ledger}");
    }
    for (node, count) in forward_by_node {
        assert_eq!(count, 1, "duplicate forward submission for node {node}");
    }
    for (forward_ledger, count) in remediation_by_forward {
        assert_eq!(
            count, 1,
            "duplicate remediation submission for forward ledger {forward_ledger}"
        );
    }
}

fn side_effect_submission_count_for_ledger(
    store: &TestTypedRunStore,
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectSubmissionObserved(payload)
                    if &payload.ledger_key == ledger_key
            )
        })
        .count()
}

fn remediation_submission_count_for_forward_ledger(
    store: &TestTypedRunStore,
    run_id: &RunId,
    forward_ledger_key: &events::SideEffectLedgerKey,
) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectSubmissionObserved(payload)
                    if matches!(
                        &payload.ledger_purpose,
                        events::SideEffectLedgerPurpose::Remediation {
                            forward_ledger_key: linked
                        } if linked == forward_ledger_key
                    )
            )
        })
        .count()
}

fn side_effect_fixture_digest(ctx: &ErasedRunCtx<'_>, role: &str) -> ContentDigest {
    content_digest_json(serde_json::json!({
        "attempt": ctx.attempt_id().as_str(),
        "node": ctx.node().node_id.as_str(),
        "role": role,
    }))
    .expect("side-effect fixture digest")
}

fn side_effect_fixture_artifact_pair(
    ctx: &ErasedRunCtx<'_>,
    role: &str,
) -> (ArtifactId, ContentDigest) {
    let digest = side_effect_fixture_digest(ctx, role);
    (
        ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
    )
}

fn side_effect_ledger_key_for_ctx(ctx: &ErasedRunCtx<'_>) -> events::SideEffectLedgerKey {
    if let Some(forward_ledger_key) = linked_forward_ledger_for_remediation(ctx) {
        events::SideEffectLedgerKey::new(format!(
            "remediation-{}-{}",
            forward_ledger_key,
            ctx.attempt_no()
        ))
        .expect("remediation ledger key")
    } else {
        events::SideEffectLedgerKey::new(format!(
            "forward-{}-{}-{}",
            ctx.run_id(),
            ctx.node().node_id,
            ctx.attempt_no()
        ))
        .expect("forward ledger key")
    }
}

fn side_effect_ledger_purpose_for_ctx(ctx: &ErasedRunCtx<'_>) -> events::SideEffectLedgerPurpose {
    linked_forward_ledger_for_remediation(ctx)
        .map(
            |forward_ledger_key| events::SideEffectLedgerPurpose::Remediation {
                forward_ledger_key,
            },
        )
        .unwrap_or(events::SideEffectLedgerPurpose::Forward)
}

fn linked_forward_ledger_for_remediation(
    ctx: &ErasedRunCtx<'_>,
) -> Option<events::SideEffectLedgerKey> {
    if let Some(projection) =
        side_effect_projection_for_attempt(ctx.projections(), ctx.node(), ctx.attempt_id())
            .expect("remediation projection lookup")
    {
        if let events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } =
            &projection.ledger_purpose
        {
            return Some(forward_ledger_key.clone());
        }
    }
    let forward_node_id = ctx
        .runtime_spec()
        .forward_node_for_remediation(&ctx.node().node_id)?;
    ctx.projections()
        .side_effects()
        .find_map(|(_, projection)| {
            (projection.intent.node_id == *forward_node_id
                && matches!(
                    &projection.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                )
                && matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                ))
            .then(|| projection.ledger_key.clone())
        })
}

fn side_effect_claim_owner(attempt_no: u32, generation: u32) -> events::RunnerInvocationId {
    events::RunnerInvocationId::new(format!("owner-{attempt_no}-{generation}"))
        .expect("claim owner")
}

fn side_effect_fencing_token(
    attempt_no: u32,
    generation: u32,
) -> events::side_effect::ClaimFencingToken {
    events::side_effect::ClaimFencingToken::new(format!("token-{attempt_no}-{generation}"))
        .expect("fencing token")
}

fn side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(ctx.node().config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn side_effect_claimed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectClaimed(events::side_effect::Claimed {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx),
        claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
    })
}

fn side_effect_prepared(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    side_effect_prepared_with_resource_key(ctx, ledger, invocation_epoch, claim_generation, None)
}

fn side_effect_prepared_with_resource_key(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
    resource_key: Option<events::ResourceKeyEvidence>,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectInvocationPrepared(events::side_effect::InvocationPrepared {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
        resource_key,
        prepared_artifact_id: None,
        prepared_hash: None,
    })
}

fn side_effect_failed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    failure_phase: events::side_effect::FailurePhase,
    retryable: bool,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectFailed(events::side_effect::Failed {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx),
        invocation_epoch,
        failure_phase,
        retryable,
        error: side_effect_error(retryable),
    })
}

fn side_effect_error(retryable: bool) -> events::MfmErrorInfo {
    events::MfmErrorInfo {
        code: events::ErrorCode::new("sidefx_failed").expect("error code"),
        category: events::ErrorCategory::SideEffect,
        retryable,
        safe_message: "side-effect failed".to_owned(),
        public_details: None,
        diagnostic_ref: None,
    }
}

fn public_output_error() -> events::MfmErrorInfo {
    events::MfmErrorInfo {
        code: events::ErrorCode::new("public_output_render_failed").expect("error code"),
        category: events::ErrorCategory::Runtime,
        retryable: true,
        safe_message: "public output render failed".to_owned(),
        public_details: None,
        diagnostic_ref: None,
    }
}
