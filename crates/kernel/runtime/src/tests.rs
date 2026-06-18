use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor, EffectSpec, ManagedPlatformWrite,
};
use mfm_ids::{
    ArtifactId, DigestBytes, EffectKind, EffectVersion, EventId, LoweringVersion, SchemaId,
    ScopeId, SeedId, SemanticTypeId, SpecVersion, StateKind, StateVersion,
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
use mfm_store::v1::{TypedProjectionRead, TypedRunEventStore};
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

macro_rules! store_typed_commit_request {
    (
        run_id: $run_id:expr,
        expected_next_seq: $expected_next_seq:expr,
        commit_key: $commit_key:expr,
        payloads: $payloads:expr,
        required_artifacts: $required_artifacts:expr,
        preconditions: $preconditions:expr $(,)?
    ) => {
        store::TypedCommitRequest::from_payloads(
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
        request: store::TypedCommitRequest,
    ) -> store::Result<store::CommitOutcome>;
}

impl TestPreparedCommitExt for store::InMemoryTypedRunStore {
    fn append_prepared_commit(
        &mut self,
        request: store::TypedCommitRequest,
    ) -> store::Result<store::CommitOutcome> {
        let admitted_artifacts = request.required_artifacts().to_vec();
        let commit = store::PreparedTypedCommit::new(request, admitted_artifacts)?;
        self.append_prepared_typed_commit(commit)
    }
}

fn validate_runtime_stream_for_tests(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    RuntimeRunView::from_stream(runtime_spec, run_id, stream).map(|_| ())
}

struct RecordedPreparedCommit {
    seq: store::StreamSeq,
    commit_key: store::CommitKey,
    payloads: Vec<events::KernelEventPayload>,
    admitted_artifacts: Vec<store::ArtifactEvidenceRef>,
}

struct RecordingTypedRunStore {
    inner: store::InMemoryTypedRunStore,
    commits: Vec<RecordedPreparedCommit>,
}

impl RecordingTypedRunStore {
    fn new() -> Self {
        Self {
            inner: store::InMemoryTypedRunStore::new(),
            commits: Vec::new(),
        }
    }
}

struct StaleOnceTypedRunStore {
    inner: store::InMemoryTypedRunStore,
    stale_terminal_injected: bool,
}

impl StaleOnceTypedRunStore {
    fn new() -> Self {
        Self {
            inner: store::InMemoryTypedRunStore::new(),
            stale_terminal_injected: false,
        }
    }
}

struct AsyncInMemoryTypedRunStore {
    inner: Mutex<store::InMemoryTypedRunStore>,
}

impl AsyncInMemoryTypedRunStore {
    fn new(inner: store::InMemoryTypedRunStore) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }

    fn with_inner<R>(&self, f: impl FnOnce(&store::InMemoryTypedRunStore) -> R) -> R {
        let inner = self.inner.lock().expect("async in-memory store lock");
        f(&inner)
    }
}

impl store::AsyncTypedRunEventStore for AsyncInMemoryTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_typed_commit<'a>(
        &'a self,
        commit: store::PreparedTypedCommit,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            self.inner
                .lock()
                .expect("async in-memory store lock")
                .append_prepared_typed_commit(commit)
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        Box::pin(async move {
            Ok(self
                .inner
                .lock()
                .expect("async in-memory store lock")
                .load_run_stream(run_id))
        })
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        Box::pin(async move {
            Ok(self
                .inner
                .lock()
                .expect("async in-memory store lock")
                .expected_next_seq(run_id))
        })
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        Box::pin(async move {
            Ok(self
                .inner
                .lock()
                .expect("async in-memory store lock")
                .projection_snapshot()
                .clone())
        })
    }
}

impl store::TypedProjectionRead for StaleOnceTypedRunStore {
    fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        self.inner.projection_snapshot()
    }
}

impl store::TypedRunEventStore for StaleOnceTypedRunStore {
    fn append_prepared_typed_commit(
        &mut self,
        commit: store::PreparedTypedCommit,
    ) -> store::Result<store::CommitOutcome> {
        let is_run_start = commit
            .request()
            .payloads()
            .iter()
            .any(|payload| matches!(payload, events::KernelEventPayload::RunStarted(_)));
        let should_inject = !self.stale_terminal_injected
            && !is_run_start
            && commit.request().payloads().iter().any(|payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::StateAttemptCompleted(_)
                        | events::KernelEventPayload::StateAttemptFailed(_)
                        | events::KernelEventPayload::StateAttemptInterrupted(_)
                )
            });
        if should_inject {
            self.stale_terminal_injected = true;
            let expected = commit.request().expected_next_seq();
            let run_id = commit.request().run_id().clone();
            self.inner.append_prepared_typed_commit(commit)?;
            return Err(store::StoreError::StaleExpectedNextSeq {
                expected,
                actual: self.inner.expected_next_seq(&run_id),
            });
        }
        self.inner.append_prepared_typed_commit(commit)
    }

    fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        self.inner.load_run_stream(run_id)
    }

    fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
        self.inner.expected_next_seq(run_id)
    }
}

impl store::TypedProjectionRead for RecordingTypedRunStore {
    fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        self.inner.projection_snapshot()
    }
}

impl store::TypedRunEventStore for RecordingTypedRunStore {
    fn append_prepared_typed_commit(
        &mut self,
        commit: store::PreparedTypedCommit,
    ) -> store::Result<store::CommitOutcome> {
        let payloads = commit.request().payloads().to_vec();
        let admitted_artifacts = commit.admitted_artifacts().to_vec();
        let outcome = self.inner.append_prepared_typed_commit(commit)?;
        if let store::CommitOutcome::Appended(batch) = &outcome {
            self.commits.push(RecordedPreparedCommit {
                seq: batch.seq(),
                commit_key: batch.commit_key().clone(),
                payloads,
                admitted_artifacts,
            });
        }
        Ok(outcome)
    }

    fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        self.inner.load_run_stream(run_id)
    }

    fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
        self.inner.expected_next_seq(run_id)
    }
}

type TestArtifactMap = BTreeMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>;

#[derive(Clone, Default)]
struct TestRuntimeArtifactStager {
    artifacts: Arc<Mutex<TestArtifactMap>>,
}

impl RuntimeArtifactStager for TestRuntimeArtifactStager {
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a> {
        Box::pin(async move {
            verify_artifact_bytes(&bytes, &evidence)?;
            self.artifacts
                .lock()
                .expect("test artifact stager lock")
                .insert(evidence.artifact_id.clone(), (bytes, evidence));
            Ok(())
        })
    }
}

impl store::RetainedArtifactReadProvider for TestRuntimeArtifactStager {
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
struct RecordingRuntimeArtifactStager {
    staged: Arc<Mutex<Vec<store::ArtifactEvidenceRef>>>,
    artifacts: Arc<Mutex<TestArtifactMap>>,
}

impl RuntimeArtifactStager for RecordingRuntimeArtifactStager {
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a> {
        Box::pin(async move {
            verify_artifact_bytes(&bytes, &evidence)?;
            self.staged
                .lock()
                .expect("recording stager lock")
                .push(evidence.clone());
            self.artifacts
                .lock()
                .expect("recording artifact stager lock")
                .insert(evidence.artifact_id.clone(), (bytes, evidence));
            Ok(())
        })
    }
}

impl store::RetainedArtifactReadProvider for RecordingRuntimeArtifactStager {
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

struct FailingAfterRuntimeArtifactStager {
    remaining_successes: AtomicUsize,
    artifacts: Mutex<TestArtifactMap>,
}

struct FailingNodeStateOutputArtifactStager {
    failed_node_id: NodeId,
    artifacts: Mutex<TestArtifactMap>,
}

impl FailingNodeStateOutputArtifactStager {
    fn new(failed_node_id: NodeId) -> Self {
        Self {
            failed_node_id,
            artifacts: Mutex::new(BTreeMap::new()),
        }
    }
}

impl RuntimeArtifactStager for FailingNodeStateOutputArtifactStager {
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a> {
        Box::pin(async move {
            verify_artifact_bytes(&bytes, &evidence)?;
            if evidence.artifact_role == events::ArtifactRole::StateOutput
                && evidence.producer_node_id.as_ref() == Some(&self.failed_node_id)
            {
                return Err(RuntimeError::Store(
                    "test node state-output staging failure".to_owned(),
                ));
            }
            self.artifacts
                .lock()
                .expect("node-failing artifact stager lock")
                .insert(evidence.artifact_id.clone(), (bytes, evidence));
            Ok(())
        })
    }
}

impl store::RetainedArtifactReadProvider for FailingNodeStateOutputArtifactStager {
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

impl FailingAfterRuntimeArtifactStager {
    fn after(successes: usize) -> Self {
        Self {
            remaining_successes: AtomicUsize::new(successes),
            artifacts: Mutex::new(BTreeMap::new()),
        }
    }
}

impl RuntimeArtifactStager for FailingAfterRuntimeArtifactStager {
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a> {
        Box::pin(async move {
            verify_artifact_bytes(&bytes, &evidence)?;
            if self
                .remaining_successes
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                self.artifacts
                    .lock()
                    .expect("failing artifact stager lock")
                    .insert(evidence.artifact_id.clone(), (bytes, evidence));
                Ok(())
            } else {
                Err(RuntimeError::Store(
                    "test artifact staging failure".to_owned(),
                ))
            }
        })
    }
}

impl store::RetainedArtifactReadProvider for FailingAfterRuntimeArtifactStager {
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

fn test_scheduler(registry: ErasedRunnerRegistry) -> SerialTypedScheduler {
    test_scheduler_with_stager(registry, Arc::new(TestRuntimeArtifactStager::default()))
}

fn test_scheduler_with_stager(
    registry: ErasedRunnerRegistry,
    artifact_stager: Arc<dyn RuntimeArtifactStore>,
) -> SerialTypedScheduler {
    SerialTypedScheduler::new(registry, artifact_stager)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive b"),
        SchedulerStatus::Advanced
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_some());
}

#[tokio::test]
async fn scheduler_completes_run_after_public_output_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let staged = Arc::new(Mutex::new(Vec::new()));
    let scheduler = test_scheduler_with_stager(
        register_fixture_capabilities(registry, &fixture),
        Arc::new(RecordingRuntimeArtifactStager {
            staged: Arc::clone(&staged),
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }),
    );
    let mut store = RecordingTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive full representative run"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );

    let stream = store.load_run_stream(&fixture.run_id);
    validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("representative stream validates");
    assert_every_certified_node_has_attempt(&fixture.runtime_spec, &stream);
    assert!(node_by_output(&fixture, &fixture.cell_a)
        .framework
        .is_none());
    assert!(matches!(
        &certified_bootstrap_run_node(&fixture.runtime_spec)
            .expect("bootstrap node")
            .framework,
        Some(spec::FrameworkNodeSpec::BootstrapRun(_))
    ));
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

    let staged_artifacts = staged.lock().expect("staged artifact lock").clone();
    let mut first_reference_by_artifact = BTreeMap::<ArtifactId, usize>::new();
    for (commit_index, commit) in store.commits.iter().enumerate() {
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
            assert!(
                staged_artifacts.contains(admitted),
                "artifact {} was admitted without runtime staging",
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
        let commit = &store.commits[commit_index];
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
async fn run_launch_executes_bootstrap_genesis_batch_and_stages_launch_artifacts() {
    let fixture = fixture();
    let staged = Arc::new(Mutex::new(Vec::new()));
    let scheduler = test_scheduler_with_stager(
        registered_fixture_runners(&fixture),
        Arc::new(RecordingRuntimeArtifactStager {
            staged: Arc::clone(&staged),
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }),
    );
    let mut store = store::InMemoryTypedRunStore::new();

    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let stream = store.load_run_stream(&fixture.run_id);
    let bootstrap_node =
        certified_bootstrap_run_node(&fixture.runtime_spec).expect("bootstrap node");
    let first = stream.first().expect("stream event");
    let genesis_commit = stream
        .iter()
        .take_while(|event| event.seq() == first.seq() && event.commit_key() == first.commit_key())
        .collect::<Vec<_>>();
    assert!(matches!(
        genesis_commit[0].payload(),
        events::KernelEventPayload::RunStarted(_)
    ));
    assert_eq!(genesis_commit[0].ordinal(), store::CommitOrdinal::new(0));
    let bootstrap_cell = genesis_commit
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == bootstrap_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("bootstrap receipt cell");
    assert_eq!(bootstrap_cell.cell_id, bootstrap_node.output_cell);
    assert!(genesis_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == bootstrap_node.node_id && payload.attempt_no == 1
        )
    }));
    assert!(genesis_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.node_id == bootstrap_node.node_id
                    && payload.attempt_id == bootstrap_cell.attempt_id
                    && payload.output_cell_id == bootstrap_node.output_cell
        )
    }));
    assert!(genesis_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(&bootstrap_node.node_id)
                    && payload.attempt_id.as_ref() == Some(&bootstrap_cell.attempt_id)
                    && payload.artifact_ref.artifact_id == bootstrap_cell.artifact_id
                    && payload.artifact_ref.content_digest == bootstrap_cell.content_digest
        )
    }));
    assert!(genesis_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::RetentionRefsAppended(payload)
                if payload.reason == events::RetentionReason::RunStarted
                    && payload.refs.iter().any(|reference|
                        reference.artifact_id == bootstrap_cell.artifact_id
                            && reference.content_digest == bootstrap_cell.content_digest
                            && reference.role == events::ArtifactRole::StateOutput)
        )
    }));

    let staged = staged.lock().expect("recording stager lock");
    assert!(staged.iter().any(|evidence| {
        evidence.artifact_id == bootstrap_cell.artifact_id
            && evidence.digest == bootstrap_cell.content_digest
            && evidence.producer_node_id.as_ref() == Some(&bootstrap_node.node_id)
            && evidence.artifact_role == events::ArtifactRole::StateOutput
    }));
    let run_started = match genesis_commit[0].payload() {
        events::KernelEventPayload::RunStarted(payload) => payload,
        _ => panic!("first genesis event must be RunStarted"),
    };
    assert!(staged.iter().any(|evidence| {
        evidence.artifact_id == run_started.spec_artifact_id
            && evidence.artifact_role == events::ArtifactRole::TypedExecutionSpec
    }));
    assert!(staged.iter().any(|evidence| {
        evidence.artifact_id == run_started.certificate_artifact_id
            && evidence.artifact_role == events::ArtifactRole::TypedSpecCertificate
    }));
    assert!(staged.iter().any(|evidence| {
        evidence.artifact_id == fixture.seed_ref.seed_artifact.artifact_id
            && evidence.artifact_role == events::ArtifactRole::SeedInput
    }));
}

