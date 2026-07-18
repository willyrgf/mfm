use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.app.test",
    name = "fact_value",
    version = "1",
    schema = "mfm.app.test.fact_value"
)]
pub(super) struct AppFactValue {
    amount: u64,
}

pub(super) struct AppFactReadCap;

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

pub(super) fn app_fact_adapter_kind() -> mfm_ids::AdapterKind {
    mfm_ids::AdapterKind::new(
        "mfm.app.test",
        "fact-adapter",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.app.test.fact-adapter"),
    )
    .expect("adapter kind")
}

pub(super) fn app_fact_adapter_version() -> mfm_ids::AdapterVersion {
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
pub(super) struct AppLaunchFact {
    subject: AppFactValue,
    response: AppFactValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
pub(super) struct AppFactStateConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.app.test.public_outputs")]
pub(super) struct AppFactPublicOutputs<'p, 's> {
    result: mfm_program::Handle<'p, 's, AppFactValue>,
}

pub(super) struct AppFactState {
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
    type Plan = AppFactValue;
    type Evidence = AppFactValue;

    fn plan(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        Ok(AppFactValue {
            amount: input.amount * self.config.multiplier,
        })
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &mfm_program::ExternalReadEvidenceSet<Self::Evidence>,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        if !evidence.fact_query_evidence().is_empty()
            || evidence.primary_evidence() != &self.plan(input, context)?
        {
            return Err(mfm_program::StateError::Message(
                "app fact fixture evidence did not match its plan".to_owned(),
            ));
        }
        Ok(evidence.primary_evidence().clone())
    }
}

pub(super) fn app_fact_launch_plan_with_state_key(state_key: &str) -> TypedProgramLaunchPlan {
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

pub(super) fn app_fact_certification_registry(
    include_fact_descriptor: bool,
) -> CertificationRegistry {
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

pub(super) struct AppFactRecordingRunner {
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
            output.record_external_read_evidence(&value)?;
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

pub(super) fn app_fact_runner_capability_binding() -> mfm_runtime::RunnerCapabilityBinding {
    mfm_runtime::RunnerCapabilityBinding::for_capability::<AppFactReadCap>(
        app_fact_adapter_kind(),
        app_fact_adapter_version(),
    )
    .expect("runner capability binding")
}

pub(super) fn app_launch_fact_query_request() -> PublicFactQueryRequest {
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

pub(super) fn app_fact_runner_registry(
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
    registry
        .register_capability_set(
            &node.capability_bindings,
            mfm_runtime::CapabilityImplementationId::new("mfm.app.test.fact-runner")
                .expect("implementation id"),
        )
        .expect("app fact capability registration");
    mfm_runtime::RunnerRegistrationBuilder::new(&mut registry)
        .register_runner(
            node.descriptor_id.clone(),
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

pub(super) fn prepare_app_fact_launch(
    include_fact_descriptor: bool,
) -> Result<RunLaunchRequest, PublicError> {
    prepare_app_fact_launch_with_invocation_key(include_fact_descriptor, None)
}

pub(super) fn prepare_btc_collector_internal_test_launch() -> Result<RunLaunchRequest, PublicError>
{
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
    let (certified_spec, scoped, config_inputs, seed_inputs) =
        certify_launch_plan(&plan, &registry)?;

    prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: &scoped,
            store_scope_id: StoreScopeId::new(
                "mfm.store_scope.v1:00000000000000000000000000000000",
            )
            .expect("store scope"),
            invocation_key_digest: default_invocation_key_digest(),
            entry_point_evidence: events::EntryPointLaunchEvidence::new(
                "mfm.bitcoin/btc_chain_head_internal_test@1",
                Vec::new(),
            )
            .expect("entry point evidence"),
        },
        config_inputs,
        seed_inputs,
    )
}

pub(super) fn prepare_app_fact_launch_with_invocation_key(
    include_fact_descriptor: bool,
    invocation_key_digest: Option<ContentDigest>,
) -> Result<RunLaunchRequest, PublicError> {
    prepare_app_fact_launch_with_invocation_key_and_state_key(
        include_fact_descriptor,
        invocation_key_digest,
        "fact-state",
    )
}

pub(super) fn prepare_app_fact_launch_with_invocation_key_and_state_key(
    include_fact_descriptor: bool,
    invocation_key_digest: Option<ContentDigest>,
    state_key: &str,
) -> Result<RunLaunchRequest, PublicError> {
    let plan = app_fact_launch_plan_with_state_key(state_key);
    let registry = app_fact_certification_registry(include_fact_descriptor);
    let (certified_spec, scoped, config_inputs, seed_inputs) =
        certify_launch_plan(&plan, &registry)?;

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
            entry_point_evidence: events::EntryPointLaunchEvidence::new(
                "mfm.app.test/fact_launch@1",
                Vec::new(),
            )
            .expect("entry point evidence"),
        },
        config_inputs,
        seed_inputs,
    )
}

pub(super) fn default_invocation_key_digest() -> ContentDigest {
    content_digest_for_bytes(b"mfm.app.test.default-invocation")
}

pub(super) async fn launch_app_fact_run(
) -> (RunId, store::AsyncInMemoryRunStore, CertificationRegistry) {
    launch_app_fact_run_with_visibility(mfm_program::facts::FactVisibility::indexed_default(
        mfm_program::facts::FactAudience::Platform,
    ))
    .await
}

pub(super) async fn launch_app_fact_run_with_visibility(
    visibility: mfm_program::facts::FactVisibility,
) -> (RunId, store::AsyncInMemoryRunStore, CertificationRegistry) {
    let store = store::AsyncInMemoryRunStore::default();
    launch_app_fact_run_in_store_with_visibility(store, visibility, None).await
}

pub(super) async fn launch_app_fact_run_in_store_with_visibility(
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

pub(super) async fn launch_app_fact_run_in_store_with_visibility_and_state_key(
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
