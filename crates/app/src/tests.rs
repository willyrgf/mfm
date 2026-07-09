use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;
use mfm_capabilities::{CapabilitySpec, ReadExternalRole};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, MfmFactType as _, NoContext,
    PublicOutputKey, ReadState, RootBuilder, ScopeKey, StateKey, StateRegistryBuilder, StateResult,
    StateSpec, TypedProgramLaunchPlan,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, PublicOutputs};
use mfm_store::v1::{ExecutionClaimStore as _, RunEventStore as _};
use serde::{Deserialize, Serialize};

#[test]
fn run_read_services_carry_explicit_fact_query_receipt_trust_root() {
    let store = store::AsyncInMemoryRunStore::new();
    let registry = CertificationRegistry::new();
    let key = SigningKey::from_bytes(&[11; 32]);
    let trust_root = store::FactQueryReceiptTrustRoot::new(
        mfm_facts::StoreIdentity::new("store.default").expect("store identity"),
        mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        mfm_facts::StoreKeyId::new("key.default").expect("key id"),
        key.verifying_key().to_bytes(),
    )
    .expect("trust root");

    let without_root = RunReadServices::new_with_certification_registry(
        store.clone(),
        store.clone(),
        registry.clone(),
    );
    assert!(without_root.fact_query_receipt_trust_root.is_none());

    let with_root = RunReadServices::new_with_certification_registry_and_fact_query_trust_root(
        store.clone(),
        store,
        registry,
        Some(trust_root.clone()),
    );
    assert_eq!(
        with_root.fact_query_receipt_trust_root.as_ref(),
        Some(&trust_root)
    );
}

#[test]
fn live_transport_runtime_caches_parsed_runtime_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("runtime.toml");
    std::fs::write(
        &config_path,
        r#"
        [btc.routes.public-bitcoin-core]
        rpc_url = "http://127.0.0.1:8332"
        "#,
    )
    .expect("write runtime config");
    let runtime = crate::live_transports::LiveTransportRuntime::new(
        crate::live_transports::RuntimeConfigLoader::from_path_or_env(Some(&config_path)),
    );

    assert!(runtime.btc_configured().expect("btc configured"));

    std::fs::write(&config_path, "not valid toml = [").expect("replace runtime config");

    assert!(
        runtime.btc_configured().expect("cached btc configured"),
        "live runtime must not reparse runtime config after first use"
    );
}

#[test]
fn live_transport_runtime_rejects_malformed_btc_runtime_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("runtime.toml");
    std::fs::write(&config_path, "not valid toml = [").expect("write runtime config");
    let runtime = crate::live_transports::LiveTransportRuntime::new(
        crate::live_transports::RuntimeConfigLoader::from_path_or_env(Some(&config_path)),
    );

    let error = runtime
        .btc_configured()
        .expect_err("malformed present config must not be treated as absent BTC config");

    assert!(
        matches!(error, mfm_runtime::RuntimeError::RunnerBinding(ref message) if message.contains("syntax")),
        "{error}"
    );
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.app.test",
    name = "fact_value",
    version = "1",
    schema = "mfm.app.test.fact_value"
)]
struct AppFactValue {
    amount: u64,
}

struct AppFactReadCap;

impl CapabilitySpec for AppFactReadCap {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<mfm_ids::CapabilityKind> {
        mfm_ids::CapabilityKind::new(
            "mfm.app.test",
            "fact-read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.app.test.fact-read"),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<mfm_ids::CapabilityVersion> {
        mfm_ids::CapabilityVersion::new("mfm.app.test.fact_read.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "fact-read"
    }
}

fn app_fact_adapter_kind() -> mfm_ids::AdapterKind {
    mfm_ids::AdapterKind::new(
        "mfm.app.test",
        "fact-adapter",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.app.test.fact-adapter"),
    )
    .expect("adapter kind")
}

fn app_fact_adapter_version() -> mfm_ids::AdapterVersion {
    mfm_ids::AdapterVersion::new("mfm.app.test.fact_adapter.v1").expect("adapter version")
}

#[allow(clippy::duplicated_attributes)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.app.test",
    name = "launch_fact",
    version = "1",
    schema = "mfm.app.test.launch_fact"
)]
#[mfm_fact(kind = "mfm.app.test.launch")]
#[mfm_fact(field(
    id = "subject.amount",
    source = "subject",
    path = "amount",
    value_type = "unsigned_integer",
    operator = "equal",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.amount",
    source = "result",
    path = "amount",
    value_type = "unsigned_integer",
    operators(equal, greater_than),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.amount.asc",
    term(field = "result.amount", direction = "ascending", nulls = "last")
))]
struct AppLaunchFact {
    subject: AppFactValue,
    response: AppFactValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct AppFactStateConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.app.test.public_outputs")]
struct AppFactPublicOutputs<'p, 's> {
    result: mfm_program::Handle<'p, 's, AppFactValue>,
}

struct AppFactState {
    config: AppFactStateConfig,
}

impl StateSpec for AppFactState {
    type Config = AppFactStateConfig;
    type Context = NoContext;
    type Input = AppFactValue;
    type Output = AppFactValue;
    type Effect = mfm_effects::ReadExternal;
    type Caps = (AppFactReadCap,);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        mfm_ids::StateKind::new(
            "mfm.app.test",
            "fact-state",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.app.test.fact-state"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        mfm_ids::StateVersion::new("mfm.app.test.fact_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.app.test.fact_state"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        Ok(vec![AdapterBindingSpec {
            adapter_kind: app_fact_adapter_kind(),
            adapter_version: app_fact_adapter_version(),
        }])
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<mfm_program::FactDescriptorRef>> {
        Ok(vec![mfm_program::fact_descriptor_ref::<AppLaunchFact>()?])
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for AppFactState {
    type RunFuture<'a> = std::future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        std::future::ready(Ok(AppFactValue {
            amount: input.amount * self.config.multiplier,
        }))
    }
}

fn app_fact_launch_plan_with_state_key(state_key: &str) -> TypedProgramLaunchPlan {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<AppFactState>()
        .expect("state registration");
    let seed = CanonicalSeed::from_value(&AppFactValue { amount: 7 }).expect("seed");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let input = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                seed.clone(),
            )?;
            let result = root.scope().state::<AppFactState, _>(
                StateKey::new(state_key)?,
                NoContext,
                AppFactStateConfig { multiplier: 2 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &AppFactPublicOutputs { result },
            )
        },
    )
    .expect("fact launch draft");
    let seeds = std::collections::BTreeMap::from([(
        draft.seeds()[0].seed_id.clone(),
        seed.canonical_json().clone(),
    )]);
    TypedProgramLaunchPlan::from_draft_and_seed_material(draft, seeds).expect("launch plan")
}