#[tokio::test]
async fn run_launch_staging_failure_prevents_start_commit() {
    let fixture = fixture();
    let scheduler = test_scheduler_with_stager(
        registered_fixture_runners(&fixture),
        Arc::new(FailingAfterRuntimeArtifactStager::after(0)),
    );
    let mut store = store::InMemoryTypedRunStore::new();
    assert!(matches!(
        start_fixture_run(
            &scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await,
        Err(RuntimeError::Store(message))
            if message.contains("test artifact staging failure")
    ));
    assert!(store.load_run_stream(&fixture.run_id).is_empty());
}

#[tokio::test]
async fn runtime_rejects_bootstrap_receipt_artifact_ref_metadata_tampering() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let bootstrap_node =
        certified_bootstrap_run_node(&fixture.runtime_spec).expect("bootstrap node");
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let artifact_ref_pos = valid_stream
        .iter()
        .position(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.node_id.as_ref() == Some(&bootstrap_node.node_id)
            )
        })
        .expect("bootstrap artifact ref");
    let mut payload = match valid_stream[artifact_ref_pos].payload().clone() {
        events::KernelEventPayload::ArtifactReferenced(payload) => payload,
        _ => unreachable!("position checked"),
    };
    payload.artifact_ref.byte_len += 1;
    let corrupt_stream = rewrite_commit_payload(
        &valid_stream,
        artifact_ref_pos,
        events::KernelEventPayload::ArtifactReferenced(payload),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("bootstrap receipt artifact reference")
                || message.contains("sealed BootstrapRun genesis commit")
    ));
}

#[tokio::test]
async fn runtime_rejects_run_start_without_bootstrap_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let bootstrap_node =
        certified_bootstrap_run_node(&fixture.runtime_spec).expect("bootstrap node");
    let valid_stream = store.load_run_stream(&fixture.run_id);
    assert_eq!(
        valid_stream
            .iter()
            .filter(|event| matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if payload.node_id == bootstrap_node.node_id
            ))
            .count(),
        1
    );
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == bootstrap_node.node_id
        )
    });
    assert_eq!(corrupt_stream.len(), valid_stream.len() - 1);

    assert!(matches!(
        validate_historical_bootstrap_run_batch(
            &fixture.runtime_spec,
            &fixture.run_id,
            &corrupt_stream
        ),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("unexpected payload count")
    ));
    assert!(validate_runtime_stream_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        &corrupt_stream
    )
    .is_err());
}

#[tokio::test]
async fn public_output_receipt_staging_failure_leaves_open_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler_with_stager(
        registered_fixture_runners(&fixture),
        Arc::new(FailingNodeStateOutputArtifactStager::new(
            fixture.render_node.clone(),
        )),
    );
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive a");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive b");

    let stream_len_before = store.load_run_stream(&fixture.run_id).len();
    assert!(matches!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::Store(message))
            if message.contains("test node state-output staging failure")
    ));
    assert_eq!(
        store.load_run_stream(&fixture.run_id).len(),
        stream_len_before + 1
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.render_cell)
        .is_none());
    assert!(store.load_run_stream(&fixture.run_id).iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::PublicOutputProduced(_)
        )
    }));
    let render_attempt = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &fixture.render_node,
        1,
    )
    .expect("render attempt id");
    assert!(matches!(
        store
            .projection_snapshot()
            .attempt(&fixture.render_node, &render_attempt)
            .expect("open render attempt")
            .status,
        store::AttemptStatus::Started { .. }
    ));
}

#[tokio::test]
async fn scheduler_binds_staged_retention_refs_and_projects_manifest() {
    let fixture = fixture_with_retention_lifecycle_node();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let start_retention = store
        .projection_snapshot()
        .retention(&fixture.run_id)
        .expect("run-start retention");
    assert!(start_retention
        .refs
        .contains_key(&fixture.seed_ref.seed_artifact.artifact_id));

    let status = scheduler
        .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
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

    let projection = store
        .projection_snapshot()
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
    let mut store = store::InMemoryTypedRunStore::new();
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

    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("advance current store with retention projection");
    assert!(store
        .projection_snapshot()
        .retention(&fixture.run_id)
        .and_then(|retention| retention.manifest.as_ref())
        .is_some());

    let mut stale_store = StaleStreamStore {
        inner: &mut store,
        stream: stale_stream,
    };
    assert_eq!(
        scheduler
            .drive_once(&mut stale_store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("idempotent retention retry"),
        SchedulerStatus::Advanced
    );
}

#[tokio::test]
async fn retention_manifest_projection_rejects_corrupt_stream_before_commit() {
    let fixture = fixture_with_retention_lifecycle_node();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;

    let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_pos = corrupt_stream
        .iter()
        .position(|event| {
            event.ordinal() == store::CommitOrdinal::new(0)
                && matches!(
                    event.payload(),
                    events::KernelEventPayload::StateAttemptStarted(_)
                )
        })
        .expect("attempt-start event");
    let mut corrupt_payload = match corrupt_stream[corrupt_pos].payload().clone() {
        events::KernelEventPayload::StateAttemptStarted(payload) => payload,
        _ => unreachable!("position checked"),
    };
    corrupt_payload.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
    corrupt_stream[corrupt_pos] = rewrite_single_payload_envelope(
        &corrupt_stream[corrupt_pos],
        events::KernelEventPayload::StateAttemptStarted(corrupt_payload),
    );

    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(&corrupt_stream)
        .expect("projection rebuild accepts ordered corrupt stream");
    let mut corrupt_store = ReadOnlyCorruptStore {
        stream: corrupt_stream,
        projection,
    };

    assert!(matches!(
        scheduler
            .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("event payload spec hash")
    ));
}

#[tokio::test]
async fn runtime_rejects_standalone_retention_manifest_projection_history() {
    let fixture = fixture_with_retention_lifecycle_node();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
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
    let manifest = build_retention_manifest_artifact_with_producer(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store.load_run_stream(&fixture.run_id),
        Some(retention_node.node_id.clone()),
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
        RuntimeRunView::from_store(&fixture.runtime_spec, &fixture.run_id, &store),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("retention manifest projection was not produced")
    ));
}

#[tokio::test]
async fn runtime_rejects_completed_history_without_retention_projection() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_commit_containing(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::RetentionManifestProjected(_)
        )
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("started before certified input cell")
    ));
}

#[tokio::test]
async fn runtime_rejects_standalone_run_completed_after_retention_projection() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive to completion");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let completion_payload = valid_stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("run completion");
    let mut corrupt_stream = rewrite_stream_without_commit_containing(&valid_stream, |payload| {
        matches!(payload, events::KernelEventPayload::RunCompleted(_))
    });
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "standalone-run-completed-after-retention",
        events::KernelEventPayload::RunCompleted(completion_payload),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn runtime_rejects_complete_run_receipt_commit_without_run_completed() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
async fn runtime_rejects_complete_run_receipt_artifact_ref_metadata_tampering() {
    for mutation in ["byte_len", "media_type"] {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        start_fixture_run(
            &scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await
        .expect("start run");
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion");

        let complete_node =
            certified_complete_run_node(&fixture.runtime_spec).expect("complete node");
        let valid_stream = store.load_run_stream(&fixture.run_id);
        let artifact_ref_pos = valid_stream
            .iter()
            .position(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::ArtifactReferenced(payload)
                        if payload.node_id.as_ref() == Some(&complete_node.node_id)
                )
            })
            .expect("completion artifact ref");
        let mut payload = match valid_stream[artifact_ref_pos].payload().clone() {
            events::KernelEventPayload::ArtifactReferenced(payload) => payload,
            _ => unreachable!("checked above"),
        };
        match mutation {
            "byte_len" => payload.artifact_ref.byte_len += 1,
            "media_type" => {
                payload.artifact_ref.media_type =
                    spec::MediaType::new("application/octet-stream").expect("media type");
            }
            _ => unreachable!("known mutation"),
        }
        let corrupt_stream = rewrite_commit_payload(
            &valid_stream,
            artifact_ref_pos,
            events::KernelEventPayload::ArtifactReferenced(payload),
        );

        assert!(matches!(
            validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("sealed framework batch")
        ));
    }
}

#[tokio::test]
async fn runtime_rejects_bare_retention_refs_before_completion() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "bare-runtime-retention-ref",
        events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: fixture.run_id.clone(),
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            refs: vec![events::RetentionRef {
                artifact_id: fixture.seed_ref.seed_artifact.artifact_id.clone(),
                role: fixture.seed_ref.seed_artifact.role,
                content_digest: fixture.seed_ref.seed_artifact.content_digest.clone(),
            }],
            reason: events::RetentionReason::RuntimeEvidence,
        }),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("same-commit typed payload evidence")
    ));
}

#[tokio::test]
async fn runtime_rejects_missing_run_start_retention_refs() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                reason: events::RetentionReason::RunStarted,
                ..
            })
        )
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("sealed BootstrapRun genesis commit")
    ));
}

#[tokio::test]
async fn runtime_rejects_non_completed_run_completion_without_saga_terminal_authority() {
    for (name, outcome) in [
        ("compensated", events::RunCompletionOutcome::Compensated),
        (
            "manually-resolved",
            events::RunCompletionOutcome::ManuallyResolved,
        ),
        (
            "failed-without-acdc-claim",
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
        ),
    ] {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        start_fixture_run(
            &scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await
        .expect("start run");
        let seq = store.expected_next_seq(&fixture.run_id);
        let request = store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: seq,
            commit_key: store::CommitKey::new(format!("forged-{name}-completion"))
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunCompleted(
                events::RunCompleted {
                    run_id: fixture.run_id.clone(),
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    outcome,
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let forged = store::build_committed_batch(&request, seq).expect("forged completion batch");
        let mut stream = store.load_run_stream(&fixture.run_id);
        stream.extend(forged.events().iter().cloned());

        assert!(matches!(
            validate_runtime_stream_for_tests(
                &fixture.runtime_spec,
                &fixture.run_id,
                &stream,
            ),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("saga terminal resolution requires terminal saga mode")
        ));
    }
}

#[tokio::test]
async fn runtime_rejects_post_completion_retention_refs() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion"),
        SchedulerStatus::PublicOutputProjected
    );

    let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "post-completion-retention-ref",
        events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: fixture.run_id.clone(),
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            refs: vec![events::RetentionRef {
                artifact_id: fixture.seed_ref.seed_artifact.artifact_id.clone(),
                role: fixture.seed_ref.seed_artifact.role,
                content_digest: fixture.seed_ref.seed_artifact.content_digest.clone(),
            }],
            reason: events::RetentionReason::RuntimeEvidence,
        }),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("events after RunCompleted")
    ));
}

#[tokio::test]
async fn runtime_rejects_run_completed_with_active_retention_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
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
    let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
    let public_event_id = corrupt_stream
        .iter()
        .find(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::PublicOutputProduced(_)
            )
        })
        .expect("public output produced")
        .event_id()
        .clone();
    append_payloads_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "active-retention-at-completion",
        vec![
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: retention_node.node_id.clone(),
                attempt_id: AttemptId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0x75; 32]),
                ),
                attempt_no: 99,
                state_kind: retention_node.state_kind.clone(),
                state_version: retention_node.state_version.clone(),
            }),
            events::KernelEventPayload::RunCompleted(events::RunCompleted {
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
                        public_output_event_id: public_event_id,
                    },
                )),
            }),
        ],
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("attempts are active")
    ));
}

