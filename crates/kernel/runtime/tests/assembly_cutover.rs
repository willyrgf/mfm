use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_capabilities::ProposedStateOutcome;
use mfm_ids::{AppendRequestId, RunId, StableId, StoreEpoch, StoreScopeId, TenantScopeId};
use mfm_journal::single_trust::RunRecord;
use mfm_program::{
    nominal_contract_ref, state_implementation_ref, Declaration, ExecutionMode, FailureValue,
    ProgramCatalog, ProgramDocument, State, StateDeclaration,
};
use mfm_program_derive::{MfmConfig, MfmValue};
use mfm_runtime::{PureImplementation, ResumeStep, Runtime, RuntimeAssemblyBuilder, SpawnStep};
use mfm_store::{
    ConfigurationCommitOutcome, StoreWorkLimits, StructuredStore, StructuredStoreIdentity,
};
use mfm_values::ValidatedConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct NonCloneInput {
    value: String,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Success {
    value: String,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Failure {
    code: String,
}

impl FailureValue for Failure {
    fn integrity_blocked() -> Self {
        Self {
            code: "integrity_blocked".to_owned(),
        }
    }
}

struct FailingState;

impl State for FailingState {
    type Input = NonCloneInput;
    type Output = Success;
    type Failure = Failure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.runtime-cutover/failing-state@1")
            .map_err(|_| mfm_program::ProgramError::InvalidContract)
    }
}

struct AdvancingState;

impl State for AdvancingState {
    type Input = NonCloneInput;
    type Output = NonCloneInput;
    type Failure = Failure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.runtime-cutover/advancing-state@1")
            .map_err(|_| mfm_program::ProgramError::InvalidContract)
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
struct TestConfig {
    revision: u8,
}

fn identity() -> StructuredStoreIdentity {
    StructuredStoreIdentity::new(
        StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef").expect("scope"),
        StoreEpoch::new(1),
        TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef").expect("tenant"),
    )
}

fn catalog_and_program(document: ProgramDocument) -> (ProgramCatalog, mfm_program::Program) {
    let mut builder = ProgramCatalog::builder();
    builder
        .register_value::<NonCloneInput>()
        .expect("input contract");
    builder
        .register_value::<Success>()
        .expect("success contract");
    builder
        .register_value::<Failure>()
        .expect("failure contract");
    builder.finish(document).expect("catalog and program")
}

async fn configuration_head(
    store: &mfm_store::ConfigurationStore,
) -> mfm_store::ResolvedConfigurationHead {
    let owner = store
        .initial_write_session::<TestConfig>()
        .prepare_local(
            AppendRequestId::new("runtime-cutover-config-000001").expect("request"),
            ValidatedConfig::new(TestConfig { revision: 1 }).expect("config"),
        )
        .expect("configuration owner");
    match store.commit(owner).await.expect("configuration commit") {
        ConfigurationCommitOutcome::NewlyCommitted(value)
        | ConfigurationCommitOutcome::Found(value) => value.into_head(),
        other => panic!("unexpected configuration disposition: {other:?}"),
    }
}

#[tokio::test]
async fn terminal_typed_failure_needs_no_fake_success_state() {
    let input = nominal_contract_ref::<NonCloneInput>().expect("input");
    let success = nominal_contract_ref::<Success>().expect("success");
    let failure = nominal_contract_ref::<Failure>().expect("failure");
    let document = ProgramDocument::new(
        StableId::new("mfm.runtime-cutover/failure@1").expect("entry"),
        success,
        input.clone(),
        vec![Declaration::State(Box::new(
            StateDeclaration::new(
                mfm_program::SequentialControlAddress::new(0, Vec::new()).expect("address"),
                state_implementation_ref::<FailingState>().expect("state"),
                input.clone(),
                nominal_contract_ref::<Success>().expect("success"),
                Some(failure),
                ExecutionMode::Pure,
                true,
            )
            .expect("terminal State"),
        ))],
    )
    .expect("program");
    let (catalog, program) = catalog_and_program(document);
    let callbacks = Arc::new(AtomicUsize::new(0));
    let mut assembly = RuntimeAssemblyBuilder::new(catalog.clone());
    let counter = Arc::clone(&callbacks);
    assembly
        .register_pure::<FailingState>(PureImplementation::new(move |_input| {
            counter.fetch_add(1, Ordering::SeqCst);
            ProposedStateOutcome::Failure {
                failure: Failure {
                    code: "rejected".to_owned(),
                },
            }
        }))
        .expect("registration");
    let store =
        StructuredStore::open_memory(identity(), catalog.clone(), StoreWorkLimits::default())
            .expect("store");
    let (history, _reader, configuration, _audit) = store.split().into_parts();
    let runtime = Runtime::new(assembly.finish().expect("assembly"), history).expect("runtime");
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect("run");
    let spawned = runtime
        .admission(
            run_id.clone(),
            program,
            catalog
                .qualify(
                    input,
                    NonCloneInput {
                        value: "moved-once".to_owned(),
                    },
                )
                .expect("input"),
            configuration_head(&configuration).await,
            Vec::new(),
        )
        .expect("admission")
        .spawn()
        .await;
    let session = match spawned {
        SpawnStep::Active(session) => session,
        _ => panic!("terminal State must begin as an active session"),
    };
    let terminal = match session.drive().await {
        mfm_runtime::RuntimeStep::Terminal(terminal) => terminal,
        _ => panic!("declared typed failure must become terminal"),
    };
    assert_eq!(callbacks.load(Ordering::SeqCst), 1);
    assert!(matches!(
        terminal.qualified_run().frames().last().expect("conclusion").record(),
        RunRecord::StateConcluded(conclusion)
            if matches!(conclusion.outcome(), mfm_journal::single_trust::StateOutcome::Failure(_))
    ));
    assert!(matches!(
        runtime.resume_run(run_id).await,
        ResumeStep::Terminal(_)
    ));
}

#[tokio::test]
async fn pure_session_advances_through_runtime_and_store() {
    let input = nominal_contract_ref::<NonCloneInput>().expect("input");
    let failure = nominal_contract_ref::<Failure>().expect("failure");
    let first = mfm_program::SequentialControlAddress::new(0, Vec::new()).expect("first");
    let second = mfm_program::SequentialControlAddress::new(1, Vec::new()).expect("second");
    let state = state_implementation_ref::<AdvancingState>().expect("state");
    let document = ProgramDocument::new(
        StableId::new("mfm.runtime-cutover/repeated@1").expect("entry"),
        input.clone(),
        input.clone(),
        vec![
            Declaration::State(Box::new(
                StateDeclaration::with_next(
                    first,
                    state.clone(),
                    input.clone(),
                    input.clone(),
                    Some(failure.clone()),
                    ExecutionMode::Pure,
                    second,
                )
                .expect("first State"),
            )),
            Declaration::State(Box::new(
                StateDeclaration::new(
                    mfm_program::SequentialControlAddress::new(1, Vec::new()).expect("second"),
                    state,
                    input.clone(),
                    input.clone(),
                    Some(failure),
                    ExecutionMode::Pure,
                    true,
                )
                .expect("second State"),
            )),
        ],
    )
    .expect("program");
    let (catalog, program) = catalog_and_program(document);
    let callbacks = Arc::new(AtomicUsize::new(0));
    let mut assembly = RuntimeAssemblyBuilder::new(catalog.clone());
    let counter = Arc::clone(&callbacks);
    assembly
        .register_pure::<AdvancingState>(PureImplementation::new(move |input: NonCloneInput| {
            counter.fetch_add(1, Ordering::SeqCst);
            ProposedStateOutcome::Success {
                output: NonCloneInput { value: input.value },
                facts: mfm_facts::FactProposalSet::empty(),
            }
        }))
        .expect("one semantic registration");
    let store =
        StructuredStore::open_memory(identity(), catalog.clone(), StoreWorkLimits::default())
            .expect("store");
    let (history, _reader, configuration, _audit) = store.split().into_parts();
    let runtime = Runtime::new(assembly.finish().expect("assembly"), history).expect("runtime");
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect("run");
    let first_step = runtime
        .admission(
            run_id.clone(),
            program,
            catalog
                .qualify(
                    input,
                    NonCloneInput {
                        value: "retained-affinely".to_owned(),
                    },
                )
                .expect("input"),
            configuration_head(&configuration).await,
            Vec::new(),
        )
        .expect("admission")
        .spawn()
        .await;
    let session = match first_step {
        SpawnStep::Active(session) => session,
        _ => panic!("first repeated occurrence must advance"),
    };
    let session = match session.drive().await {
        mfm_runtime::RuntimeStep::Advanced(session) => session,
        _ => panic!("first repeated occurrence must advance"),
    };
    assert_eq!(session.head_sequence(), 2);
    drop(session);
    let session = match runtime.resume_run(run_id).await {
        ResumeStep::Active(session) => session,
        _ => panic!("cold resume must select the second repeated occurrence"),
    };
    let terminal = match session.drive().await {
        mfm_runtime::RuntimeStep::Terminal(terminal) => terminal,
        _ => panic!("second repeated occurrence must terminate"),
    };
    assert_eq!(terminal.head_sequence(), 3);
    assert_eq!(callbacks.load(Ordering::SeqCst), 2);
}