fn app_fact_certification_registry(include_fact_descriptor: bool) -> CertificationRegistry {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<AppFactState>()
        .expect("certification state registration");
    if include_fact_descriptor {
        registry
            .register_fact_type::<AppLaunchFact>()
            .expect("fact descriptor registration");
    }
    registry
}

struct AppFactRecordingRunner {
    visibility: mfm_program::facts::FactVisibility,
}

impl mfm_runtime::ErasedNodeRunner for AppFactRecordingRunner {
    fn run_erased<'a>(
        &'a self,
        ctx: mfm_runtime::ErasedRunCtx<'a>,
    ) -> mfm_runtime::ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let value = AppFactValue { amount: 14 };
            let artifacts = mfm_runtime::RunnerArtifactBuilder::new(&ctx);
            let payloads = mfm_runtime::RunnerPayloadBuilder::new(&ctx);
            let output_artifact = artifacts.state_output(&value)?;
            let output_payload = payloads.cell_produced(&output_artifact)?;
            let mut output = mfm_runtime::RunnerOutputBuilder::new(&ctx);
            output.stage_attempt_artifact(&output_artifact)?;
            output.record_fact(
                mfm_runtime::FactRecordInput::new(
                    AppLaunchFact {
                        subject: AppFactValue { amount: 7 },
                        response: AppFactValue { amount: 15 },
                    },
                    self.visibility.clone(),
                )
                .observed_at("2026-07-02T00:00:00Z"),
                app_fact_runner_capability_binding(),
            )?;
            output.payload(output_payload);
            Ok(output.finish())
        })
    }
}

fn app_fact_runner_capability_binding() -> mfm_runtime::RunnerCapabilityBinding {
    mfm_runtime::RunnerCapabilityBinding::for_capability::<AppFactReadCap>(
        app_fact_adapter_kind(),
        app_fact_adapter_version(),
    )
    .expect("runner capability binding")
}

fn app_launch_fact_query_request() -> PublicFactQueryRequest {
    PublicFactQueryRequest::from_selector(
        "mfm.app.test.launch",
        PublicFactQuerySelector {
            return_fields: vec!["subject.amount".to_owned(), "result.amount".to_owned()],
            ordering: Some("result.amount.asc".to_owned()),
            limit: Some(10),
            ..PublicFactQuerySelector::default()
        },
    )
    .expect("public fact request")
}

fn app_fact_runner_registry(
    runtime_spec: &CertifiedRuntimeSpec,
    visibility: mfm_program::facts::FactVisibility,
) -> ErasedRunnerRegistry {
    let node = runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("app fact state node");
    let descriptor = runtime_spec
        .state_descriptor_for_node(node)
        .expect("app fact state descriptor");
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.clone()).expect("factory id");
    let executable = events::ExecutableIdentity {
        factory_id: factory_id.clone(),
        cargo_package_digest: content_digest_for_bytes(b"mfm.app.test.fact-runner.cargo"),
        binary_digest: content_digest_for_bytes(b"mfm.app.test.fact-runner.binary"),
        nix_derivation_hash: None,
        nix_output_hash: None,
    };
    let mut registry = ErasedRunnerRegistry::new();
    mfm_runtime::RunnerRegistrationBuilder::new(
        &mut registry,
        mfm_runtime::CapabilityImplementationId::new("mfm.app.test.fact-runner")
            .expect("implementation id"),
    )
    .register_descriptor(
        node.descriptor_id.clone(),
        &node.capability_bindings,
        factory_id.clone(),
        executable,
        Arc::new(AppFactRecordingRunner { visibility }),
    )
    .expect("app fact runner registration")
    .register_adapter_executable(
        app_fact_adapter_kind(),
        app_fact_adapter_version(),
        events::ExecutableIdentity {
            factory_id,
            cargo_package_digest: content_digest_for_bytes(b"mfm.app.test.fact-adapter.cargo"),
            binary_digest: content_digest_for_bytes(b"mfm.app.test.fact-adapter.binary"),
            nix_derivation_hash: None,
            nix_output_hash: None,
        },
    )
    .expect("app fact runner registration");
    registry
}

fn prepare_app_fact_launch(include_fact_descriptor: bool) -> Result<RunLaunchRequest, AppError> {
    prepare_app_fact_launch_with_invocation_key(include_fact_descriptor, None)
}

fn prepare_btc_collector_internal_test_launch() -> Result<RunLaunchRequest, AppError> {
    let draft = mfm_op_btc_collectors::btc_chain_head_collector_cycle_program_draft(
        mfm_op_btc_collectors::BtcChainHeadCollectorConfig::default(),
    )
    .expect("btc collector draft");
    let observation_context_seed =
        CanonicalSeed::from_value(&mfm_op_btc_collectors::BtcChainHeadObservationContext {
            observed_at_unix_ms: None,
        })
        .expect("observation context seed");
    let seeds = BTreeMap::from([(
        draft.seeds()[0].seed_id.clone(),
        observation_context_seed.canonical_json().clone(),
    )]);
    let plan = TypedProgramLaunchPlan::from_draft_and_seed_material(draft, seeds)
        .expect("btc collector launch plan");
    let registry = production_certification_registry().expect("production registry");
    let lowered = mfm_certify::lower_program_draft(&plan.draft).expect("lowered btc collector");
    let scoped = registry
        .scoped_for_spec(lowered.spec())
        .expect("scoped production registry");
    let certified_spec =
        mfm_certify::certify_typed_spec(lowered, &scoped).expect("certified btc collector");
    let mut config_inputs = plan
        .config_material
        .into_iter()
        .map(|artifact| RunLaunchConfigArtifact {
            schema_id: artifact.schema_id,
            bytes: artifact.bytes.to_vec(),
            media_type: artifact.media_type,
        })
        .collect::<Vec<_>>();
    config_inputs.extend(framework_config_launch_artifacts_for_spec(
        &certified_spec.envelope().spec,
    )?);
    let seed_inputs = plan
        .seed_material
        .into_iter()
        .map(|artifact| RunLaunchSeedArtifact {
            seed_id: artifact.seed_id,
            bytes: artifact.bytes.to_vec(),
            media_type: artifact.media_type,
        })
        .collect();

    prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: &scoped,
            store_scope_id: StoreScopeId::new(
                "mfm.store_scope.v1:00000000000000000000000000000000",
            )
            .expect("store scope"),
            invocation_key_digest: default_invocation_key_digest(),
            entry_point_evidence: events::EntryPointLaunchEvidence {
                resolved_op_id: events::EntryPointOpId::new(
                    "mfm.bitcoin.btc_chain_head_collector_internal_test",
                )
                .expect("entry point"),
                entry_point_registry_digest: content_digest_for_bytes(
                    b"mfm.app.test.btc-collector-internal-registry",
                ),
            },
        },
        config_inputs,
        seed_inputs,
    )
}