#[tokio::test]
async fn runtime_rejects_retained_evidence_between_retention_projection_and_completion() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive to completion");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let completion_payload = valid_stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("run completion");
    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    let attempt_id = AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x77; 32]),
    );
    let diagnostic_digest = content(0x78);
    let diagnostic_artifact =
        ArtifactId::from_digest(diagnostic_digest.algorithm(), *diagnostic_digest.digest());
    let diagnostic_ref = events::ArtifactEvidenceRef {
        artifact_id: diagnostic_artifact.clone(),
        role: events::ArtifactRole::RedactedDiagnostic,
        schema_id: retention_node.config_ref.schema_id.clone(),
        semantic_type_id: None,
        content_digest: diagnostic_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    };
    let mut corrupt_stream = rewrite_stream_without_commit_containing(&valid_stream, |payload| {
        matches!(payload, events::KernelEventPayload::RunCompleted(_))
    });
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "retained-evidence-attempt-start",
        events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: retention_node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            attempt_no: 100,
            state_kind: retention_node.state_kind.clone(),
            state_version: retention_node.state_version.clone(),
        }),
    );
    append_payloads_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "retained-evidence-after-retention-projection",
        vec![
            events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: Some(retention_node.node_id.clone()),
                attempt_id: Some(attempt_id.clone()),
                artifact_ref: diagnostic_ref.clone(),
            }),
            events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: retention_node.node_id.clone(),
                attempt_id,
                retryable: false,
                error: events::MfmErrorInfo {
                    diagnostic_ref: Some(diagnostic_ref.clone()),
                    ..public_output_error()
                },
            }),
            events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                run_id: fixture.run_id.clone(),
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                refs: vec![events::RetentionRef {
                    artifact_id: diagnostic_artifact,
                    role: events::ArtifactRole::RedactedDiagnostic,
                    content_digest: diagnostic_digest,
                }],
                reason: events::RetentionReason::RuntimeEvidence,
            }),
        ],
    );
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "completion-after-post-retention-evidence",
        events::KernelEventPayload::RunCompleted(completion_payload),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn runtime_rejects_extra_attempt_evidence_in_retention_projection_commit() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive to completion");

    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    let attempt_id = AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x79; 32]),
    );
    let diagnostic_digest = content(0x7a);
    let diagnostic_artifact =
        ArtifactId::from_digest(diagnostic_digest.algorithm(), *diagnostic_digest.digest());
    let diagnostic_ref = events::ArtifactEvidenceRef {
        artifact_id: diagnostic_artifact,
        role: events::ArtifactRole::RedactedDiagnostic,
        schema_id: retention_node.config_ref.schema_id.clone(),
        semantic_type_id: None,
        content_digest: diagnostic_digest,
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    };
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = prepend_payloads_to_retention_projection_commit_for_tests(
        &valid_stream,
        Vec::new(),
        vec![events::KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: Some(retention_node.node_id.clone()),
                attempt_id: Some(attempt_id.clone()),
                artifact_ref: diagnostic_ref.clone(),
            },
        )],
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains(
                "retention manifest projection commit contains unsupported payload"
            )
    ));
}

#[tokio::test]
async fn runtime_rejects_same_sequence_sidecar_commit_at_retention_projection() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive to completion");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream =
        append_same_sequence_sidecar_to_retention_projection_for_tests(&valid_stream);

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::Store(message)) if message.contains("multiple commit keys")
    ));
}

#[tokio::test]
async fn runtime_rejects_post_completion_retention_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion"),
        SchedulerStatus::PublicOutputProjected
    );

    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "post-completion-retention-attempt",
        events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: retention_node.node_id.clone(),
            attempt_id: AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, DF),
            attempt_no: 99,
            state_kind: retention_node.state_kind.clone(),
            state_version: retention_node.state_version.clone(),
        }),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("events after RunCompleted")
    ));
}

#[tokio::test]
async fn runtime_rejects_started_attempt_for_terminal_retention_node() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("produce first cell");

    let terminal_node = node_by_output(&fixture, &fixture.cell_a);
    let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "terminal-node-started-again",
        events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: terminal_node.node_id.clone(),
            attempt_id: AttemptId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x76; 32]),
            ),
            attempt_no: 99,
            state_kind: terminal_node.state_kind.clone(),
            state_version: terminal_node.state_version.clone(),
        }),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("after its output cell became terminal")
    ));
}

#[tokio::test]
async fn public_output_render_failure_resumes_and_completes() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive a");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
fn certified_runtime_spec_accepts_verified_bundle_authority() {
    let (certified, registry) = certifier_backed_runtime_authority();
    let bundle = certified.bundle().expect("certified bundle");
    let verified = mfm_certify::verify_certified_bundle(
        bundle.spec_bytes(),
        bundle.certificate_bytes(),
        &registry,
    )
    .expect("verified persisted bundle");
    let runtime = CertifiedRuntimeSpec::new(verified).expect("runtime authority");
    assert!(!runtime.topological_order().is_empty());
}

#[tokio::test]
async fn replay_rejects_run_completed_without_public_output_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        events::RetentionReason::RunStarted,
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
        let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
async fn runtime_rejects_public_output_retention_reason_on_user_commit() {
    struct RetainedStateOutputRunner {
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for RetainedStateOutputRunner {
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
                    staged_retention_refs: vec![StagedRetentionRefs::runtime_evidence(vec![
                        retention_ref_for_artifact(&artifact),
                    ])],
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
            RetainedStateOutputRunner {
                output_bytes: br#"{"retained":true}"#.to_vec(),
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive retained state output"),
        SchedulerStatus::Advanced
    );

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_pos = valid_stream
        .iter()
        .position(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                    reason: events::RetentionReason::RuntimeEvidence,
                    ..
                })
            )
        })
        .expect("runtime evidence retention refs");
    let mut corrupt_payload = match valid_stream[corrupt_pos].payload().clone() {
        events::KernelEventPayload::RetentionRefsAppended(payload) => payload,
        _ => unreachable!("position checked"),
    };
    corrupt_payload.reason = events::RetentionReason::PublicOutput;
    let corrupt_stream = rewrite_commit_payload(
        &valid_stream,
        corrupt_pos,
        events::KernelEventPayload::RetentionRefsAppended(corrupt_payload),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("public-output retention refs must be appended")
    ));
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let commit = store::PreparedTypedCommit::new(leaked_artifact_request, Vec::new())
        .expect("prepare leak probe");
    assert!(matches!(
        store.append_prepared_typed_commit(commit),
        Err(store::StoreError::MissingArtifact { artifact_id }) if artifact_id == staged_artifact
    ));
}

#[test]
fn materialization_rejects_seed_digest_not_certified() {
    let fixture = fixture();
    let mut seed = fixture.seed_ref.clone();
    seed.digest = content(0xee);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = store::InMemoryTypedRunStore::new();
    assert!(matches!(
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![seed],),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_missing_config_artifact_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = store::InMemoryTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence.config_artifacts.clear();
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture.run_id.clone(),
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
    let store = store::InMemoryTypedRunStore::new();
    let base = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    let mut bad_spec = base.clone();
    bad_spec.spec_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture.run_id.clone(),
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
            fixture.run_id.clone(),
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
            fixture.run_id.clone(),
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
            fixture.run_id.clone(),
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
    let mut store = store::InMemoryTypedRunStore::new();
    let launch =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .expect("prepare launch");

    let authority = scheduler
        .start_run_admitted(&mut store, &fixture.runtime_spec, launch)
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

    let run_started = store
        .load_run_stream(&fixture.run_id)
        .into_iter()
        .find_map(|event| match event.payload().clone() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .expect("RunStarted payload");
    assert_eq!(
        run_started.runner_executables,
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
    let store = store::InMemoryTypedRunStore::new();
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
    let store = store::InMemoryTypedRunStore::new();
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
    let mut store = store::InMemoryTypedRunStore::new();
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
    let error = resume_scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
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
    let error = resume_scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
async fn runner_invocation_requires_committed_config_reference() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let config_ref_pos = valid_stream
        .iter()
        .position(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
            )
        })
        .expect("config artifact reference");
    let mut corrupt_payload = match valid_stream[config_ref_pos].payload().clone() {
        events::KernelEventPayload::ArtifactReferenced(payload) => payload,
        _ => unreachable!("position checked"),
    };
    corrupt_payload.artifact_ref.role = events::ArtifactRole::StateOutput;
    let corrupt_stream = rewrite_commit_payload(
        &valid_stream,
        config_ref_pos,
        events::KernelEventPayload::ArtifactReferenced(corrupt_payload),
    );
    let mut corrupt_store = StaleStreamStore {
        inner: &mut store,
        stream: corrupt_stream,
    };

    assert!(matches!(
        scheduler
            .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("sealed BootstrapRun genesis commit")
    ));
}

#[tokio::test]
async fn runner_invocation_rejects_config_reference_outside_run_start_commit() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let config_ref = valid_stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::TypedConfig =>
            {
                Some(event.payload().clone())
            }
            _ => None,
        })
        .expect("config artifact reference");
    let mut corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
        )
    });
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "late-config-reference",
        config_ref,
    );
    let mut corrupt_store = StaleStreamStore {
        inner: &mut store,
        stream: corrupt_stream,
    };

    assert!(matches!(
        scheduler
            .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("sealed BootstrapRun genesis commit")
    ));
}

#[tokio::test]
async fn runner_invocation_requires_committed_produced_input_artifact_reference() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut corrupt_store = StaleStreamStore {
        inner: &mut store,
        stream: corrupt_stream,
    };

    assert!(matches!(
        scheduler
            .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InputMaterialization(message))
            if message.contains("is not committed in the run stream")
    ));
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
        1
    );
}

#[tokio::test]
async fn post_start_materialization_failure_terminalizes_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node = node_by_output(&fixture, &fixture.cell_b).clone();
    {
        let mut corrupt_store = MissingInputArtifactRefStore {
            inner: &mut store,
            producer_node_id,
        };
        assert_eq!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("terminalize runtime validation failure"),
        SchedulerStatus::Advanced
    );

    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runtime_validation_failed");
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert!(matches!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("synthetic corrupt stream authority")
    ));

    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempts = store
        .projection_snapshot()
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
}

#[tokio::test]
async fn runner_invocation_rejects_late_produced_input_artifact_reference() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let output_ref = valid_stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                    && payload.node_id.as_ref() == Some(&producer_node_id) =>
            {
                Some(event.payload().clone())
            }
            _ => None,
        })
        .expect("state output artifact reference");
    let mut corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                    && payload.node_id.as_ref() == Some(&producer_node_id)
        )
    });
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "late-state-output-reference",
        output_ref,
    );
    let mut corrupt_store = StaleStreamStore {
        inner: &mut store,
        stream: corrupt_stream,
    };

    assert!(matches!(
        scheduler
            .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("same-commit typed payload")
    ));
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
        0
    );
}

#[tokio::test]
async fn runner_invocation_rejects_unsupported_artifact_reference_role() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let artifact = store::ArtifactEvidenceRef {
        artifact_id: artifact(0xe1),
        digest: content(0xe2),
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::SideEffectIntent,
    };
    let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
    append_payload_commit_for_tests(
        &mut corrupt_stream,
        &fixture.run_id,
        "unsupported-artifact-reference",
        events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: Some(node.node_id.clone()),
            attempt_id: None,
            artifact_ref: event_artifact_ref_from_store(&artifact),
        }),
    );
    let mut corrupt_store = StaleStreamStore {
        inner: &mut store,
        stream: corrupt_stream,
    };

    assert!(matches!(
        scheduler
            .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("unsupported artifact reference role")
    ));
}

#[tokio::test]
async fn replay_rejects_terminal_cell_producer_outside_certified_spec() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn store_rejects_fact_without_started_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
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
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn replay_rejects_public_output_with_forged_rendered_digest() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive a");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
            let Some(store::CellTerminalProjection::Produced {
                artifact_id,
                content_digest,
                ..
            }) = store
                .projection_snapshot()
                .cell_terminal(&public_cell.cell_id)
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
    store
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
        .expect("append forged public output");

    assert!(matches!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("rendered digest")
    ));
}

#[tokio::test]
async fn replay_rejects_split_public_output_terminal_commit() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive a");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive b");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("render public output");

    let stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = split_public_output_payload_to_own_commit_for_tests(&stream);
    assert!(matches!(
        store::ProjectionSnapshot::rebuild_from_run_stream(&corrupt_stream),
        Err(store::StoreError::ProjectionConflict { message, .. })
            if message.contains("public output requires matching render receipt terminal")
    ));
}

