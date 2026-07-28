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
    type Effect = mfm_capabilities::ReadExternal;
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

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for AppFactState {
    type Plan = AppFactValue;
    type Evidence = AppFactValue;
    type Facts = mfm_values::NonEmpty<AppLaunchFact>;

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
    ) -> StateResult<(Self::Output, Self::Facts)> {
        if !evidence.fact_query_evidence().is_empty()
            || evidence.primary_evidence() != &self.plan(input, context)?
        {
            return Err(mfm_program::StateError::Message(
                "app fact fixture evidence did not match its plan".to_owned(),
            ));
        }
        Ok((
            evidence.primary_evidence().clone(),
            mfm_values::NonEmpty::new(
                AppLaunchFact {
                    subject: input.clone(),
                    response: AppFactValue { amount: 15 },
                },
                Vec::new(),
            ),
        ))
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

pub(super) fn app_fact_certification_registry() -> CertificationRegistry {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<AppFactState>()
        .expect("certification state registration");
    registry
}

pub(super) struct AppFactReadExecutor;

impl mfm_runtime::ExternalReadPlanExecutor<AppFactState> for AppFactReadExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: mfm_runtime::RunnerIngressContext<'a>,
        _state: &'a AppFactState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a AppFactValue,
        _ctx: &'a mfm_runtime::ErasedRunCtx<'_>,
    ) -> mfm_runtime::ExternalReadExecutionFuture<'a, AppFactValue> {
        Box::pin(async move { Ok(mfm_runtime::ExternalReadExecution::primary(plan.clone())) })
    }
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
) -> ErasedRunnerRegistry {
    let node = runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("app fact state node");
    let mut registry = test_runner_registry();
    let runner_factory = test_factory_binding(&registry, "read_external");
    let adapter_factory = test_factory_binding(&registry, "app_fact_adapter");
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
            runner_factory.factory_id(),
            runner_factory.executable(),
            Arc::new(mfm_runtime::ExternalReadRunner::<AppFactState, _>::new(
                AppFactReadExecutor,
            )),
        )
        .expect("app fact runner registration")
        .register_adapter_executable(
            app_fact_adapter_kind(),
            app_fact_adapter_version(),
            adapter_factory.executable(),
        )
        .expect("app fact runner registration");
    registry
}

pub(super) fn prepare_app_fact_launch() -> Result<RunLaunchRequest, PublicError> {
    prepare_app_fact_launch_with_invocation_key(None)
}

pub(super) fn prepare_app_fact_launch_with_invocation_key(
    invocation_key_digest: Option<ContentDigest>,
) -> Result<RunLaunchRequest, PublicError> {
    prepare_app_fact_launch_with_invocation_key_and_state_key(invocation_key_digest, "fact-state")
}

pub(super) fn prepare_app_fact_launch_with_invocation_key_and_state_key(
    invocation_key_digest: Option<ContentDigest>,
    state_key: &str,
) -> Result<RunLaunchRequest, PublicError> {
    let plan = app_fact_launch_plan_with_state_key(state_key);
    let registry = app_fact_certification_registry();
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
    launch_app_fact_run_in_store(store::AsyncInMemoryRunStore::default(), None).await
}

pub(super) async fn launch_app_fact_run_in_store(
    store: store::AsyncInMemoryRunStore,
    invocation_key_digest: Option<ContentDigest>,
) -> (RunId, store::AsyncInMemoryRunStore, CertificationRegistry) {
    launch_app_fact_run_in_store_with_state_key(store, invocation_key_digest, "fact-state", true)
        .await
}

pub(super) async fn launch_app_fact_run_in_store_with_state_key(
    store: store::AsyncInMemoryRunStore,
    invocation_key_digest: Option<ContentDigest>,
    state_key: &str,
    require_completion: bool,
) -> (RunId, store::AsyncInMemoryRunStore, CertificationRegistry) {
    let request =
        prepare_app_fact_launch_with_invocation_key_and_state_key(invocation_key_digest, state_key)
            .expect("prepared app fact launch");
    let runners = app_fact_runner_registry(&request.runtime_spec);
    let registry = app_fact_certification_registry();
    let run_id = request.run_id.clone();
    let execution_scope =
        store::ExecutionClaimScope::from_run_identity_material(&request.identity_material);
    let scheduler = SerialTypedScheduler::new(runners);
    let launch = scheduler
        .prepare_run_launch(
            &request.runtime_spec,
            request.identity_material,
            request.evidence,
            store::StreamSeq::FIRST,
        )
        .await
        .expect("prepare fact run launch");
    scheduler
        .start_run(&store, launch)
        .await
        .expect("start fact run");
    let mut current = scheduler
        .load_admitted_run(&store, request.runtime_spec, &run_id)
        .await
        .expect("load admitted fact run");
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
        let (completed, fact_recorded) = {
            let lifecycle = store::current_lifecycle::read(current.view());
            let mut fact_recorded = false;
            let _ = lifecycle.visit_records(|record| {
                fact_recorded |= matches!(
                    record.kind(),
                    store::current_lifecycle::CurrentRecordKindRef::FactRecorded(_)
                );
                std::ops::ControlFlow::<()>::Continue(())
            });
            (lifecycle.completion().is_some(), fact_recorded)
        };
        if completed {
            break;
        }
        if !require_completion && fact_recorded {
            break;
        }
        match scheduler
            .drive_once(&store, current, &execution_scope, token.clone())
            .await
        {
            Ok(result) => current = result.into_current_run(),
            Err(error) => {
                let view = load_verified_run_view(&store, &registry, &run_id)
                    .await
                    .expect("verified failed fact run");
                let event_variants = app_fact_record_variants(&view);
                panic!("drive fact run failed: {error:?}; events: {event_variants:?}");
            }
        }
    }
    let lifecycle = store::current_lifecycle::read(current.view());
    if require_completion {
        assert!(lifecycle.completion().is_some());
    }
    let event_variants = app_fact_record_variants(current.view());
    let mut fact_recorded = false;
    let _ = lifecycle.visit_records(|record| {
        fact_recorded |= matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::FactRecorded(_)
        );
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert!(fact_recorded, "events: {event_variants:?}");

    (run_id, store, registry)
}

fn app_fact_record_variants(view: &store::VerifiedRunView) -> Vec<String> {
    let lifecycle = store::current_lifecycle::read(view);
    let mut variants = Vec::new();
    let _ = lifecycle.visit_records(|record| {
        let variant = match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::RunAdmitted(_) => "RunAdmitted".into(),
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptStarted(_) => {
                "StateAttemptStarted".into()
            }
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptFailed(payload) => format!(
                "StateAttemptFailed:{}:{}",
                payload.error.code, payload.error.safe_message
            ),
            store::current_lifecycle::CurrentRecordKindRef::CellProduced(_) => {
                "CellProduced".into()
            }
            store::current_lifecycle::CurrentRecordKindRef::FactRecorded(_) => {
                "FactRecorded".into()
            }
            store::current_lifecycle::CurrentRecordKindRef::ArtifactReferenced(_) => {
                "ArtifactReferenced".into()
            }
            store::current_lifecycle::CurrentRecordKindRef::RetentionRefsAppended(_) => {
                "RetentionRefsAppended".into()
            }
            store::current_lifecycle::CurrentRecordKindRef::PublicOutputProduced(_) => {
                "PublicOutputProduced".into()
            }
            store::current_lifecycle::CurrentRecordKindRef::RunCompleted(_) => {
                "RunCompleted".into()
            }
            _ => "Other".into(),
        };
        variants.push(variant);
        std::ops::ControlFlow::<()>::Continue(())
    });
    variants
}