fn prepare_app_fact_launch_with_invocation_key(
    include_fact_descriptor: bool,
    invocation_key_digest: Option<ContentDigest>,
) -> Result<RunLaunchRequest, AppError> {
    prepare_app_fact_launch_with_invocation_key_and_state_key(
        include_fact_descriptor,
        invocation_key_digest,
        "fact-state",
    )
}

fn prepare_app_fact_launch_with_invocation_key_and_state_key(
    include_fact_descriptor: bool,
    invocation_key_digest: Option<ContentDigest>,
    state_key: &str,
) -> Result<RunLaunchRequest, AppError> {
    let plan = app_fact_launch_plan_with_state_key(state_key);
    let registry = app_fact_certification_registry(include_fact_descriptor);
    let lowered = mfm_certify::lower_program_draft(&plan.draft).expect("lowered");
    let scoped = registry
        .scoped_for_spec(lowered.spec())
        .expect("scoped registry");
    let certified_spec = mfm_certify::certify_typed_spec(lowered, &scoped).expect("certified spec");
    let mut config_inputs = plan
        .config_material
        .into_iter()
        .map(|artifact| RunLaunchConfigArtifact {
            schema_id: artifact.schema_id,
            bytes: artifact.bytes.to_vec(),
            media_type: artifact.media_type,
        })
        .collect::<Vec<_>>();
    config_inputs.extend(framework_config_launch_artifacts_for_spec(
        &certified_spec.envelope().spec,
    )?);
    let seed_inputs = plan
        .seed_material
        .into_iter()
        .map(|artifact| RunLaunchSeedArtifact {
            seed_id: artifact.seed_id,
            bytes: artifact.bytes.to_vec(),
            media_type: artifact.media_type,
        })
        .collect();

    prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: &scoped,
            store_scope_id: StoreScopeId::new(
                "mfm.store_scope.v1:00000000000000000000000000000000",
            )
            .expect("store scope"),
            invocation_key_digest: invocation_key_digest
                .unwrap_or_else(default_invocation_key_digest),
            entry_point_evidence: events::EntryPointLaunchEvidence {
                resolved_op_id: events::EntryPointOpId::new("mfm.app.test.fact-launch")
                    .expect("entry point"),
                entry_point_registry_digest: content_digest_for_bytes(b"mfm.app.test.registry"),
            },
        },
        config_inputs,
        seed_inputs,
    )
}

fn default_invocation_key_digest() -> ContentDigest {
    content_digest_for_bytes(b"mfm.app.test.default-invocation")
}

async fn launch_app_fact_run() -> (RunId, store::AsyncInMemoryRunStore, CertificationRegistry) {
    launch_app_fact_run_with_visibility(mfm_program::facts::FactVisibility::indexed_default(
        mfm_program::facts::FactAudience::Platform,
    ))
    .await
}

async fn launch_app_fact_run_with_visibility(
    visibility: mfm_program::facts::FactVisibility,
) -> (RunId, store::AsyncInMemoryRunStore, CertificationRegistry) {
    let store = store::AsyncInMemoryRunStore::default();
    launch_app_fact_run_in_store_with_visibility(store, visibility, None).await
}

async fn launch_app_fact_run_in_store_with_visibility(
    store: store::AsyncInMemoryRunStore,
    visibility: mfm_program::facts::FactVisibility,
    invocation_key_digest: Option<ContentDigest>,
) -> (RunId, store::AsyncInMemoryRunStore, CertificationRegistry) {
    launch_app_fact_run_in_store_with_visibility_and_state_key(
        store,
        visibility,
        invocation_key_digest,
        "fact-state",
        true,
    )
    .await
}