#[tokio::test]
async fn old_model_framework_stream_without_attempt_start_rejects_on_runtime_load() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive a");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("drive b");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("render public output");

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
        .expect("render node");
    let stream = store.load_run_stream(&fixture.run_id);
    let old_model_stream = rewrite_stream_without_commit_containing(&stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::StateAttemptStarted(started)
                if started.node_id == render_node.node_id
        )
    });

    let old_model_store = ReadOnlyCorruptStore {
        stream: old_model_stream,
        projection: store::ProjectionSnapshot::default(),
    };
    let loader = crate::history::VerifiedRunContextLoader::new(
        crate::binding::BoundRuntimeContextLoader::new(registered_fixture_runners(&fixture)),
    );
    assert!(matches!(
        loader.load(&fixture.runtime_spec, &fixture.run_id, &old_model_store),
        Err(RuntimeError::Store(message))
            if message.contains("unsupported old stream model")
                && message.contains("StateAttemptStarted")
    ));
}

#[tokio::test]
async fn recovery_interrupts_started_pure_attempt_before_retrying_fresh_attempt() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("interrupt pure"),
        SchedulerStatus::Advanced
    );
    let interrupted_attempt = store
        .projection_snapshot()
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
}

#[tokio::test]
async fn recovery_delegates_started_side_effect_attempt_to_side_effect_lifecycle() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
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

#[tokio::test]
async fn recovery_sweep_includes_open_remediation_attempts() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let scheduler = compensated_saga_scheduler(&fixture);
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..12 {
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive forward side-effect phase"),
            SchedulerStatus::Advanced
        );
    }
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
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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

    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
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

    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = StaleOnceTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive after injected stale terminal append"),
        SchedulerStatus::Advanced
    );
    assert!(
        store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_some(),
        "scheduler must reload and observe the concurrently advanced terminal projection"
    );
}

#[tokio::test]
async fn side_effect_scheduler_commits_durable_ledger_phases_before_output() {
    let fixture = fixture_with_first_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive side effect phase"),
            SchedulerStatus::Advanced
        );
        let projection =
            side_effect_projection_for_attempt(store.projection_snapshot(), node, &attempt_id)
                .expect("projection lookup")
                .expect("side-effect projection");
        if matches!(
            projection.phase,
            store::SideEffectPhase::ConfirmationObserved { .. }
        ) && store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none()
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("materialize side-effect output"),
        SchedulerStatus::Advanced
    );

    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    let projection =
        side_effect_projection_for_attempt(store.projection_snapshot(), node, &attempt_id)
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
            events::KernelEventPayload::SideEffectClaimTakenOver(_)
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
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("advance before receipt"),
            SchedulerStatus::Advanced
        );
    }
    assert!(matches!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..4 {
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("advance through receipt"),
            SchedulerStatus::Advanced
        );
    }
    assert!(matches!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..4 {
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("advance through receipt"),
            SchedulerStatus::Advanced
        );
    }
    assert!(matches!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        FailActiveSideEffectAfterSagaRunner::new(&fixture),
    ));
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("prepare forward side effect"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        side_effect_projection_for_attempt(
            store.projection_snapshot(),
            &forward_node,
            &forward_attempt
        )
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("fail active pre-boundary forward attempt"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        side_effect_projection_for_attempt(
            store.projection_snapshot(),
            &forward_node,
            &forward_attempt
        )
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        FailActiveSideEffectAfterSagaRunner::new(&fixture),
    ));
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("advance forward side effect before not-submitted proof"),
            SchedulerStatus::Advanced
        );
    }
    append_not_submitted_proven(&mut store, &fixture, &forward_node, &forward_attempt, 1);
    assert!(matches!(
        side_effect_projection_for_attempt(
            store.projection_snapshot(),
            &forward_node,
            &forward_attempt
        )
        .expect("side-effect lookup")
        .expect("side-effect projection")
        .phase,
        store::SideEffectPhase::NotSubmittedProven { .. }
    ));

    let failing_attempt = append_attempt_start(&mut store, &fixture, &failing_node, 1);
    append_attempt_failure(&mut store, &fixture, &failing_node, &failing_attempt, false);

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("fail active not-submitted forward attempt"),
        SchedulerStatus::Advanced
    );
    assert!(matches!(
        side_effect_projection_for_attempt(
            store.projection_snapshot(),
            &forward_node,
            &forward_attempt
        )
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
            DeterministicSideEffectRunner::new(&fixture),
        ))
        .expect("binding forward a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "sidefx",
            DeterministicSideEffectRunner::new(&fixture),
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..12 {
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive forward side-effect phase"),
            SchedulerStatus::Advanced
        );
    }
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_some());
    let forward_a_ledger = forward_ledger_for_node(store.projection_snapshot(), &forward_a.node_id);
    let forward_b_ledger = forward_ledger_for_node(store.projection_snapshot(), &forward_b.node_id);

    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&fixture.run_id, &fixture.runtime_spec.spec().saga);
    assert_eq!(saga.run_mode, store::RunMode::Remediating);
    assert_eq!(saga.obligations.len(), 2);

    for _ in 0..12 {
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive remediation phase"),
            SchedulerStatus::Advanced
        );
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
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("resolve compensated terminal"),
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
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
    let forward_a_ledger = forward_ledger_for_node(store.projection_snapshot(), &forward_a.node_id);
    let forward_b_ledger = forward_ledger_for_node(store.projection_snapshot(), &forward_b.node_id);

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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
            DeterministicSideEffectRunner::new(&fixture),
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
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive forward side-effect to confirmation"),
            SchedulerStatus::Advanced
        );
        let projection = side_effect_projection_for_attempt(
            store.projection_snapshot(),
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
    let projection = side_effect_projection_for_attempt(
        store.projection_snapshot(),
        &forward_node,
        &forward_attempt,
    )
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
            AmbiguousSideEffectRunner::new(&fixture),
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
    let artifact_store = Arc::new(TestRuntimeArtifactStager::default());
    let scheduler = test_scheduler_with_stager(
        register_fixture_capabilities(registry.clone(), &fixture),
        artifact_store.clone(),
    );
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
        fresh_scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
            AmbiguousSideEffectRunner::new(&fixture),
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
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let error = build_manual_resolution_prefix_authority(
        &store,
        &fixture.runtime_spec,
        &fixture.run_id,
        manual.clone(),
    )
    .expect_err("manual prefix rejects open attempt");
    assert!(
        matches!(&error, RuntimeError::InvalidRunStream(message) if message.contains("requires no open semantic attempts")),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_rejects_historical_manual_resolution_with_open_attempt_prefix() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            AmbiguousSideEffectRunner::new(&fixture),
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
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("advance to manual block"),
            SchedulerStatus::Advanced
        );
    }

    let mut manual_payload_store = store.clone();
    append_manual_resolution(
        &scheduler,
        &mut manual_payload_store,
        &fixture,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    let manual_payload = manual_payload_store
        .load_run_stream(&fixture.run_id)
        .into_iter()
        .find_map(|event| match event.payload().clone() {
            events::KernelEventPayload::ManualResolutionRecorded(payload) => Some(payload),
            _ => None,
        })
        .expect("manual resolution payload");

    let node_b = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("node b")
        .clone();
    let open_attempt = append_attempt_start(&mut store, &fixture, &node_b, 2);
    let mut stream = store.load_run_stream(&fixture.run_id);
    append_payload_commit_for_tests(
        &mut stream,
        &fixture.run_id,
        "forged-manual-resolution-open-prefix",
        events::KernelEventPayload::ManualResolutionRecorded(manual_payload),
    );
    append_payload_commit_without_prefix_validation_for_tests(
        &mut stream,
        &fixture.run_id,
        "close-open-attempt-after-forged-manual",
        events::KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node_b.node_id.clone(),
            attempt_id: open_attempt,
        }),
    );

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &stream),
        Err(RuntimeError::Store(message) | RuntimeError::InvalidRunStream(message))
            if message.contains("requires no open semantic attempts")
    ));
}

#[tokio::test]
async fn runtime_missing_manual_terminal_authorization_artifact_leaves_open_attempt() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            AmbiguousSideEffectRunner::new(&fixture),
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
    let artifact_store = Arc::new(TestRuntimeArtifactStager::default());
    let scheduler = test_scheduler_with_stager(
        register_fixture_capabilities(registry, &fixture),
        artifact_store.clone(),
    );
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    artifact_store
        .artifacts
        .lock()
        .expect("test artifact store")
        .remove(&manual.authorization_artifact_id)
        .expect("authorization artifact was staged");
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
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
            AmbiguousSideEffectRunner::new(&fixture),
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let evidence_bytes = br#"{"operator_note":"too_early"}"#.to_vec();

    let error = scheduler
        .record_manual_resolution(
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..12 {
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive forward side-effect phase");
    }
    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("terminalize wrong remediation ledger purpose"),
        SchedulerStatus::Advanced
    );
    assert_failure_code_count(&store, "runner_output_invalid", 1);
}

#[tokio::test]
async fn runtime_rejects_exclusive_side_effect_without_resource_key() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        DeterministicSideEffectRunner::new(&fixture),
    ));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("terminalize missing exclusive resource key"),
        SchedulerStatus::Advanced
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
}

#[tokio::test]
async fn runtime_sequences_two_runs_on_same_exclusive_lane_until_release() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DA);
    let resource_key = exclusive_resource_key(&fixture, "wallet-1");
    let lane_key = store::ResourceLaneKey::from_evidence(&resource_key);
    let runner = side_effect_runner_with_resource_keys(
        &fixture,
        resource_keys_for_all_side_effects(&fixture, "wallet-1"),
    );
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(&fixture, runner));
    let mut store = store::InMemoryTypedRunStore::new();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    append_synthetic_run_started(&mut store, &fixture, &holder_run_id, "holder-run-start");
    let (holder_attempt, holder_ledger) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &node,
        "wallet-1",
        "holder-prepare",
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start peer run");
    assert_eq!(
        store
            .projection_snapshot()
            .resource_lane(&lane_key)
            .expect("held lane")
            .holder
            .run_id,
        holder_run_id
    );

    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("peer blocks on lane"),
        SchedulerStatus::Advanced
    );
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &node.node_id
    )
    .is_none());
    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("peer remains blocked"),
        SchedulerStatus::Blocked
    );

    append_synthetic_side_effect_failed(
        &mut store,
        &fixture,
        &holder_run_id,
        &node,
        &holder_attempt,
        &holder_ledger,
        "holder-release",
    );
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("peer prepares after release"),
        SchedulerStatus::Advanced
    );
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &node.node_id
    )
    .is_some());
}

#[tokio::test]
async fn async_runtime_blocks_exclusive_lane_before_staging_artifact() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DA);
    let runner = side_effect_runner_with_resource_keys(
        &fixture,
        resource_keys_for_all_side_effects(&fixture, "wallet-async"),
    );
    let staged = Arc::new(Mutex::new(Vec::new()));
    let artifacts = Arc::new(Mutex::new(TestArtifactMap::new()));
    let scheduler = test_scheduler_with_stager(
        registered_first_side_effect_runners_with(&fixture, runner),
        Arc::new(RecordingRuntimeArtifactStager {
            staged: Arc::clone(&staged),
            artifacts,
        }),
    );
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    let mut inner = store::InMemoryTypedRunStore::new();
    append_synthetic_run_started(
        &mut inner,
        &fixture,
        &holder_run_id,
        "async-holder-run-start",
    );
    append_synthetic_exclusive_prepare(
        &mut inner,
        &fixture,
        &holder_run_id,
        &node,
        "wallet-async",
        "async-holder-prepare",
    );
    let store = AsyncInMemoryTypedRunStore::new(inner);
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture.run_id.clone(),
            run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            store::StreamSeq::FIRST,
        )
        .expect("prepare async peer launch");
    scheduler
        .start_run_async(&store, launch)
        .await
        .expect("start async peer run");
    let staged_before_block = staged.lock().expect("staged lock").len();

    assert_eq!(
        scheduler
            .drive_until_blocked_async(&store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("async peer blocks on lane"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        staged.lock().expect("staged lock").len(),
        staged_before_block
    );
    store.with_inner(|inner| {
        assert!(side_effect_projection_for_run_node(
            inner.projection_snapshot(),
            &fixture.run_id,
            &node.node_id
        )
        .is_none());
    });

    assert_eq!(
        scheduler
            .drive_until_blocked_async(&store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("async peer remains blocked on lane"),
        SchedulerStatus::Blocked
    );
    assert_eq!(
        staged.lock().expect("staged lock").len(),
        staged_before_block
    );
}

#[tokio::test]
async fn runtime_allows_unrelated_exclusive_keys_to_progress_across_runs() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DB);
    let runner = side_effect_runner_with_resource_keys(
        &fixture,
        resource_keys_for_all_side_effects(&fixture, "wallet-b"),
    );
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(&fixture, runner));
    let mut store = store::InMemoryTypedRunStore::new();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    append_synthetic_run_started(
        &mut store,
        &fixture,
        &holder_run_id,
        "unrelated-holder-start",
    );
    append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &node,
        "wallet-a",
        "unrelated-holder-prepare",
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start peer run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("peer prepares unrelated key"),
        SchedulerStatus::Advanced
    );
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &node.node_id
    )
    .is_some());
}