async fn launch_app_fact_run_in_store_with_visibility_and_state_key(
    store: store::AsyncInMemoryRunStore,
    visibility: mfm_program::facts::FactVisibility,
    invocation_key_digest: Option<ContentDigest>,
    state_key: &str,
    require_completion: bool,
) -> (RunId, store::AsyncInMemoryRunStore, CertificationRegistry) {
    let request = prepare_app_fact_launch_with_invocation_key_and_state_key(
        true,
        invocation_key_digest,
        state_key,
    )
    .expect("prepared app fact launch");
    let runtime_spec =
        CertifiedRuntimeSpec::new(request.certified_spec.clone()).expect("runtime spec");
    let runners = app_fact_runner_registry(&runtime_spec, visibility);
    let registry = app_fact_certification_registry(true);
    let run_id = request.run_id.clone();
    let execution_scope =
        store::ExecutionClaimScope::from_run_identity_material(&request.identity_material);
    let scheduler = SerialTypedScheduler::new(runners, Arc::new(store.clone()));
    let launch = scheduler
        .prepare_run_launch(
            &runtime_spec,
            request.identity_material,
            request.evidence,
            store.expected_next_seq(&run_id).await.expect("next seq"),
        )
        .expect("prepare fact run launch");
    scheduler
        .start_run(&store, launch)
        .await
        .expect("start fact run");
    let token = store::AdmissionToken::new("mfm.app.test.fact-runner-claim")
        .expect("execution claim token");
    match store
        .acquire_execution_claim(&execution_scope, &run_id, token.clone())
        .await
        .expect("acquire execution claim")
    {
        store::NowaitSkipAdmissionResult::Admitted(_) => {}
        store::NowaitSkipAdmissionResult::Busy(_) => panic!("fact run execution claim busy"),
    }

    for _ in 0..16 {
        let committed = store
            .load_committed_run_stream(&run_id)
            .await
            .expect("committed stream");
        if committed.projection().run_state(&run_id) == store::RunState::Completed {
            break;
        }
        if !require_completion
            && committed
                .projection()
                .fact_records()
                .any(|(_claim_id, record)| record.source_run_id == run_id)
        {
            break;
        }
        match scheduler
            .drive_once(
                &store,
                &runtime_spec,
                &run_id,
                &execution_scope,
                token.clone(),
            )
            .await
        {
            Ok(_) => {}
            Err(error) => {
                let stream = store.load_run_stream(&run_id).await.unwrap_or_default();
                let event_variants = stream
                    .iter()
                    .map(|event| match event.payload() {
                        events::KernelEventPayload::RunAdmitted(_) => "RunAdmitted".to_owned(),
                        events::KernelEventPayload::StateAttemptStarted(_) => {
                            "StateAttemptStarted".to_owned()
                        }
                        events::KernelEventPayload::StateAttemptFailed(payload) => format!(
                            "StateAttemptFailed:{}:{}",
                            payload.error.code, payload.error.safe_message
                        ),
                        events::KernelEventPayload::CellProduced(_) => "CellProduced".to_owned(),
                        events::KernelEventPayload::FactRecorded(_) => "FactRecorded".to_owned(),
                        events::KernelEventPayload::ArtifactReferenced(_) => {
                            "ArtifactReferenced".to_owned()
                        }
                        events::KernelEventPayload::RetentionRefsAppended(_) => {
                            "RetentionRefsAppended".to_owned()
                        }
                        events::KernelEventPayload::RunCompleted(_) => "RunCompleted".to_owned(),
                        _ => "Other".to_owned(),
                    })
                    .collect::<Vec<_>>();
                panic!("drive fact run failed: {error:?}; events: {event_variants:?}");
            }
        }
    }
    let committed = store
        .load_committed_run_stream(&run_id)
        .await
        .expect("fact committed stream");
    if require_completion {
        assert_eq!(
            committed.projection().run_state(&run_id),
            store::RunState::Completed
        );
    }
    let event_variants = committed
        .events()
        .iter()
        .map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(_) => "RunAdmitted".to_owned(),
            events::KernelEventPayload::StateAttemptStarted(_) => "StateAttemptStarted".to_owned(),
            events::KernelEventPayload::StateAttemptFailed(payload) => format!(
                "StateAttemptFailed:{}:{}",
                payload.error.code, payload.error.safe_message
            ),
            events::KernelEventPayload::CellProduced(_) => "CellProduced".to_owned(),
            events::KernelEventPayload::FactRecorded(_) => "FactRecorded".to_owned(),
            events::KernelEventPayload::ArtifactReferenced(_) => "ArtifactReferenced".to_owned(),
            events::KernelEventPayload::RetentionRefsAppended(_) => {
                "RetentionRefsAppended".to_owned()
            }
            events::KernelEventPayload::PublicOutputProduced(_) => {
                "PublicOutputProduced".to_owned()
            }
            events::KernelEventPayload::RunCompleted(_) => "RunCompleted".to_owned(),
            _ => "Other".to_owned(),
        })
        .collect::<Vec<_>>();
    assert!(
        committed
            .events()
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::FactRecorded(_))),
        "events: {event_variants:?}"
    );

    (run_id, store, registry)
}

#[derive(Clone, Copy, Debug)]
enum CommittedStreamArtifactMode {
    Missing,
    TamperFactResponse,
}

#[derive(Clone)]
struct OverriddenCommittedStreamStore {
    inner: store::AsyncInMemoryRunStore,
    mode: CommittedStreamArtifactMode,
}

impl OverriddenCommittedStreamStore {
    fn new(inner: store::AsyncInMemoryRunStore, mode: CommittedStreamArtifactMode) -> Self {
        Self { inner, mode }
    }
}

impl store::RunEventStore for OverriddenCommittedStreamStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        self.inner.append_prepared_commit_bundle(bundle)
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        Box::pin(async move {
            let committed = self.inner.load_committed_run_stream(run_id).await?;
            let mut artifact_bytes = committed.artifact_byte_authority().clone();
            match self.mode {
                CommittedStreamArtifactMode::Missing => artifact_bytes.clear(),
                CommittedStreamArtifactMode::TamperFactResponse => {
                    let (_, (bytes, _)) = artifact_bytes
                        .iter_mut()
                        .find(|(_, (_, evidence))| {
                            evidence.artifact_role == events::ArtifactRole::FactResponse
                        })
                        .expect("fact response artifact bytes");
                    bytes.push(b'\n');
                }
            }
            store::CommittedRunStream::from_events_with_artifact_bytes(
                run_id.clone(),
                committed.events().to_vec(),
                &artifact_bytes,
            )
        })
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

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.fact_projection_snapshot()
    }
}

impl store::StoreScopeStore for OverriddenCommittedStreamStore {
    type Error = store::StoreError;

    fn load_store_scope_id<'a>(&'a self) -> store::AsyncStoreFuture<'a, StoreScopeId, Self::Error> {
        self.inner.load_store_scope_id()
    }
}

impl store::RetainedArtifactReadProvider for OverriddenCommittedStreamStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        self.inner.read_retained_artifact(requirement)
    }
}

#[derive(Clone)]
struct ExpectingRetainedArtifactProvider {
    expected: store::EventArtifactRequirement,
    artifact: store::VerifiedRunArtifactBytes,
    seen: Arc<Mutex<Vec<store::EventArtifactRequirement>>>,
}

impl store::RetainedArtifactReadProvider for ExpectingRetainedArtifactProvider {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        let result = if requirement == &self.expected {
            self.seen
                .lock()
                .expect("seen lock")
                .push(requirement.clone());
            Ok(self.artifact.clone())
        } else {
            Err(store::StoreError::ArtifactEvidenceMismatch {
                artifact_id: requirement.artifact_id.clone(),
                field: "requirement",
            })
        };
        Box::pin(std::future::ready(result))
    }
}

#[tokio::test]
async fn retained_artifact_adapter_preserves_read_request_expectations() {
    let bytes = br#"{"answer":42}"#.to_vec();
    let digest = content_digest_for_bytes(&bytes);
    let schema_id = SchemaId::new(
        "mfm.test.config",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.config"),
    )
    .expect("schema id");
    let config_ref = spec::ConfigRef {
        schema_id: schema_id.clone(),
        artifact_id: artifact_id_for_digest(&digest),
        digest: digest.clone(),
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
    };
    let expected = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: config_ref.artifact_id.clone(),
        digest: Some(config_ref.digest.clone()),
        byte_len: Some(config_ref.byte_len),
        media_type: Some(config_ref.media_type.clone()),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: config_ref.artifact_id.clone(),
        digest,
        byte_len: bytes.len() as u64,
        media_type: config_ref.media_type.clone(),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let artifact = store::VerifiedRunArtifactBytes::new(bytes, evidence, &expected)
        .expect("verified artifact");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = artifact_read_provider_from_retained(ExpectingRetainedArtifactProvider {
        expected: expected.clone(),
        artifact,
        seen: Arc::clone(&seen),
    });

    provider
        .read_artifact(
            &mfm_artifact_capabilities::ArtifactReadRequest::from_certified_config_ref(&config_ref),
        )
        .await
        .expect("adapter preserves exact config expectation");

    assert_eq!(*seen.lock().expect("seen lock"), vec![expected]);
}

#[tokio::test]
async fn app_read_services_reconstruct_fact_bearing_status_from_committed_stream() {
    let (run_id, store, registry) = launch_app_fact_run().await;
    let read_services =
        make_run_read_services_with_certification_registry(store.clone(), store, registry);

    let status = read_services
        .run_status(&run_id)
        .await
        .expect("fact-bearing status uses committed artifact authority");
    assert_eq!(status.run_mode, RunModeStatus::Completed);
    assert!(status.head_seq > store::StreamSeq::FIRST.as_u64());

    let stream = read_services
        .run_stream(&run_id)
        .await
        .expect("fact-bearing stream uses committed artifact authority");
    assert_eq!(stream.run_id, run_id.as_str());
    assert!(stream.events.len() > 1);
}

#[tokio::test]
async fn app_read_services_fail_closed_for_missing_or_tampered_fact_artifacts() {
    let (run_id, store, registry) = launch_app_fact_run().await;

    for mode in [
        CommittedStreamArtifactMode::Missing,
        CommittedStreamArtifactMode::TamperFactResponse,
    ] {
        let overridden = OverriddenCommittedStreamStore::new(store.clone(), mode);
        let read_services = make_run_read_services_with_certification_registry(
            overridden.clone(),
            overridden,
            registry.clone(),
        );

        let status_error = read_services
            .run_status(&run_id)
            .await
            .expect_err("fact artifact authority failure must close status reads");
        assert_eq!(status_error.code, "RunStoreRejected", "mode {mode:?}");

        let stream_error = read_services
            .run_stream(&run_id)
            .await
            .expect_err("fact artifact authority failure must close stream reads");
        assert_eq!(stream_error.code, "RunStoreRejected", "mode {mode:?}");
    }
}

#[tokio::test]
async fn public_fact_catalog_discovers_only_platform_descriptors() {
    let (_run_id, store, _registry) = launch_app_fact_run().await;
    let descriptor = AppLaunchFact::descriptor().expect("fact descriptor");
    let projection = store.projection_snapshot().expect("projection snapshot");
    let public_catalog =
        FactCatalogService::from_public_projection(vec![descriptor.clone()], &projection)
            .expect("public catalog");

    assert_eq!(
        public_catalog.list_kinds(),
        vec![PublicFactKindSummary {
            fact_kind: "mfm.app.test.launch".to_owned(),
            descriptor_count: 1,
        }]
    );

    let (_claim_id, platform_entry) = projection
        .fact_index_entries()
        .next()
        .expect("platform fact index entry");
    let mut control_entry = platform_entry.clone();
    control_entry.audience = mfm_facts::FactAudience::Control;
    let control_projection =
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_index_entries: BTreeMap::from([(
                control_entry.fact_claim_id.clone(),
                control_entry,
            )]),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("control projection");
    let control_catalog =
        FactCatalogService::from_public_projection(vec![descriptor], &control_projection)
            .expect("control catalog");

    assert!(control_catalog.list_kinds().is_empty());
    let error = control_catalog
        .describe_kind("mfm.app.test.launch")
        .expect_err("control-only descriptors are not publicly discoverable");
    assert_eq!(error.class, ErrorClass::NotFound);
    assert_eq!(error.code, "FactNotFound");

    let (_record_claim_id, platform_record) = projection
        .fact_records()
        .next()
        .expect("recorded fact projection");
    let run_private_claim = mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::RunPrivate,
        fact_kind: platform_record.claim.fact_kind().clone(),
        fact_descriptor_hash: platform_record.claim.fact_descriptor_hash().clone(),
        subject: platform_record.claim.subject().clone(),
        observed_at: platform_record.claim.observed_at().map(str::to_owned),
        request: platform_record.claim.request().cloned(),
        response: platform_record.claim.response().clone(),
        producer: platform_record.claim.producer().clone(),
    })
    .expect("run-private claim");
    let mut run_private_record = platform_record.clone();
    run_private_record.claim = run_private_claim;
    let run_private_projection =
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_records: BTreeMap::from([(
                run_private_record.fact_claim_id.clone(),
                run_private_record,
            )]),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("run-private projection");
    let run_private_catalog = FactCatalogService::from_public_projection(
        vec![AppLaunchFact::descriptor().expect("fact descriptor")],
        &run_private_projection,
    )
    .expect("run-private catalog");
    assert!(run_private_catalog.list_kinds().is_empty());
}

#[tokio::test]
async fn run_read_services_load_public_fact_catalog_from_retained_projection_authority() {
    let (_run_id, store, registry) = launch_app_fact_run().await;
    let services =
        make_run_read_services_with_certification_registry(store.clone(), store.clone(), registry);

    assert_eq!(
        services.fact_kinds().await.expect("fact kinds"),
        vec![PublicFactKindSummary {
            fact_kind: "mfm.app.test.launch".to_owned(),
            descriptor_count: 1,
        }]
    );
    let described = services
        .describe_fact_kind("mfm.app.test.launch")
        .await
        .expect("fact description");
    assert_eq!(described.len(), 1);
    assert_eq!(described[0].fields.len(), 2);
    let explained = services
        .explain_fact_kind("mfm.app.test.launch")
        .await
        .expect("fact explanation");
    assert_eq!(explained.descriptors, described);

    let projection = store.projection_snapshot().expect("projection snapshot");
    let (_claim_id, entry) = projection
        .fact_index_entries()
        .next()
        .expect("platform fact index entry");
    let public_ref =
        public_ref_id(&internal_fact_ref_for_entry(entry).expect("platform internal ref"))
            .expect("public ref id");
    let resolved = services
        .resolve_public_fact_ref(&public_ref)
        .await
        .expect("resolve public ref");
    assert_eq!(resolved.fact_kind, "mfm.app.test.launch");
    assert_eq!(resolved.fields.len(), 2);

    let rendered = serde_json::to_string(&resolved).expect("public fact JSON");
    assert_public_fact_json_redacts_private_tokens_for_test(&rendered, std::iter::empty::<&str>());
}