#[tokio::test]
async fn runtime_blocks_parallel_branches_of_one_run_on_held_exclusive_lane() {
    let fixture = fixture_with_run_id(fixture_with_independent_exclusive_side_effects(), DC);
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DD);
    let runner = side_effect_runner_with_resource_keys(
        &fixture,
        resource_keys_for_all_side_effects(&fixture, "wallet-shared"),
    );
    let scheduler = test_scheduler(registered_two_side_effect_runners_with(&fixture, runner));
    let mut store = store::InMemoryTypedRunStore::new();
    let node_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let node_b = node_by_output(&fixture, &fixture.cell_b).clone();
    append_synthetic_run_started(&mut store, &fixture, &holder_run_id, "branch-holder-start");
    append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &node_a,
        "wallet-shared",
        "branch-holder-prepare",
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start branched run");
    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("branched run blocks"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node_a.node_id),
        1
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node_b.node_id),
        0
    );
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &node_a.node_id
    )
    .is_none());
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &node_b.node_id
    )
    .is_none());
}

#[tokio::test]
async fn runtime_advances_independent_node_while_resource_lane_is_parked() {
    let base = fixture_with_independent_second_node_and_first_side_effect_state();
    let descriptors = vec![base.descriptor_a.clone()];
    let fixture = with_exclusive_resource_claims(base, &descriptors);
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DA);
    let runner = side_effect_runner_with_resource_keys(
        &fixture,
        resource_keys_for_all_side_effects(&fixture, "wallet-parked"),
    );
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(&fixture, runner));
    let mut store = store::InMemoryTypedRunStore::new();
    let blocked_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let independent_node = node_by_output(&fixture, &fixture.cell_b).clone();
    append_synthetic_run_started(
        &mut store,
        &fixture,
        &holder_run_id,
        "independent-holder-start",
    );
    append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &blocked_node,
        "wallet-parked",
        "independent-holder-prepare",
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start independent run");

    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("independent node advances while lane is parked"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &blocked_node.node_id),
        1
    );
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &blocked_node.node_id
    )
    .is_none());
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &independent_node.node_id),
        1
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&independent_node.output_cell)
        .is_some());
}

#[tokio::test]
async fn runtime_advances_independent_side_effect_lane_while_resource_lane_is_parked() {
    let fixture = fixture_with_independent_exclusive_side_effect_lanes();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DB);
    let blocked_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let independent_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let mut resource_keys = BTreeMap::new();
    resource_keys.insert(
        blocked_node.node_id.clone(),
        exclusive_resource_key(&fixture, "wallet-parked"),
    );
    resource_keys.insert(
        independent_node.node_id.clone(),
        resource_key_in_namespace(
            &fixture,
            independent_resource_namespace(),
            "wallet-independent",
        ),
    );
    let scheduler = test_scheduler(registered_two_side_effect_runners_with(
        &fixture,
        side_effect_runner_with_resource_keys(&fixture, resource_keys),
    ));
    let mut store = store::InMemoryTypedRunStore::new();
    append_synthetic_run_started(
        &mut store,
        &fixture,
        &holder_run_id,
        "independent-lane-holder-start",
    );
    append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &blocked_node,
        "wallet-parked",
        "independent-lane-holder-prepare",
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start independent lane run");

    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("independent side-effect lane advances while another lane is parked"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &blocked_node.node_id),
        1
    );
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &blocked_node.node_id
    )
    .is_none());
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &independent_node.node_id
    )
    .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&independent_node.output_cell)
        .is_some());
}

#[tokio::test]
async fn runtime_parks_same_namespace_side_effect_until_blocked_lane_releases() {
    let fixture = fixture_with_independent_exclusive_side_effects();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DD);
    let blocked_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let same_namespace_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let mut resource_keys = BTreeMap::new();
    resource_keys.insert(
        blocked_node.node_id.clone(),
        exclusive_resource_key(&fixture, "wallet-parked"),
    );
    resource_keys.insert(
        same_namespace_node.node_id.clone(),
        exclusive_resource_key(&fixture, "wallet-other"),
    );
    let scheduler = test_scheduler(registered_two_side_effect_runners_with(
        &fixture,
        side_effect_runner_with_resource_keys(&fixture, resource_keys),
    ));
    let mut store = store::InMemoryTypedRunStore::new();
    append_synthetic_run_started(
        &mut store,
        &fixture,
        &holder_run_id,
        "same-namespace-holder-start",
    );
    let (holder_attempt, holder_ledger) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &blocked_node,
        "wallet-parked",
        "same-namespace-holder-prepare",
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start same-namespace run");

    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("same-namespace node waits while lane key is unknown"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &blocked_node.node_id),
        1
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &same_namespace_node.node_id),
        0
    );

    append_synthetic_side_effect_failed(
        &mut store,
        &fixture,
        &holder_run_id,
        &blocked_node,
        &holder_attempt,
        &holder_ledger,
        "same-namespace-holder-release",
    );
    let status = scheduler
        .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("same-namespace node advances after lane release");
    assert!(matches!(
        status,
        SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
    ));
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &blocked_node.node_id
    )
    .is_some());
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &same_namespace_node.node_id
    )
    .is_some());
}

#[tokio::test]
async fn transition_allows_same_namespace_node_with_different_projected_lane() {
    let fixture = fixture_with_independent_exclusive_side_effects();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DF);
    let blocked_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let same_namespace_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let scheduler = test_scheduler(registered_two_side_effect_runners_with(
        &fixture,
        DeterministicSideEffectRunner::new(&fixture),
    ));
    let mut store = store::InMemoryTypedRunStore::new();
    append_synthetic_run_started(
        &mut store,
        &fixture,
        &holder_run_id,
        "projected-lane-holder-start",
    );
    append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &blocked_node,
        "wallet-parked",
        "projected-lane-holder-prepare",
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start projected-lane run");
    let (same_namespace_attempt, _) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        &same_namespace_node,
        "wallet-other",
        "projected-lane-same-namespace-prepare",
    );

    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    let blocked_lane =
        store::ResourceLaneKey::from_evidence(&exclusive_resource_key(&fixture, "wallet-parked"));
    let blocked_lanes = BTreeSet::from([crate::attempt::ResourceLaneBlockWitness {
        node_id: blocked_node.node_id.clone(),
        lane_key: blocked_lane,
    }]);

    match crate::transition::TransitionLifecycle::decide(
        &fixture.runtime_spec,
        &fixture.run_id,
        &view,
        &blocked_lanes,
    )
    .expect("transition decision")
    {
        crate::transition::TransitionDecision::ContinueAttempt(attempt) => {
            assert_eq!(attempt.node.node_id, same_namespace_node.node_id);
            assert_eq!(attempt.attempt_id, Some(same_namespace_attempt));
            assert_eq!(attempt.attempt_no, 1);
        }
        crate::transition::TransitionDecision::StartNode(_)
        | crate::transition::TransitionDecision::StartRemediation(_)
        | crate::transition::TransitionDecision::AwaitManualResolution
        | crate::transition::TransitionDecision::ResolveSagaTerminal(_)
        | crate::transition::TransitionDecision::Blocked => {
            panic!("same-namespace node with different projected lane should continue")
        }
    }
}

#[tokio::test]
async fn runtime_blocks_remediation_lane_until_conflicting_holder_releases() {
    let fixture = fixture_with_two_exclusive_side_effects_and_failing_tail();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DE);
    let runner = side_effect_runner_with_resource_keys(
        &fixture,
        resource_keys_for_all_side_effects(&fixture, "wallet-remediate"),
    );
    let mut registry = registered_two_side_effect_runners_with(&fixture, runner.clone());
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start compensating run");

    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    drive_side_effect_to_confirmation(&scheduler, &mut store, &fixture, &forward_a).await;
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("materialize forward a"),
        SchedulerStatus::Advanced
    );
    drive_side_effect_to_confirmation(&scheduler, &mut store, &fixture, &forward_b).await;
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("materialize forward b"),
        SchedulerStatus::Advanced
    );
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let failure_attempt = append_attempt_start(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);

    append_synthetic_run_started(
        &mut store,
        &fixture,
        &holder_run_id,
        "remediation-holder-start",
    );
    let (holder_attempt, holder_ledger) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &forward_b,
        "wallet-remediate",
        "remediation-holder-prepare",
    );
    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("remediation blocks on lane"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("remediation remains blocked on lane"),
        SchedulerStatus::Blocked
    );
    assert!(remediation_intent_forward_links(&store, &fixture.run_id).is_empty());

    append_synthetic_side_effect_failed(
        &mut store,
        &fixture,
        &holder_run_id,
        &forward_b,
        &holder_attempt,
        &holder_ledger,
        "remediation-holder-release",
    );
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("remediation prepares after lane release"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        remediation_intent_forward_links(&store, &fixture.run_id).len(),
        1
    );
}

#[tokio::test]
async fn runtime_ambiguous_holder_releases_lane_for_peer() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DE);
    let resource_key = exclusive_resource_key(&fixture, "wallet-ambiguous");
    let lane_key = store::ResourceLaneKey::from_evidence(&resource_key);
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        side_effect_runner_with_resource_keys(
            &fixture,
            resource_keys_for_all_side_effects(&fixture, "wallet-ambiguous"),
        ),
    ));
    let mut store = store::InMemoryTypedRunStore::new();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    append_synthetic_run_started(
        &mut store,
        &fixture,
        &holder_run_id,
        "ambiguous-holder-start",
    );
    let (holder_attempt, holder_ledger) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &node,
        "wallet-ambiguous",
        "ambiguous-holder-prepare",
    );
    append_synthetic_invocation_started(
        &mut store,
        &fixture,
        &holder_run_id,
        &node,
        &holder_attempt,
        &holder_ledger,
        "ambiguous-holder-invocation-started",
    );
    append_synthetic_ambiguous(
        &mut store,
        &fixture,
        &holder_run_id,
        &node,
        &holder_attempt,
        &holder_ledger,
        "ambiguous-holder-evidence",
    );
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start peer");

    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("peer runs after ambiguous holder releases lane"),
        SchedulerStatus::PublicOutputProjected
    );
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &node.node_id
    )
    .is_some());
}

#[tokio::test]
async fn runtime_prepared_holder_lane_releases_at_run_terminal() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let holder_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, DF);
    let resource_key = exclusive_resource_key(&fixture, "wallet-prepared");
    let lane_key = store::ResourceLaneKey::from_evidence(&resource_key);
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        side_effect_runner_with_resource_keys(
            &fixture,
            resource_keys_for_all_side_effects(&fixture, "wallet-prepared"),
        ),
    ));
    let mut store = store::InMemoryTypedRunStore::new();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    append_synthetic_run_started(
        &mut store,
        &fixture,
        &holder_run_id,
        "prepared-holder-start",
    );
    append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &holder_run_id,
        &node,
        "wallet-prepared",
        "prepared-holder-prepare",
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start peer");
    assert_eq!(
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("peer blocks on prepared holder"),
        SchedulerStatus::Advanced
    );
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &node.node_id
    )
    .is_none());
    append_synthetic_completed_terminal(
        &mut store,
        &fixture,
        &holder_run_id,
        "prepared-holder-terminal",
    );
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("peer prepares after holder terminal"),
        SchedulerStatus::Advanced
    );
    assert!(side_effect_projection_for_run_node(
        store.projection_snapshot(),
        &fixture.run_id,
        &node.node_id
    )
    .is_some());
}

#[tokio::test]
async fn side_effect_not_submitted_resume_claims_next_epoch() {
    let fixture = fixture_with_first_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("prepare side effect");
    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("take over and start invocation");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    append_not_submitted_proven(&mut store, &fixture, node, &attempt_id, 1);

    scheduler
        .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
        .await
        .expect("resume not-submitted");
    let projection =
        side_effect_projection_for_attempt(store.projection_snapshot(), node, &attempt_id)
            .expect("projection lookup")
            .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::InvocationStarted {
            invocation_epoch: 2,
            claim_generation: 3,
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
            AmbiguousSideEffectRunner::new(&fixture),
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
    let mut store = store::InMemoryTypedRunStore::new();
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
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("advance to ambiguity"),
            SchedulerStatus::Advanced
        );
    }
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("ambiguous side effect resolves terminal"),
        SchedulerStatus::Advanced
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
            AmbiguousSideEffectRunner::new(&fixture),
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..3 {
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("advance to ambiguity");
    }
    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("ambiguity resolves terminal"),
        SchedulerStatus::Advanced
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
    let mut store = store::InMemoryTypedRunStore::new();
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    assert_eq!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
struct DeterministicSideEffectRunner {
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    resource_keys: BTreeMap<NodeId, events::ResourceKeyEvidence>,
}