#[tokio::test]
async fn run_read_services_public_fact_reads_are_store_scoped_across_runs() {
    let store = store::AsyncInMemoryRunStore::default();
    let (first_run_id, store, _registry) = launch_app_fact_run_in_store_with_visibility(
        store,
        mfm_program::facts::FactVisibility::indexed_default(
            mfm_program::facts::FactAudience::Platform,
        ),
        Some(content_digest_for_bytes(
            b"mfm.app.test.first-public-fact-run",
        )),
    )
    .await;
    let (second_run_id, store, registry) =
        launch_app_fact_run_in_store_with_visibility_and_state_key(
            store,
            mfm_program::facts::FactVisibility::indexed_default(
                mfm_program::facts::FactAudience::Platform,
            ),
            Some(content_digest_for_bytes(
                b"mfm.app.test.second-public-fact-run",
            )),
            "fact-state-second",
            false,
        )
        .await;
    assert_ne!(first_run_id, second_run_id);
    let services =
        make_run_read_services_with_certification_registry(store.clone(), store.clone(), registry);

    assert_eq!(
        services.fact_kinds().await.expect("fact kinds"),
        vec![PublicFactKindSummary {
            fact_kind: "mfm.app.test.launch".to_owned(),
            descriptor_count: 1,
        }]
    );

    let page = services
        .query_public_facts(app_launch_fact_query_request())
        .await
        .expect("public fact query");

    assert_eq!(page.facts.len(), 2);
    let public_refs = page
        .facts
        .iter()
        .map(|fact| fact.public_ref.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(public_refs.len(), 2);
    for public_ref in public_refs {
        let resolved = services
            .resolve_public_fact_ref(&public_ref)
            .await
            .expect("resolve public ref");
        assert_eq!(resolved.fact_kind, "mfm.app.test.launch");
        assert_eq!(resolved.fields.len(), 2);
    }
}

#[tokio::test]
async fn run_read_services_do_not_disclose_non_public_facts() {
    for (case, visibility, has_index_entry) in [
        (
            "control",
            mfm_program::facts::FactVisibility::indexed_default(
                mfm_program::facts::FactAudience::Control,
            ),
            true,
        ),
        (
            "run-private",
            mfm_program::facts::FactVisibility::RunPrivate,
            false,
        ),
    ] {
        let (_run_id, store, registry) = launch_app_fact_run_with_visibility(visibility).await;
        let services = make_run_read_services_with_certification_registry(
            store.clone(),
            store.clone(),
            registry,
        );

        assert!(
            services
                .fact_kinds()
                .await
                .expect("non-public fact kinds")
                .is_empty(),
            "{case} facts should not be listed"
        );
        let error = services
            .describe_fact_kind("mfm.app.test.launch")
            .await
            .expect_err("non-public descriptors are not public");
        assert_eq!(error.class, ErrorClass::NotFound, "{case}");
        assert_eq!(error.code, "FactNotFound", "{case}");
        let error = services
            .explain_fact_kind("mfm.app.test.launch")
            .await
            .expect_err("non-public descriptors are not explainable");
        assert_eq!(error.class, ErrorClass::NotFound, "{case}");
        assert_eq!(error.code, "FactNotFound", "{case}");

        let projection = store.projection_snapshot().expect("projection snapshot");
        if has_index_entry {
            let (_claim_id, entry) = projection
                .fact_index_entries()
                .next()
                .expect("control fact index entry");
            let public_ref =
                public_ref_id(&internal_fact_ref_for_entry(entry).expect("control internal ref"))
                    .expect("control public ref-shaped id");
            let error = services
                .resolve_public_fact_ref(&public_ref)
                .await
                .expect_err("non-public refs resolve as not found");
            assert_eq!(error.class, ErrorClass::NotFound, "{case}");
            assert_eq!(error.code, "FactNotFound", "{case}");
        } else {
            assert!(
                projection.fact_records().next().is_some(),
                "{case} fact should still be retained internally"
            );
            assert!(
                projection.fact_index_entries().next().is_none(),
                "{case} fact should not be publicly indexed"
            );
        }
    }
}

#[derive(Clone)]
struct FakeFactQueryExecutor {
    rows: Vec<AppFactQueryRow>,
}

impl PublicFactQueryExecutor for FakeFactQueryExecutor {
    fn execute_public_fact_query_plan<'a>(
        &'a self,
        _plan: &'a mfm_facts::CanonicalFactQueryPlan,
    ) -> PublicFactQueryFuture<'a> {
        let rows = self.rows.clone();
        Box::pin(async move { Ok(rows) })
    }
}

#[tokio::test]
async fn public_fact_query_filters_non_public_refs_and_redacts_internal_fields() {
    let (_run_id, store, _registry) = launch_app_fact_run().await;
    let descriptor = AppLaunchFact::descriptor().expect("fact descriptor");
    let projection = store.projection_snapshot().expect("projection snapshot");
    let catalog = FactCatalogService::from_public_projection(vec![descriptor], &projection)
        .expect("public catalog");
    let (_claim_id, platform_entry) = projection
        .fact_index_entries()
        .next()
        .expect("platform fact index entry");
    let platform_ref = internal_fact_ref_for_entry(platform_entry).expect("platform internal ref");
    let platform_fields = returned_fields_for_entry(&projection, platform_entry);
    let mut control_entry = platform_entry.clone();
    control_entry.audience = mfm_facts::FactAudience::Control;
    let control_ref = internal_fact_ref_for_entry(&control_entry).expect("control ref");
    let executor = FakeFactQueryExecutor {
        rows: vec![
            AppFactQueryRow::new(platform_ref, platform_fields.clone()),
            AppFactQueryRow::new(control_ref, platform_fields),
        ],
    };

    let page = query_public_facts(
        &catalog,
        &executor,
        &mfm_facts::StoreScopeRef::new("mfm.store.default").expect("store scope"),
        &mfm_facts::ScopeDecisionEvidence::new(content_digest_for_bytes(
            b"mfm.public-facts.default-scope.v1",
        )),
        app_launch_fact_query_request(),
    )
    .await
    .expect("public fact query");

    assert_eq!(page.facts.len(), 1);
    assert_eq!(page.facts[0].fact_kind, "mfm.app.test.launch");
    assert_eq!(page.facts[0].fields.len(), 2);
    assert!(page.facts[0]
        .fields
        .iter()
        .any(|field| field.field_id == "result.amount"
            && field.value == PublicFactScalarValue::UnsignedInteger(15)));

    let rendered = serde_json::to_string(&page).expect("public page JSON");
    assert_public_fact_json_redacts_private_tokens_for_test(
        &rendered,
        [
            platform_entry.artifact_id.as_str(),
            platform_entry.artifact_evidence_hash.as_str(),
            platform_entry.fact_descriptor_hash.as_str(),
            platform_entry.subject_material_hash.as_str(),
        ],
    );
}

fn returned_fields_for_entry(
    projection: &store::ProjectionSnapshot,
    entry: &store::FactIndexProjection,
) -> Vec<mfm_facts::FactFieldValue> {
    projection
        .fact_term_entries()
        .filter(|((claim_id, _field_id), _term)| claim_id == &entry.fact_claim_id)
        .filter(|((_claim_id, field_id), _term)| {
            ["subject.amount", "result.amount"].contains(&field_id.as_str())
        })
        .map(|((_claim_id, _field_id), term)| {
            mfm_facts::FactFieldValue::new(
                term.field_id.clone(),
                term.value_type,
                term.value.clone(),
            )
            .expect("returned field")
        })
        .collect()
}

fn internal_fact_ref_for_entry(
    entry: &store::FactIndexProjection,
) -> Result<mfm_facts::InternalFactRef, AppError> {
    entry.internal_ref().map_err(AppError::from)
}

#[test]
fn certified_launch_stages_fact_descriptor_artifacts() {
    let request = prepare_app_fact_launch(true).expect("prepared launch");
    let descriptor = AppLaunchFact::descriptor().expect("fact descriptor");
    let canonical =
        mfm_program::facts::canonical_fact_descriptor_bytes(&descriptor).expect("canonical");
    let descriptor_hash =
        mfm_program::facts::fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let descriptor_schema =
        mfm_program::facts::fact_descriptor_schema_id().expect("descriptor schema");

    assert_eq!(request.evidence.fact_descriptor_artifacts.len(), 1);
    let artifact = &request.evidence.fact_descriptor_artifacts[0];
    assert_eq!(artifact.bytes, canonical.as_bytes());
    assert_eq!(artifact.evidence.digest, descriptor_hash);
    assert_eq!(
        artifact.evidence.artifact_id,
        artifact_id_for_digest(&descriptor_hash)
    );
    assert_eq!(
        artifact.evidence.byte_len,
        canonical.as_bytes().len() as u64
    );
    assert_eq!(
        artifact.evidence.artifact_role,
        events::ArtifactRole::FactDescriptor
    );
    assert_eq!(
        artifact.evidence.schema_id.as_ref(),
        Some(&descriptor_schema)
    );
    assert!(artifact.evidence.semantic_type_id.is_none());
    assert!(artifact.evidence.producer_node_id.is_none());
    assert!(artifact.evidence.producer_seed_id.is_none());
}

#[test]
fn certified_launch_rejects_missing_fact_descriptor_artifacts() {
    let error =
        prepare_app_fact_launch(false).expect_err("hash-only fact descriptor refs must not launch");

    assert_eq!(error.class, ErrorClass::Internal);
    assert_eq!(error.code, "FactDescriptorArtifactMissing");
}

#[test]
fn resource_key_status_redacts_raw_key() {
    let raw_key = "0x000000000000000000000000000000000000dead";
    let evidence = events::ResourceKeyEvidence {
        namespace: spec::ResourceNamespace::new("mfm.test.account_nonce")
            .expect("resource namespace"),
        key_schema_id: SchemaId::new(
            "mfm.test.account_nonce.resource_key",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.account_nonce.resource_key"),
        )
        .expect("schema id"),
        key: events::ResourceKey::new(raw_key).expect("resource key"),
    };

    let status = resource_key_status(&evidence);
    let rendered = serde_json::to_string(&status).expect("status JSON");

    assert_eq!(status.namespace, "mfm.test.account_nonce");
    assert_ne!(status.key_digest, raw_key);
    assert!(rendered.contains("key_digest"));
    assert!(!rendered.contains(raw_key));
    assert!(!rendered.contains("\"key\""));
}

#[test]
fn replay_diagnostic_rejects_digest_matched_malformed_evm_chain_mismatch_details() {
    for details in [
        serde_json::json!({
            "network_id": "bad network id",
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": "primary",
            "policy_id": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 0,
            "observed_chain_id": 31338,
            "source_ref": "primary",
            "policy_id": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31337,
            "source_ref": "primary",
            "policy_id": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": "primary",
            "policy_id": "primary",
            "unexpected": true,
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": "bad source ref",
            "policy_id": "primary",
        }),
    ] {
        let digest = canonical_value_digest(&details).expect("details digest");
        let expected = events::RedactedJson::new(digest);
        let error = verify_replay_public_details(Some(&expected), &details)
            .expect_err("digest-matched malformed public details must fail replay");

        assert_eq!(error.code, "ReplayDiagnosticInvalid");
    }
}

#[tokio::test]
async fn run_read_services_are_evidence_only() {
    let source = include_str!("lib.rs");
    let production_read_constructor = source
        .split("pub async fn connect_production_run_read_services")
        .nth(1)
        .expect("production read constructor is present")
        .split("/// Builds the production typed runner registry")
        .next()
        .expect("production read constructor is bounded");
    assert!(!production_read_constructor.contains("production_runner_registry"));
    assert!(!production_read_constructor.contains("std::env"));

    let read_services_impl = source
        .split("pub struct RunReadServices")
        .nth(1)
        .expect("read services are present")
        .split("/// Application facade for certified typed runtime dispatch.")
        .next()
        .expect("read services implementation is bounded");
    assert!(!read_services_impl.contains("production_runner_registry"));
    assert!(!read_services_impl.contains("std::env"));

    let store = store::AsyncInMemoryRunStore::default();
    let services = make_run_read_services_with_certification_registry(
        store.clone(),
        store,
        production_certification_registry().expect("cert registry"),
    );
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .expect("run id");

    let status = services
        .run_status(&run_id)
        .await
        .expect_err("missing run should come from store evidence");
    assert_eq!(status.code, "RunNotFound");

    let observations = services
        .read_run_observations(store::RunObservationQuery::new(None, 50, 0))
        .await
        .expect("list observations does not construct live drivers");
    assert!(observations.runs.is_empty());
}