impl DeterministicSideEffectRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            cap_kind: side_effect_capability_kind(),
            cap_version: side_effect_capability_version(),
            adapter_kind: fixture.adapter_kind.clone(),
            adapter_version: fixture.adapter_version.clone(),
            resource_keys: BTreeMap::new(),
        }
    }

    fn with_resource_keys(
        mut self,
        resource_keys: BTreeMap<NodeId, events::ResourceKeyEvidence>,
    ) -> Self {
        self.resource_keys = resource_keys;
        self
    }

    fn resource_key_for_ctx(&self, ctx: &ErasedRunCtx<'_>) -> Option<events::ResourceKeyEvidence> {
        self.resource_keys.get(&ctx.node().node_id).cloned()
    }

    fn prepared(
        &self,
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        claim_generation: u32,
    ) -> RunnerEventPayload {
        side_effect_prepared_with_resource_key(
            ctx,
            ledger,
            invocation_epoch,
            claim_generation,
            self.resource_key_for_ctx(ctx),
        )
    }
}

impl ErasedNodeRunner for DeterministicSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let ledger = side_effect_ledger_key_for_ctx(&ctx);
            let phase = side_effect_projection_for_attempt(
                ctx.projections(),
                ctx.node(),
                ctx.attempt_id(),
            )?;
            match phase {
                None => self.prepare(ctx, ledger),
                Some(projection)
                    if matches!(
                        projection.phase,
                        store::SideEffectPhase::Claimed { .. }
                            | store::SideEffectPhase::InvocationPrepared { .. }
                    ) =>
                {
                    let claim = projection.claim.as_ref().expect("claim projection");
                    Ok(ErasedRunnerOutput::new(vec![
                        side_effect_claim_taken_over(&ctx, ledger.clone(), claim, 2),
                        self.prepared(&ctx, ledger.clone(), 1, 2),
                        side_effect_invocation_started(&ctx, ledger, 1, 2),
                    ]))
                }
                Some(store::SideEffectProjection {
                    phase:
                        store::SideEffectPhase::InvocationStarted {
                            invocation_epoch, ..
                        }
                        | store::SideEffectPhase::SubmissionUnknown { invocation_epoch },
                    ..
                }) => {
                    let (artifact_id, digest) =
                        side_effect_fixture_artifact_pair(&ctx, "submission");
                    let staged_artifact = staged_side_effect_artifact(
                        &ctx,
                        side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::Submission,
                        ),
                        ledger.clone(),
                        *invocation_epoch,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![side_effect_submission_observed(
                            &ctx,
                            ledger,
                            *invocation_epoch,
                            artifact_id,
                            digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::NotSubmittedProven { invocation_epoch },
                    claim,
                    ..
                }) => {
                    let claim = claim.as_ref().expect("claim projection");
                    let next_epoch = invocation_epoch + 1;
                    let next_generation = claim.claim_generation + 1;
                    Ok(ErasedRunnerOutput::new(vec![
                        side_effect_claimed(&ctx, ledger.clone(), next_epoch, next_generation),
                        self.prepared(&ctx, ledger.clone(), next_epoch, next_generation),
                        side_effect_invocation_started(&ctx, ledger, next_epoch, next_generation),
                    ]))
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::SubmissionObserved { invocation_epoch },
                    ..
                }) => {
                    let (artifact_id, digest) = side_effect_fixture_artifact_pair(&ctx, "receipt");
                    let staged_artifact = staged_side_effect_artifact(
                        &ctx,
                        side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::Receipt,
                        ),
                        ledger.clone(),
                        *invocation_epoch,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![side_effect_receipt_observed(
                            &ctx,
                            ledger,
                            *invocation_epoch,
                            artifact_id,
                            digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::ReceiptObserved { invocation_epoch },
                    ..
                }) => {
                    let (artifact_id, digest) =
                        side_effect_fixture_artifact_pair(&ctx, "confirmation");
                    let staged_artifact = staged_side_effect_artifact(
                        &ctx,
                        side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::Confirmation,
                        ),
                        ledger.clone(),
                        *invocation_epoch,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![side_effect_confirmation_observed(
                            &ctx,
                            ledger,
                            *invocation_epoch,
                            artifact_id,
                            digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::ConfirmationObserved { .. },
                    ..
                }) => {
                    let (output_artifact, output_digest) =
                        side_effect_fixture_artifact_pair(&ctx, "state-output");
                    let artifact = state_output_artifact(
                        ctx.node(),
                        ctx.descriptor(),
                        output_artifact.clone(),
                        output_digest.clone(),
                    );
                    let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: terminal_payloads(&ctx, output_artifact, output_digest),
                    })
                }
                Some(_) => Err(RuntimeError::Blocked(
                    "side-effect fixture blocked".to_owned(),
                )),
            }
        })
    }
}

impl DeterministicSideEffectRunner {
    fn prepare(
        &self,
        ctx: ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
    ) -> Result<ErasedRunnerOutput> {
        assert!(ctx.caps().contains(&self.cap_kind, &self.cap_version));
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
                        idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
                        idempotency_input_hash: side_effect_fixture_digest(&ctx, "idempotency"),
                        idempotency_key: events::IdempotencyKeyRef::new("idem-1")
                            .expect("idempotency key"),
                        capability_kind: self.cap_kind.clone(),
                        capability_version: self.cap_version.clone(),
                        adapter_kind: self.adapter_kind.clone(),
                        adapter_version: self.adapter_version.clone(),
                    },
                ),
                side_effect_claimed(&ctx, ledger.clone(), 1, 1),
                self.prepared(&ctx, ledger, 1, 1),
            ],
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

struct AmbiguousSideEffectRunner {
    inner: DeterministicSideEffectRunner,
}

impl AmbiguousSideEffectRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DeterministicSideEffectRunner::new(fixture),
        }
    }
}

impl ErasedNodeRunner for AmbiguousSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let ledger = side_effect_ledger_key_for_ctx(&ctx);
            let phase = side_effect_projection_for_attempt(
                ctx.projections(),
                ctx.node(),
                ctx.attempt_id(),
            )?;
            if matches!(
                phase.map(|projection| &projection.phase),
                Some(store::SideEffectPhase::InvocationStarted { .. })
            ) {
                let artifact_id = artifact(0xcb);
                let digest = content(0xca);
                let staged_artifact = staged_side_effect_artifact(
                    &ctx,
                    side_effect_artifact(
                        &ctx,
                        artifact_id.clone(),
                        digest.clone(),
                        events::ArtifactRole::AmbiguityEvidence,
                    ),
                    ledger.clone(),
                    1,
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: vec![RunnerEventPayload::SideEffectAmbiguous(
                        events::side_effect::Ambiguous {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            ledger_key: ledger,
                            ledger_purpose: side_effect_ledger_purpose_for_ctx(&ctx),
                            invocation_epoch: 1,
                            ambiguity_code: events::AmbiguityCode::new("unknown_submission")
                                .expect("ambiguity code"),
                            evidence_schema_id: ctx.node().config_ref.schema_id.clone(),
                            evidence_hash: digest,
                            evidence_artifact_id: artifact_id,
                        },
                    )],
                })
            } else {
                self.inner.run_erased(ctx).await
            }
        })
    }
}

struct TouchedSetSideEffectRunner {
    inner: DeterministicSideEffectRunner,
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
            inner: DeterministicSideEffectRunner::new(fixture),
            receipt: TouchedSetEmission::MatchPayloadSchema,
            confirmation: TouchedSetEmission::None,
        }
    }

    fn with_confirmation(fixture: &Fixture) -> Self {
        Self {
            inner: DeterministicSideEffectRunner::new(fixture),
            receipt: TouchedSetEmission::None,
            confirmation: TouchedSetEmission::MatchPayloadSchema,
        }
    }
}

impl ErasedNodeRunner for TouchedSetSideEffectRunner {
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
    inner: DeterministicSideEffectRunner,
}

impl FailActiveSideEffectAfterSagaRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DeterministicSideEffectRunner::new(fixture),
        }
    }
}

impl ErasedNodeRunner for FailActiveSideEffectAfterSagaRunner {
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
                if let Some((ledger, phase)) = projected {
                    if let Some((invocation_epoch, failure_phase)) =
                        saga_closure_failure_for_phase(&phase)
                    {
                        return Ok(ErasedRunnerOutput::new(vec![side_effect_failed(
                            &ctx,
                            ledger,
                            invocation_epoch,
                            failure_phase,
                            false,
                        )]));
                    }
                }
            }
            self.inner.run_erased(ctx).await
        })
    }
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
    inner: DeterministicSideEffectRunner,
}

impl RemediationEmitsForwardPurposeRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DeterministicSideEffectRunner::new(fixture),
        }
    }
}

impl ErasedNodeRunner for RemediationEmitsForwardPurposeRunner {
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
    inner: &'a mut store::InMemoryTypedRunStore,
    stream: Vec<store::KernelEventEnvelope>,
}

impl store::TypedProjectionRead for StaleStreamStore<'_> {
    fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        self.inner.projection_snapshot()
    }
}

impl store::TypedRunEventStore for StaleStreamStore<'_> {
    fn append_prepared_typed_commit(
        &mut self,
        commit: store::PreparedTypedCommit,
    ) -> store::Result<store::CommitOutcome> {
        self.inner.append_prepared_typed_commit(commit)
    }

    fn load_run_stream(&self, _run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        self.stream.clone()
    }

    fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
        self.inner.expected_next_seq(run_id)
    }
}

struct MissingInputArtifactRefStore<'a> {
    inner: &'a mut store::InMemoryTypedRunStore,
    producer_node_id: NodeId,
}

impl store::TypedProjectionRead for MissingInputArtifactRefStore<'_> {
    fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        self.inner.projection_snapshot()
    }
}

impl store::TypedRunEventStore for MissingInputArtifactRefStore<'_> {
    fn append_prepared_typed_commit(
        &mut self,
        commit: store::PreparedTypedCommit,
    ) -> store::Result<store::CommitOutcome> {
        self.inner.append_prepared_typed_commit(commit)
    }

    fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        rewrite_stream_without_payloads(&self.inner.load_run_stream(run_id), |payload| {
            matches!(
                payload,
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                        && payload.node_id.as_ref() == Some(&self.producer_node_id)
            )
        })
    }

    fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
        self.inner.expected_next_seq(run_id)
    }
}

struct ReadOnlyCorruptStore {
    stream: Vec<store::KernelEventEnvelope>,
    projection: store::ProjectionSnapshot,
}

impl store::TypedProjectionRead for ReadOnlyCorruptStore {
    fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        &self.projection
    }
}

impl store::TypedRunEventStore for ReadOnlyCorruptStore {
    fn append_prepared_typed_commit(
        &mut self,
        _commit: store::PreparedTypedCommit,
    ) -> store::Result<store::CommitOutcome> {
        Err(store::StoreError::Identity(
            "corrupt test store is read-only".to_owned(),
        ))
    }

    fn load_run_stream(&self, _run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        self.stream.clone()
    }

    fn expected_next_seq(&self, _run_id: &RunId) -> store::StreamSeq {
        store::StreamSeq::FIRST
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
        payload_canonical_byte_len: event.audit().payload_canonical_byte_len(),
    })
    .expect("rewritten envelope")
}

fn rewrite_commit_payload(
    stream: &[store::KernelEventEnvelope],
    target_pos: usize,
    payload: events::KernelEventPayload,
) -> Vec<store::KernelEventEnvelope> {
    let target = &stream[target_pos];
    let seq = target.seq();
    let commit_key = target.commit_key().clone();
    let mut positions = stream
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            (event.seq() == seq && event.commit_key() == &commit_key).then_some(index)
        })
        .collect::<Vec<_>>();
    positions.sort_by_key(|index| stream[*index].ordinal().as_u32());
    let target_ordinal = target.ordinal().as_u32() as usize;
    assert_eq!(positions[target_ordinal], target_pos);
    let mut payloads = positions
        .iter()
        .map(|index| stream[*index].payload().clone())
        .collect::<Vec<_>>();
    payloads[target_ordinal] = payload;
    let request = store_typed_commit_request! {
        run_id: target.run_id().clone(),
        expected_next_seq: seq,
        commit_key: commit_key,
            payloads: payloads,
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions::default(),
    };
    let batch =
        store::build_committed_batch(&request, seq).expect("rewritten commit payload batch");
    let mut rewritten = stream.to_vec();
    for (position, event) in positions.into_iter().zip(batch.events().iter().cloned()) {
        rewritten[position] = event;
    }
    rewritten
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

fn rewrite_stream_without_commit_containing<F>(
    stream: &[store::KernelEventEnvelope],
    mut should_remove_commit: F,
) -> Vec<store::KernelEventEnvelope>
where
    F: FnMut(&events::KernelEventPayload) -> bool,
{
    let mut rewritten = Vec::with_capacity(stream.len());
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let original_seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == original_seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        if commit
            .iter()
            .any(|event| should_remove_commit(event.payload()))
        {
            index = end;
            continue;
        }
        let seq = next_seq_for_stream_for_tests(first.run_id(), &rewritten);
        let request = store_typed_commit_request! {
            run_id: first.run_id().clone(),
            expected_next_seq: seq,
            commit_key: commit_key,
            payloads: commit
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>(),
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let batch =
            store::build_committed_batch(&request, seq).expect("rewritten commit payload batch");
        rewritten.extend(batch.events().iter().cloned());
        index = end;
    }
    rewritten
}