#[test]
fn production_registry_certifies_btc_collector_descriptors() {
    let request =
        prepare_btc_collector_internal_test_launch().expect("btc collector certifies and prepares");

    assert_eq!(
        request.evidence.entry_point.resolved_op_id.as_str(),
        "mfm.bitcoin.btc_chain_head_collector_internal_test"
    );
    assert!(!request.evidence.config_artifacts.is_empty());
    assert!(!request.evidence.seed_cells.is_empty());
}

#[tokio::test]
async fn btc_collector_launch_requires_runtime_config_before_admission() {
    let store = store::AsyncInMemoryRunStore::default();
    let request =
        prepare_btc_collector_internal_test_launch().expect("btc collector launch request");
    let run_id = request.run_id.clone();
    let runners = production_runner_registry(
        Arc::new(store.clone()),
        crate::unit_test_fact_index_provider(),
        None,
    )
    .expect("production runners without btc config");
    let services = make_run_services_with_certification_registry(
        runners,
        store.clone(),
        store.clone(),
        production_certification_registry().expect("production registry"),
    );

    let error = services
        .launch_run(request)
        .await
        .expect_err("missing btc runtime config rejects before admission");

    assert_eq!(error.code, "LaunchRunnerUnavailable");
    assert!(store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .is_empty());
}

#[tokio::test]
async fn launch_run_reaps_expired_execution_claim_and_retries_admission() {
    let store = store::AsyncInMemoryRunStore::default();
    let request = prepare_app_fact_launch(true).expect("prepared launch");
    let runtime_spec =
        CertifiedRuntimeSpec::new(request.certified_spec.clone()).expect("runtime spec");
    let runners = app_fact_runner_registry(
        &runtime_spec,
        mfm_program::facts::FactVisibility::indexed_default(
            mfm_program::facts::FactAudience::Platform,
        ),
    );
    let services = make_run_services_with_certification_registry(
        runners,
        store.clone(),
        store.clone(),
        app_fact_certification_registry(true),
    );
    let execution_scope =
        store::ExecutionClaimScope::from_run_identity_material(&request.identity_material);
    let stale_token =
        store::AdmissionToken::new("mfm.app.test.expired-launch-claim").expect("token");
    match store
        .acquire_execution_claim(&execution_scope, &request.run_id, stale_token)
        .await
        .expect("pre-acquire execution claim")
    {
        store::NowaitSkipAdmissionResult::Admitted(_) => {}
        store::NowaitSkipAdmissionResult::Busy(_) => panic!("test execution claim already busy"),
    }
    assert!(
        store
            .expire_execution_claim_for_test(&request.run_id)
            .expect("expire execution claim"),
        "pre-acquired claim should exist"
    );
    let run_id = request.run_id.clone();

    let launch = services
        .launch_run(request)
        .await
        .expect("launch retries after expired claim");

    assert!(matches!(launch, RunLaunchOutcome::Admitted { .. }));
    assert!(store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .iter()
        .any(|event| matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_))));
    assert!(matches!(
        store
            .execution_claim_status(&execution_scope)
            .await
            .expect("execution claim status"),
        store::ExecutionClaimStatus::Unclaimed
    ));
}

#[tokio::test]
async fn postgres_store_authority_error_is_redacted_for_public_app_surface() {
    let database_url = "postgres://mfm_user:super-secret@127.0.0.1:notaport/mfm";
    let error = match connect_production_run_store(Some(database_url)).await {
        Ok(_) => panic!("invalid postgres URL should not connect"),
        Err(error) => error,
    };

    assert_eq!(error.code, "RunStoreAuthorityInvalid");
    assert_eq!(error.message, "Run store authority could not be validated");
    let rendered = format!("{error:?}\n{error}");
    for forbidden in [
        database_url,
        "mfm_user",
        "super-secret",
        "127.0.0.1",
        "notaport",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "app error leaked `{forbidden}` in {rendered}"
        );
    }
}

#[test]
fn fact_receipt_signing_key_file_accepts_raw_and_hex_material() {
    let dir = tempfile::tempdir().expect("tempdir");
    let raw_path = dir.path().join("raw.key");
    let raw = [0x27_u8; 32];
    std::fs::write(&raw_path, raw).expect("write raw key");

    let loaded = load_fact_receipt_signing_key_file(&raw_path).expect("raw key loads");
    assert_eq!(&*loaded, &raw);

    let hex_path = dir.path().join("hex.key");
    std::fs::write(&hex_path, format!("0x{}\n", "27".repeat(32))).expect("write hex key");

    let loaded = load_fact_receipt_signing_key_file(&hex_path).expect("hex key loads");
    assert_eq!(&*loaded, &raw);
}

#[test]
fn fact_receipt_signing_key_errors_are_redacted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let invalid_path = dir.path().join("fact-receipt-secret.key");
    let invalid_secret = "not-valid-secret-material";
    std::fs::write(&invalid_path, invalid_secret).expect("write invalid key");

    let invalid =
        load_fact_receipt_signing_key_file(&invalid_path).expect_err("invalid key is rejected");
    assert_eq!(invalid.code, "FactReceiptSigningKeyInvalid");
    let rendered = format!("{invalid:?}\n{invalid}");
    assert!(!rendered.contains(invalid_secret));
    assert!(!rendered.contains(invalid_path.to_str().expect("utf8 path")));

    let missing_path = dir.path().join("missing-secret.key");
    let missing =
        load_fact_receipt_signing_key_file(&missing_path).expect_err("missing key is rejected");
    assert_eq!(missing.code, "FactReceiptSigningKeyReadFailed");
    let rendered = format!("{missing:?}\n{missing}");
    assert!(!rendered.contains(missing_path.to_str().expect("utf8 path")));
}

#[test]
fn fact_query_receipt_authentication_errors_report_signer_requirement() {
    let private_diagnostic = "missing fact receipt signer";
    let error =
        fact_query_execution_store_error(mfm_stream_store_postgres::PostgresStoreError::Store(
            store::StoreError::ReceiptAuthentication {
                message: private_diagnostic.to_owned(),
            },
        ));

    assert_eq!(error.code, "MissingFactReceiptSigningKey");
    assert_eq!(
        error.message,
        "Fact receipt signing key is required for public fact queries"
    );
    let rendered = format!("{error:?}\n{error}");
    assert!(!rendered.contains(private_diagnostic));
}