fn prepend_payloads_to_retention_projection_commit_for_tests(
    stream: &[store::KernelEventEnvelope],
    prelude_payloads: Vec<events::KernelEventPayload>,
    prefix_payloads: Vec<events::KernelEventPayload>,
) -> Vec<store::KernelEventEnvelope> {
    if prelude_payloads.is_empty() {
        let mut rewritten = Vec::with_capacity(stream.len() + prefix_payloads.len());
        let mut inserted = false;
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
            if commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                )
            }) {
                let mut payloads = prefix_payloads.clone();
                payloads.extend(commit.iter().map(|event| event.payload().clone()));
                let request = store_typed_commit_request! {
                    run_id: first.run_id().clone(),
                    expected_next_seq: seq,
                    commit_key: commit_key,
                    payloads: payloads,
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let batch = store::build_committed_batch(&request, seq)
                    .expect("rewritten retention projection batch");
                rewritten.extend(batch.events().iter().cloned());
                inserted = true;
            } else {
                rewritten.extend(commit.iter().cloned());
            }
            index = end;
        }
        assert!(inserted, "retention projection commit exists");
        return rewritten;
    }

    let mut rewritten =
        Vec::with_capacity(stream.len() + prelude_payloads.len() + prefix_payloads.len());
    let mut inserted = false;
    let mut next_seq = store::StreamSeq::FIRST;
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let original_seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == original_seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        if commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        }) {
            if !prelude_payloads.is_empty() {
                let request = store_typed_commit_request! {
                    run_id: first.run_id().clone(),
                    expected_next_seq: next_seq,
                    commit_key: store::CommitKey::new("retention-projection-prelude")
                        .expect("commit key"),
                    payloads: prelude_payloads.clone(),
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let batch = store::build_committed_batch(&request, next_seq)
                    .expect("retention projection prelude batch");
                rewritten.extend(batch.events().iter().cloned());
                next_seq = increment_stream_seq_for_tests(next_seq);
            }
            let mut payloads = prefix_payloads.clone();
            payloads.extend(commit.iter().map(|event| event.payload().clone()));
            let request = store_typed_commit_request! {
                run_id: first.run_id().clone(),
                expected_next_seq: next_seq,
                commit_key: commit_key,
                payloads: payloads,
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions::default(),
            };
            let batch = store::build_committed_batch(&request, next_seq)
                .expect("rewritten retention projection batch");
            rewritten.extend(batch.events().iter().cloned());
            next_seq = increment_stream_seq_for_tests(next_seq);
            inserted = true;
        } else {
            let request = store_typed_commit_request! {
                run_id: first.run_id().clone(),
                expected_next_seq: next_seq,
                commit_key: commit_key,
                payloads: commit.iter().map(|event| event.payload().clone()).collect(),
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions::default(),
            };
            let batch =
                store::build_committed_batch(&request, next_seq).expect("shifted commit batch");
            rewritten.extend(batch.events().iter().cloned());
            next_seq = increment_stream_seq_for_tests(next_seq);
        }
        index = end;
    }
    assert!(inserted, "retention projection commit exists");
    rewritten
}

fn append_same_sequence_sidecar_to_retention_projection_for_tests(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let mut rewritten = Vec::with_capacity(stream.len() + 1);
    let mut inserted = false;
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
        rewritten.extend(commit.iter().cloned());
        if commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        }) {
            let runtime_evidence = commit
                .iter()
                .find_map(|event| match event.payload() {
                    events::KernelEventPayload::RetentionRefsAppended(payload)
                        if payload.reason == events::RetentionReason::RuntimeEvidence =>
                    {
                        Some(payload)
                    }
                    _ => None,
                })
                .expect("retention projection runtime evidence refs");
            let sidecar_key =
                store::CommitKey::new("forged-same-seq-retention-sidecar").expect("commit key");
            let request = store_typed_commit_request! {
                run_id: first.run_id().clone(),
                expected_next_seq: seq,
                commit_key: sidecar_key.clone(),
                payloads: vec![events::KernelEventPayload::RetentionRefsAppended(
                    events::RetentionRefsAppended {
                        run_id: runtime_evidence.run_id.clone(),
                        spec_hash: runtime_evidence.spec_hash.clone(),
                        refs: runtime_evidence.refs.clone(),
                        reason: events::RetentionReason::RuntimeEvidence,
                    },
                )],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions::default(),
            };
            let batch = store::build_committed_batch(&request, seq).expect("sidecar commit batch");
            let next_ordinal = u32::try_from(commit.len()).expect("retention commit ordinal count");
            for (offset, event) in batch.events().iter().enumerate() {
                rewritten.push(rewrite_envelope(
                    event,
                    seq,
                    store::CommitOrdinal::new(
                        next_ordinal
                            .checked_add(u32::try_from(offset).expect("sidecar ordinal offset"))
                            .expect("sidecar ordinal"),
                    ),
                    sidecar_key.clone(),
                ));
            }
            inserted = true;
        }
        index = end;
    }
    assert!(inserted, "retention projection commit exists");
    rewritten
}

fn split_public_output_payload_to_own_commit_for_tests(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let mut rewritten = Vec::with_capacity(stream.len());
    let mut next_seq = store::StreamSeq::FIRST;
    let mut split = false;
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let original_seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == original_seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        if commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::PublicOutputProduced(_)
            )
        }) {
            let mut terminal_payloads = Vec::new();
            let mut public_event = None;
            for event in commit {
                if matches!(
                    event.payload(),
                    events::KernelEventPayload::PublicOutputProduced(_)
                ) {
                    public_event = Some(event);
                } else {
                    terminal_payloads.push(event.payload().clone());
                }
            }
            assert!(
                !terminal_payloads.is_empty(),
                "public-output commit keeps terminal evidence"
            );
            let terminal_request = store_typed_commit_request! {
                run_id: first.run_id().clone(),
                expected_next_seq: next_seq,
                commit_key: commit_key,
                payloads: terminal_payloads,
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions::default(),
            };
            let terminal_batch = store::build_committed_batch(&terminal_request, next_seq)
                .expect("terminal rewrite batch");
            rewritten.extend(terminal_batch.events().iter().cloned());
            next_seq = increment_stream_seq_for_tests(next_seq);

            rewritten.push(rewrite_envelope(
                public_event.expect("public output payload"),
                next_seq,
                store::CommitOrdinal::new(0),
                store::CommitKey::new("corrupt-public-output-split").expect("commit key"),
            ));
            next_seq = increment_stream_seq_for_tests(next_seq);
            split = true;
        } else {
            let request = store_typed_commit_request! {
                run_id: first.run_id().clone(),
                expected_next_seq: next_seq,
                commit_key: commit_key,
            payloads: commit.iter().map(|event| event.payload().clone()).collect(),
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions::default(),
            };
            let batch =
                store::build_committed_batch(&request, next_seq).expect("shifted commit batch");
            rewritten.extend(batch.events().iter().cloned());
            next_seq = increment_stream_seq_for_tests(next_seq);
        }
        index = end;
    }
    assert!(split, "public output payload exists");
    rewritten
}

fn increment_stream_seq_for_tests(seq: store::StreamSeq) -> store::StreamSeq {
    store::StreamSeq::new(seq.as_u64() + 1).expect("next shifted seq")
}

fn next_seq_for_stream_for_tests(
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> store::StreamSeq {
    store::CommittedRunStream::from_events(run_id.clone(), stream.to_vec())
        .expect("committed stream")
        .next_seq()
}

fn append_payload_commit_for_tests(
    stream: &mut Vec<store::KernelEventEnvelope>,
    run_id: &RunId,
    commit_key: &str,
    payload: events::KernelEventPayload,
) {
    append_payloads_commit_for_tests(stream, run_id, commit_key, vec![payload]);
}

fn append_payload_commit_without_prefix_validation_for_tests(
    stream: &mut Vec<store::KernelEventEnvelope>,
    run_id: &RunId,
    commit_key: &str,
    payload: events::KernelEventPayload,
) {
    let seq = stream
        .last()
        .map(|event| increment_stream_seq_for_tests(event.seq()))
        .unwrap_or(store::StreamSeq::FIRST);
    let request = store_typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: seq,
        commit_key: store::CommitKey::new(commit_key).expect("commit key"),
        payloads: vec![payload],
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions::default(),
    };
    let batch = store::build_committed_batch(&request, seq).expect("raw appended payload batch");
    stream.extend(batch.events().iter().cloned());
}

fn append_payloads_commit_for_tests(
    stream: &mut Vec<store::KernelEventEnvelope>,
    run_id: &RunId,
    commit_key: &str,
    payloads: Vec<events::KernelEventPayload>,
) {
    let seq = next_seq_for_stream_for_tests(run_id, stream);
    let request = store_typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: seq,
        commit_key: store::CommitKey::new(commit_key).expect("commit key"),
        payloads: payloads,
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions::default(),
    };
    let batch =
        store::build_committed_batch(&request, seq).expect("appended corrupt payload commit batch");
    stream.extend(batch.events().iter().cloned());
}

fn rewrite_single_payload_envelope(
    event: &store::KernelEventEnvelope,
    payload: events::KernelEventPayload,
) -> store::KernelEventEnvelope {
    assert_eq!(event.ordinal(), store::CommitOrdinal::new(0));
    let request = store_typed_commit_request! {
        run_id: event.run_id().clone(),
        expected_next_seq: event.seq(),
        commit_key: event.commit_key().clone(),
        payloads: vec![payload],
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions::default(),
    };
    let batch =
        store::build_committed_batch(&request, event.seq()).expect("rewritten payload batch");
    batch
        .events()
        .first()
        .expect("rewritten payload event")
        .clone()
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
        events::KernelEventPayload::RunStarted(payload) => {
            artifacts.push(payload.spec_artifact_id.clone());
            artifacts.push(payload.certificate_artifact_id.clone());
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

fn prepare_fixture_launch<S: store::TypedRunEventStore + ?Sized>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<PreparedRunLaunch> {
    scheduler.prepare_run_launch(
        &fixture.runtime_spec,
        fixture.run_id.clone(),
        run_start_evidence(fixture, seed_cells),
        store.expected_next_seq(&fixture.run_id),
    )
}

async fn start_fixture_run<S: store::TypedRunEventStore + ?Sized>(
    scheduler: &SerialTypedScheduler,
    store: &mut S,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<store::CommitOutcome> {
    let launch = prepare_fixture_launch(scheduler, store, fixture, seed_cells)?;
    scheduler.start_run(store, launch).await
}

fn run_start_evidence(
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> RunLaunchEvidence {
    RunLaunchEvidence {
        spec_artifact: spec_artifact(&fixture.runtime_spec),
        certificate_artifact: certificate_artifact(&fixture.runtime_spec),
        config_artifacts: fixture
            .runtime_spec
            .spec()
            .config_refs
            .iter()
            .map(|config| config_artifact(&fixture.runtime_spec, config))
            .collect(),
        framework_version: events::FrameworkVersion::new("mfm.test.1").expect("framework"),
        source_revision: events::SourceRevision::new("test-rev").expect("source"),
        adapter_executables: Vec::new(),
        seed_cells: seed_cells.into_iter().map(seed_launch_cell).collect(),
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

fn event_artifact_ref_from_store(
    artifact: &store::ArtifactEvidenceRef,
) -> events::ArtifactEvidenceRef {
    events::ArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id: artifact.schema_id.clone().expect("schema-bearing artifact"),
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    }
}

fn digest_for_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
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
    store: &mut store::InMemoryTypedRunStore,
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
    store: &mut store::InMemoryTypedRunStore,
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

fn append_synthetic_run_started(
    store: &mut store::InMemoryTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    commit_key: &str,
) {
    let spec_artifact = spec_artifact(&fixture.runtime_spec).evidence;
    let certificate_artifact = certificate_artifact(&fixture.runtime_spec).evidence;
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunStarted(events::RunStarted {
                run_id: run_id.clone(),
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                spec_artifact_id: spec_artifact.artifact_id.clone(),
                certificate_artifact_id: certificate_artifact.artifact_id.clone(),
                certificate_artifact_digest: certificate_artifact.digest.clone(),
                certificate_media_type: certificate_artifact.media_type.clone(),
                spec_media_type: spec_artifact.media_type.clone(),
                spec_version: SpecVersion::new(spec::SPEC_VERSION).expect("spec version"),
                lowering_version: LoweringVersion::new(spec::LOWERING_VERSION)
                    .expect("lowering version"),
                public_output_schema_id: fixture
                    .runtime_spec
                    .spec()
                    .public_outputs
                    .public_schema_id
                    .clone(),
                saga_policy_digest: fixture
                    .runtime_spec
                    .spec()
                    .saga
                    .saga_policy_digest()
                    .expect("saga policy digest"),
                descriptor_identities: Vec::new(),
                runner_executables: Vec::new(),
                adapter_executables: Vec::new(),
                canonicalizer_identity: spec::CanonicalizerIdentity::new("sha256-jcs-v1")
                    .expect("canonicalizer"),
                framework_version: events::FrameworkVersion::new("mfm.test.1").expect("framework"),
                source_revision: events::SourceRevision::new("test-rev").expect("source"),
                seed_cells: Vec::new(),
            })],
            required_artifacts: vec![spec_artifact, certificate_artifact],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::Absent,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic run start");
}

fn append_synthetic_exclusive_prepare(
    store: &mut store::InMemoryTypedRunStore,
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
                        prepared_artifact_id: None,
                        prepared_hash: None,
                        resource_key: Some(exclusive_resource_key(fixture, key)),
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
    store: &mut store::InMemoryTypedRunStore,
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

fn append_synthetic_side_effect_failed(
    store: &mut store::InMemoryTypedRunStore,
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
            payloads: vec![
                events::KernelEventPayload::SideEffectFailed(events::side_effect::Failed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    invocation_epoch: 1,
                    failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                    retryable: true,
                    error: side_effect_error(true),
                }),
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable: true,
                    error: side_effect_error(true),
                }),
            ],
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
        .expect("append synthetic side-effect failure");
}

fn append_synthetic_ambiguous(
    store: &mut store::InMemoryTypedRunStore,
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
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::SideEffectAmbiguous(events::side_effect::Ambiguous {
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
                }),
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable: false,
                    error: side_effect_error(false),
                }),
            ],
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

fn append_synthetic_completed_terminal(
    store: &mut store::InMemoryTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    commit_key: &str,
) {
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunCompleted(
                events::RunCompleted {
                    run_id: run_id.clone(),
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
                required_run_state: store::RequiredRunState::Started,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic terminal");
}

async fn append_manual_resolution(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
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
    let prefix = build_manual_resolution_prefix_authority(
        store,
        &fixture.runtime_spec,
        &fixture.run_id,
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
    scheduler
        .record_manual_resolution(
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
    store: &mut store::InMemoryTypedRunStore,
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
    store: &mut store::InMemoryTypedRunStore,
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
    store: &mut store::InMemoryTypedRunStore,
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
    store: &mut store::InMemoryTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
) {
    let projection =
        side_effect_projection_for_attempt(store.projection_snapshot(), node, attempt_id)
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

fn attempt_started_count(
    store: &store::InMemoryTypedRunStore,
    run_id: &RunId,
    node_id: &NodeId,
) -> usize {
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

fn assert_node_failed_with_code(
    store: &store::InMemoryTypedRunStore,
    node_id: &NodeId,
    code: &str,
) -> AttemptId {
    assert_node_failed_with_code_and_retryable(store, node_id, code, false)
}

fn assert_node_failed_with_code_and_retryable(
    store: &store::InMemoryTypedRunStore,
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

fn assert_failure_code_count(store: &store::InMemoryTypedRunStore, code: &str, expected: usize) {
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

fn fact_recorded_count(store: &store::InMemoryTypedRunStore) -> usize {
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
            DeterministicSideEffectRunner::new(fixture),
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

fn registered_two_side_effect_runners_with(
    fixture: &Fixture,
    runner: DeterministicSideEffectRunner,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "sidefx",
            runner.clone(),
        ))
        .expect("binding a");
    registry
        .register(binding(fixture.descriptor_b.clone(), "sidefx", runner))
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
            DeterministicSideEffectRunner::new(fixture),
        ))
        .expect("binding forward a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "sidefx",
            DeterministicSideEffectRunner::new(fixture),
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
            source_revision: events::SourceRevision::new("test-rev").expect("source"),
            cargo_package_name: events::PackageName::new("mfm-test").expect("package"),
            cargo_package_version: events::PackageVersion::new("0.1.0").expect("version"),
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

fn append_runtime_bootstrap_lifecycle_node(typed: &mut spec::TypedExecutionSpec) -> CellId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xe8; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xe9; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xea; 32]),
    );
    let config_ref =
        spec::framework_config_ref("bootstrap_run", &node_id).expect("bootstrap config ref");
    let input_binding =
        spec::framework_lifecycle_unit_input_binding("bootstrap_run").expect("input binding");
    let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
    let receipt_schema = spec::bootstrap_run_receipt_schema_id().expect("bootstrap receipt schema");
    let receipt_semantic =
        spec::bootstrap_run_receipt_semantic_type_id().expect("bootstrap receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "bootstrap_run",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xed; 32]),
    )
    .expect("state kind");
    let state_version =
        StateVersion::new("mfm.framework.state.bootstrap_run.v1").expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.bootstrap_run".to_owned(),
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
        scope_id: typed.scopes[0].scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xee),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/bootstrap-run").expect("stable key"),
        scope_id: typed.scopes[0].scope_id.clone(),
        state_kind,
        state_version,
        descriptor_id,
        config_ref,
        input_bindings: input_binding,
        output_cell: output_cell.clone(),
        effect_kind: managed.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::BootstrapRun(
            spec::BootstrapRunNodeSpec {},
        )),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    });
    output_cell
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
    let semantic = SemanticTypeId::new("mfm.test", "value", "1", DigestAlgorithm::Sha256JcsV1, D9)
        .expect("semantic");
    let value_schema =
        SchemaId::new("mfm.test.value", "1", DigestAlgorithm::Sha256JcsV1, DA).expect("schema");
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
    append_runtime_bootstrap_lifecycle_node(&mut spec);
    append_runtime_retention_lifecycle_node(&mut spec, render_cell.clone(), true);
    let retention_receipt = runtime_retention_receipt_cell(&spec);
    append_runtime_complete_lifecycle_node(&mut spec, retention_receipt, true);
    append_runtime_resolve_saga_terminal_lifecycle_node(&mut spec);
    let envelope = spec::HashedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
        .expect("envelope");
    let runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    Fixture {
        runtime_spec,
        run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, D5),
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
    store: &mut store::InMemoryTypedRunStore,
    fixture: &Fixture,
) {
    for _ in 0..8 {
        scheduler
            .drive_once(store, &fixture.runtime_spec, &fixture.run_id)
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
    fixture
}

fn fixture_with_independent_exclusive_side_effects() -> Fixture {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let descriptors = vec![fixture.descriptor_a.clone(), fixture.descriptor_b.clone()];
    with_exclusive_resource_claims(fixture, &descriptors)
}

fn fixture_with_independent_exclusive_side_effect_lanes() -> Fixture {
    let mut fixture = fixture_with_independent_exclusive_side_effects();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    for node in &mut envelope.spec.nodes {
        if node.descriptor_id == fixture.descriptor_b {
            let side_effect = node
                .side_effect
                .as_mut()
                .expect("descriptor b is a side-effect node");
            side_effect.resource_claim = spec::ResourceClaimSpec::Exclusive {
                namespace: independent_resource_namespace(),
                key_schema: fixture.seed_ref.schema_id.clone(),
            };
        }
    }
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
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
    fixture.descriptor_c = Some(descriptor_c);
    fixture.cell_c = Some(cell_c);
    fixture
}

fn fixture_with_two_exclusive_side_effects_and_failing_tail() -> Fixture {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let descriptors = vec![fixture.descriptor_a.clone(), fixture.descriptor_b.clone()];
    with_exclusive_resource_claims(fixture, &descriptors)
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
                evidence_schema: node.config_ref.schema_id.clone(),
            };
        }
    }
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
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

fn independent_resource_namespace() -> spec::ResourceNamespace {
    spec::ResourceNamespace::new("mfm.test.independent_wallet_nonce").expect("resource namespace")
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

fn resource_keys_for_all_side_effects(
    fixture: &Fixture,
    value: &str,
) -> BTreeMap<NodeId, events::ResourceKeyEvidence> {
    let evidence = exclusive_resource_key(fixture, value);
    fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(fixture.runtime_spec.spec().remediations.values())
        .filter(|node| node.side_effect.is_some())
        .map(|node| (node.node_id.clone(), evidence.clone()))
        .collect()
}

fn side_effect_runner_with_resource_keys(
    fixture: &Fixture,
    resource_keys: BTreeMap<NodeId, events::ResourceKeyEvidence>,
) -> DeterministicSideEffectRunner {
    DeterministicSideEffectRunner::new(fixture).with_resource_keys(resource_keys)
}

fn fixture_with_run_id(mut fixture: Fixture, digest: DigestBytes) -> Fixture {
    fixture.run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest);
    fixture
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

fn side_effect_projection_for_run_node<'a>(
    projections: &'a store::ProjectionSnapshot,
    run_id: &RunId,
    node_id: &NodeId,
) -> Option<&'a store::SideEffectProjection> {
    projections.side_effects().find_map(|(_, projection)| {
        (projection.run_id == *run_id && projection.intent.node_id == *node_id)
            .then_some(projection)
    })
}

async fn drive_until_side_effect_confirmation_without_output(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    output_cell: &CellId,
    context: &str,
) {
    for _ in 0..8 {
        assert_eq!(
            scheduler
                .drive_once(store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
        if side_effect_projection_for_run_node(
            store.projection_snapshot(),
            &fixture.run_id,
            &node.node_id,
        )
        .is_some_and(|projection| {
            matches!(
                projection.phase,
                store::SideEffectPhase::ConfirmationObserved { .. }
            ) && store
                .projection_snapshot()
                .cell_terminal(output_cell)
                .is_none()
        }) {
            return;
        }
    }
    panic!("{context} was not reached");
}

async fn drive_side_effect_to_confirmation(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
) {
    for _ in 0..8 {
        assert_eq!(
            scheduler
                .drive_once(store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive side effect to confirmation"),
            SchedulerStatus::Advanced
        );
        if side_effect_projection_for_run_node(
            store.projection_snapshot(),
            &fixture.run_id,
            &node.node_id,
        )
        .is_some_and(|projection| {
            matches!(
                projection.phase,
                store::SideEffectPhase::ConfirmationObserved { .. }
            )
        }) {
            return;
        }
    }
    panic!(
        "side-effect node {} did not reach confirmation",
        node.node_id
    );
}

async fn drive_until_cells_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
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
            scheduler
                .drive_once(store, &fixture.runtime_spec, &fixture.run_id)
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
    store: &mut store::InMemoryTypedRunStore,
    fixture: &Fixture,
    forward_ledger_key: &events::SideEffectLedgerKey,
    checkpoint: RemediationPhaseCheckpoint,
    context: &str,
) {
    for _ in 0..8 {
        assert_eq!(
            scheduler
                .drive_once(store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
        if remediation_projection_for_forward_ledger(
            store.projection_snapshot(),
            forward_ledger_key,
        )
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
        }) {
            return;
        }
    }
    panic!("{context} was not reached");
}

async fn drive_until_compensated_before_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
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
            scheduler
                .drive_once(store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive until compensated before terminal"),
            SchedulerStatus::Advanced
        );
    }
    panic!("compensated pre-terminal boundary was not reached");
}

fn remediation_intent_forward_links(
    store: &store::InMemoryTypedRunStore,
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

fn remediation_projection_for_forward_ledger<'a>(
    projections: &'a store::ProjectionSnapshot,
    forward_ledger_key: &events::SideEffectLedgerKey,
) -> Option<&'a store::SideEffectProjection> {
    projections.side_effects().find_map(|(_, projection)| {
        matches!(
            &projection.ledger_purpose,
            events::SideEffectLedgerPurpose::Remediation {
                forward_ledger_key: linked
            } if linked == forward_ledger_key
        )
        .then_some(projection)
    })
}

fn assert_no_duplicate_side_effect_submissions(
    store: &store::InMemoryTypedRunStore,
    run_id: &RunId,
) {
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
    store: &store::InMemoryTypedRunStore,
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
    store: &store::InMemoryTypedRunStore,
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

fn side_effect_claim_taken_over(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    previous: &store::SideEffectClaimProjection,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectClaimTakenOver(events::side_effect::ClaimTakenOver {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx),
        previous_claim_owner: previous.claim_owner.clone(),
        new_claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
        invocation_epoch: previous.invocation_epoch,
        previous_claim_generation: previous.claim_generation,
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
        prepared_artifact_id: None,
        prepared_hash: None,
        resource_key,
    })
}

fn side_effect_invocation_started(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectInvocationStarted(events::side_effect::InvocationStarted {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx),
        invocation_epoch,
        claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
    })
}

fn side_effect_submission_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectSubmissionObserved(events::side_effect::SubmissionObserved {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx),
        invocation_epoch,
        submission_schema_id: ctx.node().config_ref.schema_id.clone(),
        submission_hash: digest,
        submission_artifact_id: artifact_id,
    })
}

fn side_effect_receipt_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectReceiptObserved(events::side_effect::ReceiptObserved {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx),
        invocation_epoch,
        receipt_schema_id: ctx.node().config_ref.schema_id.clone(),
        receipt_hash: digest,
        receipt_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
        resource_touched_set: None,
    })
}

fn side_effect_confirmation_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectConfirmationObserved(events::side_effect::ConfirmationObserved {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx),
        invocation_epoch,
        confirmation_schema_id: ctx.node().config_ref.schema_id.clone(),
        confirmation_hash: digest,
        confirmation_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
        resource_touched_set: None,
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
