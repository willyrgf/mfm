use super::*;

use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

use alloy_primitives::{keccak256, Address, Bytes, PrimitiveSignature, TxKind, B256, U256};
use k256::ecdsa::SigningKey;
use k256::elliptic_curve::rand_core::OsRng;
use mfm_certify::CertificationRegistry;
use mfm_evm::{
    evm_diagnostic, evm_sender_lane_resource_claim, EvmBlockAnchor, EvmBlockSelector, EvmCall,
    EvmCapabilityError, EvmCode, EvmContractCallCheck, EvmContractValidationConfig,
    EvmContractValidationTarget, EvmFeeInputs, EvmNetworkBinding, EvmObservedTransaction,
    EvmReadSession, EvmReadSessionSet, EvmReceipt, EvmReceiptStatus, EvmSessionEvidence,
    EvmSessionFuture, EvmTransactionAction, EvmTransactionConfig, EvmTransactionEstimate,
    EvmTransactionOutcome, EvmTransactionResult, EvmTransactionSession, EvmTransactionSessionSet,
    EvmTransactionSuccess, SubmitEvmTransactionState, ValidateEvmContractState,
    VerifiedEvmContract, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_evm_live::{register_evm_transaction_runner, register_evm_validation_runner};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, NoContext, PublicOutputKey, PureState, RootBuilder,
    ScopeKey, SeedKey, SideEffectSagaPolicy, SideEffectVerificationSpec, StateError, StateKey,
    StateRegistryBuilder, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, PublicOutputs, StateInput};
use mfm_runtime::{register_pure_state, ErasedRunnerRegistry, RunnerRegistrationBuilder};
use mfm_signing::{
    DeterministicSigningProvider, PublicSigningIdentity, SignatureBytes, SigningFuture,
    SigningProvider, SigningRequest, SigningResult, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use mfm_store::v1::{self as store, StoreScopeStore as _};

const BASE_NONCE: u64 = 7;
const RUNTIME_CODE: &[u8] = &[0x60, 0x00, 0x60, 0x01];
const VALIDATION_CALLDATA: &[u8] = &[0x99];
const VALIDATION_RETURN: &[u8] = &[0x01];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.app.test.evm_transaction_composition_projection_config")]
struct ProjectionConfig {}

struct FirstCallActionState;
struct SecondCallActionState;
struct ValidationTargetState;
struct DirectValidationTargetState;

fn composition_state_kind(name: &str) -> mfm_program::Result<mfm_ids::StateKind> {
    mfm_ids::StateKind::new(
        "mfm.app.test",
        name,
        mfm_ids::DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(format!("mfm.app.test:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

macro_rules! impl_projection_state_spec {
    ($state:ty, $input:ty, $output:ty, $kind:literal) => {
        impl StateSpec for $state {
            type Config = ProjectionConfig;
            type Context = NoContext;
            type Input = $input;
            type Output = $output;
            type Effect = mfm_capabilities::Pure;
            type Caps = mfm_capabilities::NoCaps;

            fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
                composition_state_kind($kind)
            }

            fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
                mfm_ids::StateVersion::new(concat!("mfm.app.test.", $kind, ".v1"))
                    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
            }

            fn name() -> &'static str {
                concat!("mfm.app.test.", $kind)
            }

            fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
                Ok(Self)
            }
        }
    };
}

impl_projection_state_spec!(
    FirstCallActionState,
    EvmTransactionOutcome,
    EvmTransactionAction,
    "evm_contract_first_call"
);

impl PureState for FirstCallActionState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        let address = created_address(&input)?;
        EvmTransactionAction::call(address, [0x01], U256::ZERO).map_err(StateError::from)
    }
}

impl_projection_state_spec!(
    SecondCallActionState,
    EvmTransactionOutcome,
    EvmTransactionAction,
    "evm_contract_second_call"
);

impl PureState for SecondCallActionState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        let address = called_address(&input)?;
        EvmTransactionAction::call(address, [0x02], U256::ZERO).map_err(StateError::from)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.app.test.evm_contract_validation_target_input")]
struct ValidationTargetInput {
    created: EvmTransactionOutcome,
    final_call: EvmTransactionOutcome,
}

impl_projection_state_spec!(
    ValidationTargetState,
    ValidationTargetInput,
    EvmContractValidationTarget,
    "evm_contract_validation_target"
);

impl PureState for ValidationTargetState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        let created = created_address(&input.created)?;
        if called_address(&input.final_call)? != created {
            return Err(StateError::Message(
                "final configuration call did not target the created contract".to_owned(),
            ));
        }
        EvmContractValidationTarget::new(created, input.final_call.receipt().block_anchor().clone())
            .map_err(StateError::from)
    }
}

impl_projection_state_spec!(
    DirectValidationTargetState,
    EvmTransactionOutcome,
    EvmContractValidationTarget,
    "evm_direct_contract_validation_target"
);

impl PureState for DirectValidationTargetState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        EvmContractValidationTarget::new(
            successful_transaction_address(&input)?,
            input.receipt().block_anchor().clone(),
        )
        .map_err(StateError::from)
    }
}

fn successful_transaction_address(outcome: &EvmTransactionOutcome) -> StateResult<Address> {
    match outcome.result() {
        EvmTransactionResult::Succeeded {
            result:
                EvmTransactionSuccess::Created { address } | EvmTransactionSuccess::Called { address },
        } => Address::from_str(address).map_err(|_| {
            StateError::Message("successful transaction address was invalid".to_owned())
        }),
        EvmTransactionResult::Reverted { .. } => Err(StateError::Message(
            "reverted transaction did not produce a usable address".to_owned(),
        )),
    }
}

fn created_address(outcome: &EvmTransactionOutcome) -> StateResult<Address> {
    let EvmTransactionResult::Succeeded {
        result: EvmTransactionSuccess::Created { address },
    } = outcome.result()
    else {
        return Err(StateError::Message(
            "contract creation did not produce a created address".to_owned(),
        ));
    };
    Address::from_str(address)
        .map_err(|_| StateError::Message("created contract address was invalid".to_owned()))
}

fn called_address(outcome: &EvmTransactionOutcome) -> StateResult<Address> {
    let EvmTransactionResult::Succeeded {
        result: EvmTransactionSuccess::Called { address },
    } = outcome.result()
    else {
        return Err(StateError::Message(
            "contract call did not produce a called address".to_owned(),
        ));
    };
    Address::from_str(address)
        .map_err(|_| StateError::Message("called contract address was invalid".to_owned()))
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.app.test.evm_transaction_composition_public_outputs")]
struct TransactionCompositionPublicOutputs<'program, 'scope> {
    verified: mfm_program::Handle<'program, 'scope, VerifiedEvmContract>,
}

#[tokio::test]
async fn create_call_call_validate_graph_replays_from_evidence_only() {
    assert_composition_replays(composition_launch_material, 3).await;
}

#[tokio::test]
async fn direct_create_validate_graph_replays_from_evidence_only() {
    assert_composition_replays(direct_composition_launch_material, 1).await;
}

#[tokio::test]
async fn isolated_call_validate_graph_replays_from_evidence_only() {
    assert_composition_replays(call_validate_launch_material, 1).await;
}

#[tokio::test]
async fn reverted_call_preserves_earlier_receipts_and_prevents_later_calls() {
    let store = store::AsyncInMemoryRunStore::default();
    let signing_key = Arc::new(SigningKey::random(&mut OsRng));
    let sender = signing_key_address(&signing_key);
    let created = sender.create(BASE_NONCE);
    let world = Arc::new(TransactionCompositionWorld::reverting_first_call(
        sender, created,
    ));
    let live_validation_reads = Arc::new(AtomicUsize::new(0));
    let certification = composition_certification_registry();
    let launch_services = make_run_services(
        composition_runners(
            Arc::clone(&world),
            Arc::clone(&signing_key),
            Arc::clone(&live_validation_reads),
        ),
        Arc::new(store.clone()),
        certification.clone(),
    );
    let (draft, seed_material) = composition_launch_material(sender, created);
    let request = crate::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        launch_services.certification_registry(),
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("failed composition launch request");
    let run_id = request.run_id.clone();

    let launch = launch_services
        .launch_run(request)
        .await
        .expect("launch reverted transaction composition");
    let (_, launched, _) = launch.into_response_parts();
    let launched = launched.expect("admitted reverted composition");
    assert_eq!(launched.run_mode, RunModeStatus::FailedWithoutAcdcClaim);
    assert_eq!(world.included_count(), 2);
    assert_eq!(
        world.receipt_statuses(),
        vec![EvmReceiptStatus::Success, EvmReceiptStatus::Reverted]
    );
    assert_eq!(live_validation_reads.load(Ordering::SeqCst), 0);
    let view = load_verified_run_view(&store, &certification, &run_id)
        .await
        .expect("verified reverted composition");
    let lifecycle = store::current_lifecycle::read(&view);
    let mut receipt_count = 0;
    let mut submission_count = 0;
    let mut diagnostic_ref = None;
    let mut diagnostic_requirement = None;
    let _ = lifecycle.visit_records(|record| {
        match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::SideEffectReceiptObserved(_) => {
                receipt_count += 1;
            }
            store::current_lifecycle::CurrentRecordKindRef::SideEffectSubmissionObserved(_) => {
                submission_count += 1;
            }
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptFailed(payload) => {
                if let Some(reference) = &payload.error.diagnostic_ref {
                    diagnostic_ref = Some(reference.clone());
                    let _ = record.visit_artifact_requirements(|candidate| {
                        if candidate.source
                            == store::EventArtifactReferenceSource::StateAttemptFailureDiagnostic
                        {
                            diagnostic_requirement = Some(candidate.clone());
                            std::ops::ControlFlow::Break(())
                        } else {
                            std::ops::ControlFlow::Continue(())
                        }
                    });
                }
            }
            _ => {}
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert_eq!(receipt_count, 2);
    assert_eq!(submission_count, 2);
    let diagnostic_ref = diagnostic_ref.expect("reverted call diagnostic reference");
    let diagnostic_requirement =
        diagnostic_requirement.expect("exact reverted call diagnostic requirement");
    assert!(diagnostic_requirement.producer_node_id.is_some());
    assert!(lifecycle
        .object_for_requirement(&diagnostic_requirement)
        .is_some());

    let producer_agnostic_requirement = store::diagnostic_artifact_requirement(&diagnostic_ref);
    assert_ne!(producer_agnostic_requirement, diagnostic_requirement);
    assert!(lifecycle
        .object_for_requirement(&producer_agnostic_requirement)
        .is_none());

    let mut wrong_evidence_requirement = diagnostic_requirement.clone();
    wrong_evidence_requirement.evidence_hash = mfm_ids::ContentDigest::from_digest(
        mfm_ids::DigestAlgorithm::Sha256V1,
        mfm_canonical::sha256_digest_bytes(b"wrong diagnostic evidence"),
    );
    assert!(lifecycle
        .object_for_requirement(&wrong_evidence_requirement)
        .is_none());
    drop(launch_services);
    drop(signing_key);

    let replay_services = make_run_read_services(Arc::new(store.clone()), certification.clone());
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only failed composition replay");
    assert_eq!(replay.run_mode, RunModeStatus::FailedWithoutAcdcClaim);
    replay_services
        .inspect_replay_broker_for_test(&run_id, |broker| {
            mfm_evm_live::verify_evm_transaction_replay(broker)?;
            mfm_evm_live::verify_evm_validation_replay(broker)?;
            verify_composition_pure_states(broker);
            Ok::<(), mfm_replay::v1::ReplayError>(())
        })
        .await
        .expect("failed composition replay broker")
        .expect("explicit composition replay verifiers");
    assert_eq!(live_validation_reads.load(Ordering::SeqCst), 0);
}

type CompositionLaunchMaterial = (
    mfm_program::TypedProgramDraft,
    BTreeMap<mfm_ids::SeedId, mfm_canonical::PlainCanonicalJsonBytes>,
);

async fn assert_composition_replays(
    build: fn(Address, Address) -> CompositionLaunchMaterial,
    expected_transaction_count: usize,
) {
    let store = store::AsyncInMemoryRunStore::default();
    let signing_key = Arc::new(SigningKey::random(&mut OsRng));
    let sender = signing_key_address(&signing_key);
    let created = sender.create(BASE_NONCE);
    let world = Arc::new(TransactionCompositionWorld::new(sender, created));
    let live_validation_reads = Arc::new(AtomicUsize::new(0));
    let certification = composition_certification_registry();
    let launch_services = make_run_services(
        composition_runners(
            Arc::clone(&world),
            Arc::clone(&signing_key),
            Arc::clone(&live_validation_reads),
        ),
        Arc::new(store.clone()),
        certification.clone(),
    );
    let (draft, seed_material) = build(sender, created);
    let lowered = mfm_certify::lower_program_draft(&draft).expect("lower composition draft");
    let scoped = certification
        .scoped_for_spec(lowered.spec())
        .expect("scope composition certification registry");
    mfm_certify::certify_typed_spec(lowered, &scoped).expect("certify composition draft");
    let request = crate::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        launch_services.certification_registry(),
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("composition launch request");
    let run_id = request.run_id.clone();

    let launch = launch_services
        .launch_run(request)
        .await
        .expect("launch transaction composition graph");
    let (_, launched, _) = launch.into_response_parts();
    assert_eq!(
        launched
            .expect("completed transaction composition")
            .run_mode,
        RunModeStatus::Completed
    );
    assert_eq!(world.included_count(), expected_transaction_count);
    assert_eq!(live_validation_reads.load(Ordering::SeqCst), 3);
    drop(launch_services);
    drop(signing_key);

    let replay_services = make_run_read_services(Arc::new(store.clone()), certification.clone());
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only transaction composition replay");
    assert_eq!(replay.run_mode, RunModeStatus::Completed);
    replay_services
        .inspect_replay_broker_for_test(&run_id, |broker| {
            mfm_evm_live::verify_evm_transaction_replay(broker)?;
            mfm_evm_live::verify_evm_validation_replay(broker)?;
            verify_composition_pure_states(broker);
            Ok::<(), mfm_replay::v1::ReplayError>(())
        })
        .await
        .expect("composition replay broker")
        .expect("explicit composition replay verifiers");
    assert_eq!(live_validation_reads.load(Ordering::SeqCst), 3);
}

fn verify_composition_pure_states(broker: &mfm_replay::v1::ReplayBroker<'_>) {
    mfm_replay::v1::verify_pure_state::<FirstCallActionState>(broker)
        .expect("first-call projection replay");
    mfm_replay::v1::verify_pure_state::<SecondCallActionState>(broker)
        .expect("second-call projection replay");
    mfm_replay::v1::verify_pure_state::<ValidationTargetState>(broker)
        .expect("validation-target projection replay");
    mfm_replay::v1::verify_pure_state::<DirectValidationTargetState>(broker)
        .expect("direct validation-target projection replay");
}

fn direct_composition_launch_material(
    sender: Address,
    created: Address,
) -> CompositionLaunchMaterial {
    let create_action = CanonicalSeed::from_value(
        &EvmTransactionAction::create([0x60, 0x00], U256::ZERO).expect("create action"),
    )
    .expect("create action seed");
    let create_bytes = create_action.canonical_json().clone();
    let draft = build_root_with_registries(
        ScopeKey::new("evm-direct-transaction-composition").expect("scope key"),
        composition_state_registry(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let create_action = root.seed(SeedKey::new("create-action")?, create_action.clone())?;
            let created_outcome = root
                .scope()
                .side_effect::<SubmitEvmTransactionState, _>(
                    StateKey::new("create")?,
                    NoContext,
                    composition_transaction_config(sender),
                    create_action,
                    evm_sender_lane_resource_claim()?,
                    SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            let target = root.scope().state::<DirectValidationTargetState, _>(
                StateKey::new("validation-target")?,
                NoContext,
                ProjectionConfig {},
                created_outcome,
            )?;
            let verified = root.scope().state::<ValidateEvmContractState, _>(
                StateKey::new("validate")?,
                NoContext,
                composition_validation_config(sender, created),
                target,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TransactionCompositionPublicOutputs { verified },
            )
        },
    )
    .expect("direct transaction composition draft");
    let seed_material = BTreeMap::from([(draft.seeds()[0].seed_id.clone(), create_bytes)]);
    (draft, seed_material)
}

fn call_validate_launch_material(sender: Address, created: Address) -> CompositionLaunchMaterial {
    let call_action = CanonicalSeed::from_value(
        &EvmTransactionAction::call(created, [0x01], U256::ZERO).expect("call action"),
    )
    .expect("call action seed");
    let call_bytes = call_action.canonical_json().clone();
    let draft = build_root_with_registries(
        ScopeKey::new("evm-call-validation-composition").expect("scope key"),
        composition_state_registry(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let call_action = root.seed(SeedKey::new("call-action")?, call_action.clone())?;
            let call_outcome = root
                .scope()
                .side_effect::<SubmitEvmTransactionState, _>(
                    StateKey::new("call")?,
                    NoContext,
                    composition_transaction_config(sender),
                    call_action,
                    evm_sender_lane_resource_claim()?,
                    SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            let target = root.scope().state::<DirectValidationTargetState, _>(
                StateKey::new("validation-target")?,
                NoContext,
                ProjectionConfig {},
                call_outcome,
            )?;
            let verified = root.scope().state::<ValidateEvmContractState, _>(
                StateKey::new("validate")?,
                NoContext,
                composition_validation_config(sender, created),
                target,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TransactionCompositionPublicOutputs { verified },
            )
        },
    )
    .expect("call-validation composition draft");
    let seed_material = BTreeMap::from([(draft.seeds()[0].seed_id.clone(), call_bytes)]);
    (draft, seed_material)
}

fn composition_launch_material(sender: Address, created: Address) -> CompositionLaunchMaterial {
    let create_action = CanonicalSeed::from_value(
        &EvmTransactionAction::create([0x60, 0x00], U256::ZERO).expect("create action"),
    )
    .expect("create action seed");
    let create_bytes = create_action.canonical_json().clone();
    let draft = build_root_with_registries(
        ScopeKey::new("evm-transaction-composition").expect("scope key"),
        composition_state_registry(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let create_action = root.seed(SeedKey::new("create-action")?, create_action.clone())?;
            let created_outcome = root
                .scope()
                .side_effect::<SubmitEvmTransactionState, _>(
                    StateKey::new("create")?,
                    NoContext,
                    composition_transaction_config(sender),
                    create_action,
                    evm_sender_lane_resource_claim()?,
                    SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            let first_action = root.scope().state::<FirstCallActionState, _>(
                StateKey::new("first-call-action")?,
                NoContext,
                ProjectionConfig {},
                created_outcome.clone(),
            )?;
            let first_outcome = root
                .scope()
                .side_effect::<SubmitEvmTransactionState, _>(
                    StateKey::new("first-call")?,
                    NoContext,
                    composition_transaction_config(sender),
                    first_action,
                    evm_sender_lane_resource_claim()?,
                    SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            let second_action = root.scope().state::<SecondCallActionState, _>(
                StateKey::new("second-call-action")?,
                NoContext,
                ProjectionConfig {},
                first_outcome,
            )?;
            let final_outcome = root
                .scope()
                .side_effect::<SubmitEvmTransactionState, _>(
                    StateKey::new("second-call")?,
                    NoContext,
                    composition_transaction_config(sender),
                    second_action,
                    evm_sender_lane_resource_claim()?,
                    SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            let target = root.scope().state::<ValidationTargetState, _>(
                StateKey::new("validation-target")?,
                NoContext,
                ProjectionConfig {},
                ValidationTargetInputHandles {
                    created: created_outcome,
                    final_call: final_outcome,
                },
            )?;
            let verified = root.scope().state::<ValidateEvmContractState, _>(
                StateKey::new("validate")?,
                NoContext,
                composition_validation_config(sender, created),
                target,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TransactionCompositionPublicOutputs { verified },
            )
        },
    )
    .expect("transaction composition draft");
    let seed_material = BTreeMap::from([(draft.seeds()[0].seed_id.clone(), create_bytes)]);
    (draft, seed_material)
}

fn composition_state_registry() -> mfm_program::StateRegistrySnapshot {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<SubmitEvmTransactionState>()
        .expect("register transaction state");
    states
        .register::<FirstCallActionState>()
        .expect("register first-call projection");
    states
        .register::<SecondCallActionState>()
        .expect("register second-call projection");
    states
        .register::<ValidationTargetState>()
        .expect("register validation-target projection");
    states
        .register::<DirectValidationTargetState>()
        .expect("register direct validation-target projection");
    states
        .register::<ValidateEvmContractState>()
        .expect("register validation state");
    states.snapshot()
}

fn composition_transaction_config(sender: Address) -> EvmTransactionConfig {
    EvmTransactionConfig::new(
        "ethereum-mainnet",
        1,
        sender,
        mfm_signing::SignerRef::new("composition-signer").expect("signer ref"),
        Vec::new(),
    )
    .expect("transaction config")
}

fn composition_validation_config(sender: Address, created: Address) -> EvmContractValidationConfig {
    EvmContractValidationConfig::new(
        "ethereum-mainnet",
        1,
        keccak256(RUNTIME_CODE),
        vec![EvmContractCallCheck::new(
            sender,
            created,
            U256::ZERO,
            VALIDATION_CALLDATA,
            U256::from(100_000),
            Vec::new(),
            VALIDATION_RETURN,
        )
        .expect("validation call")],
    )
    .expect("validation config")
}

fn composition_certification_registry() -> CertificationRegistry {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<SubmitEvmTransactionState>()
        .expect("certify transaction state");
    registry
        .register_state::<FirstCallActionState>()
        .expect("certify first-call projection");
    registry
        .register_state::<SecondCallActionState>()
        .expect("certify second-call projection");
    registry
        .register_state::<ValidationTargetState>()
        .expect("certify validation-target projection");
    registry
        .register_state::<DirectValidationTargetState>()
        .expect("certify direct validation-target projection");
    registry
        .register_state::<ValidateEvmContractState>()
        .expect("certify validation state");
    registry
}

fn composition_runners(
    world: Arc<TransactionCompositionWorld>,
    signing_key: Arc<SigningKey>,
    live_validation_reads: Arc<AtomicUsize>,
) -> ErasedRunnerRegistry {
    let mut runners = test_runner_registry();
    let pure_factory = test_factory_binding(&runners, "pure");
    let read_factory = test_factory_binding(&runners, "read_external");
    let side_effect_factory = test_factory_binding(&runners, "apply_side_effect");
    let adapter_factory = test_factory_binding(&runners, "evm_jsonrpc_adapter");
    register_projection_runners(&mut runners, &pure_factory);

    let sender = world.sender;
    let transaction_session = Arc::new(CompositionTransactionSession::new(Arc::clone(&world)));
    let signer = Arc::new(CompositionSigner {
        signing_key,
        sender,
    });
    let signer_binder = mfm_signing::DeterministicSigningProviderBinder::new(
        "mfm.test.composition-signer",
        move |signer_ref| {
            let signer = Arc::clone(&signer);
            Box::pin(async move {
                if signer_ref.as_str() != "composition-signer" {
                    return Err(mfm_signing::SigningError::redacted_provider_failure(
                        "unexpected signer reference",
                    ));
                }
                Ok(signer as Arc<dyn DeterministicSigningProvider>)
            })
        },
    )
    .expect("signer binder");
    register_evm_transaction_runner(
        &mut runners,
        signer_binder,
        move |binding, signer_ref| {
            Box::pin(async move {
                if binding.network_id().as_str() == "ethereum-mainnet"
                    && binding.expected_chain_id() == 1
                    && signer_ref.as_str() == "composition-signer"
                {
                    Ok(())
                } else {
                    Err(mfm_runtime::RuntimeError::RunnerBinding(
                        "unexpected composition transaction binding".to_owned(),
                    ))
                }
            })
        },
        Arc::new(CompositionTransactionSessions {
            session: transaction_session,
        }),
        &side_effect_factory,
        &read_factory,
        &adapter_factory,
    )
    .expect("register transaction runner");

    register_evm_validation_runner(
        &mut runners,
        Arc::new(CompositionReadSessions {
            world,
            reads: live_validation_reads,
        }),
        &read_factory,
        &adapter_factory,
    )
    .expect("register validation runner");
    runners
}

fn register_projection_runners(
    registry: &mut ErasedRunnerRegistry,
    pure_factory: &mfm_runtime::RunnerFactoryBinding,
) {
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    register_pure_state::<FirstCallActionState>(&mut registrations, pure_factory, None)
        .expect("register first-call projection runner");
    register_pure_state::<SecondCallActionState>(&mut registrations, pure_factory, None)
        .expect("register second-call projection runner");
    register_pure_state::<ValidationTargetState>(&mut registrations, pure_factory, None)
        .expect("register validation-target projection runner");
    register_pure_state::<DirectValidationTargetState>(&mut registrations, pure_factory, None)
        .expect("register direct validation-target projection runner");
}

fn validate_composition_binding(binding: &EvmNetworkBinding) -> mfm_evm::EvmCapabilityResult<()> {
    if binding.network_id().as_str() == "ethereum-mainnet" && binding.expected_chain_id() == 1 {
        Ok(())
    } else {
        Err(composition_provider_failure())
    }
}

fn composition_provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationInvalid,
    ))
}

struct PendingTransaction {
    nonce: u64,
    estimate: EvmTransactionEstimate,
}

#[derive(Clone)]
struct IncludedTransaction {
    transaction: EvmObservedTransaction,
    receipt: EvmReceipt,
}

#[derive(Default)]
struct CompositionChain {
    pending: Option<PendingTransaction>,
    included: BTreeMap<B256, IncludedTransaction>,
}

struct TransactionCompositionWorld {
    sender: Address,
    created: Address,
    revert_first_call: bool,
    chain: Mutex<CompositionChain>,
}

impl TransactionCompositionWorld {
    fn new(sender: Address, created: Address) -> Self {
        Self {
            sender,
            created,
            revert_first_call: false,
            chain: Mutex::new(CompositionChain::default()),
        }
    }

    fn reverting_first_call(sender: Address, created: Address) -> Self {
        Self {
            sender,
            created,
            revert_first_call: true,
            chain: Mutex::new(CompositionChain::default()),
        }
    }

    fn included_count(&self) -> usize {
        self.chain.lock().expect("composition chain").included.len()
    }

    fn receipt_statuses(&self) -> Vec<EvmReceiptStatus> {
        let mut statuses = self
            .chain
            .lock()
            .expect("composition chain")
            .included
            .values()
            .map(|included| (included.receipt.transaction_index, included.receipt.status))
            .collect::<Vec<_>>();
        statuses.sort_by_key(|(index, _)| *index);
        statuses.into_iter().map(|(_, status)| status).collect()
    }

    fn block_by_number(&self, number: U256) -> Option<EvmBlockAnchor> {
        self.chain
            .lock()
            .expect("composition chain")
            .included
            .values()
            .find(|included| included.receipt.block.number_quantity() == Ok(number))
            .map(|included| included.receipt.block.clone())
    }

    fn has_block_hash(&self, hash: B256) -> bool {
        self.chain
            .lock()
            .expect("composition chain")
            .included
            .values()
            .any(|included| included.receipt.block.hash_value() == Ok(hash))
    }
}

struct CompositionTransactionSession {
    evidence: EvmSessionEvidence,
    world: Arc<TransactionCompositionWorld>,
}

struct CompositionTransactionSessions {
    session: Arc<CompositionTransactionSession>,
}

impl EvmTransactionSessionSet for CompositionTransactionSessions {
    fn implementation_id(&self) -> &str {
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    }

    fn session<'a>(
        &'a self,
        binding: &'a EvmNetworkBinding,
    ) -> EvmSessionFuture<'a, Arc<dyn EvmTransactionSession>> {
        Box::pin(async move {
            validate_composition_binding(binding)?;
            Ok(Arc::clone(&self.session) as Arc<dyn EvmTransactionSession>)
        })
    }
}

impl CompositionTransactionSession {
    fn new(world: Arc<TransactionCompositionWorld>) -> Self {
        let binding = EvmNetworkBinding::new(
            mfm_ids::LocalPublicId::new("ethereum-mainnet").expect("network id"),
            1,
        )
        .expect("network binding");
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("composition").expect("source ref"),
                mfm_ids::LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            world,
        }
    }
}

impl EvmTransactionSession for CompositionTransactionSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn pending_nonce<'a>(&'a self, account: Address) -> EvmSessionFuture<'a, U256> {
        assert_eq!(account, self.world.sender);
        let nonce = BASE_NONCE
            + u64::try_from(self.world.included_count()).expect("included transaction count");
        Box::pin(std::future::ready(Ok(U256::from(nonce))))
    }

    fn fee_inputs(&self) -> EvmSessionFuture<'_, EvmFeeInputs> {
        Box::pin(async {
            EvmFeeInputs::from_base_and_priority(
                U256::from(9_500_000_000_u64),
                U256::from(1_000_000_000_u64),
            )
        })
    }

    fn estimate_gas<'a>(
        &'a self,
        request: &'a EvmTransactionEstimate,
    ) -> EvmSessionFuture<'a, U256> {
        let nonce = BASE_NONCE
            + u64::try_from(self.world.included_count()).expect("included transaction count");
        self.world.chain.lock().expect("composition chain").pending = Some(PendingTransaction {
            nonce,
            estimate: request.clone(),
        });
        Box::pin(std::future::ready(Ok(U256::from(100_000))))
    }

    fn submit_raw_transaction<'a>(
        &'a self,
        signed_bytes: &'a [u8],
        expected_hash: B256,
    ) -> EvmSessionFuture<'a, B256> {
        assert_eq!(keccak256(signed_bytes), expected_hash);
        let mut chain = self.world.chain.lock().expect("composition chain");
        let pending = chain.pending.take().expect("pending transaction estimate");
        let transaction_index = u64::try_from(chain.included.len()).expect("transaction index");
        let block_number = U256::from(100 + transaction_index);
        let mut block_hash = [0_u8; 32];
        block_hash[31] = u8::try_from(transaction_index + 1).expect("block hash marker");
        let block_hash = B256::from(block_hash);
        let contract_address = match pending.estimate.to() {
            TxKind::Create => Some(self.world.sender.create(pending.nonce)),
            TxKind::Call(_) => None,
        };
        let to = match pending.estimate.to() {
            TxKind::Create => None,
            TxKind::Call(address) => Some(address),
        };
        let transaction = EvmObservedTransaction {
            transaction_hash: expected_hash,
            chain_id: U256::from(1),
            nonce: U256::from(pending.nonce),
            from: self.world.sender,
            to: pending.estimate.to(),
            value: pending.estimate.value(),
            input: pending.estimate.input().clone(),
            gas_limit: U256::from(100_000),
            max_fee_per_gas: U256::from(20_000_000_000_u64),
            max_priority_fee_per_gas: U256::from(1_000_000_000_u64),
            access_list: pending.estimate.access_list().clone(),
            placement: None,
        };
        let receipt = EvmReceipt {
            transaction_hash: expected_hash,
            transaction_index: U256::from(transaction_index),
            block: EvmBlockAnchor::new(block_number, block_hash),
            from: self.world.sender,
            to,
            contract_address,
            status: if self.world.revert_first_call && transaction_index == 1 {
                EvmReceiptStatus::Reverted
            } else {
                EvmReceiptStatus::Success
            },
            gas_used: U256::from(80_000),
            cumulative_gas_used: U256::from(80_000 * (transaction_index + 1)),
            logs: Vec::new(),
        };
        chain.included.insert(
            expected_hash,
            IncludedTransaction {
                transaction,
                receipt,
            },
        );
        Box::pin(std::future::ready(Ok(expected_hash)))
    }

    fn transaction_by_hash(
        &self,
        transaction_hash: B256,
    ) -> EvmSessionFuture<'_, Option<EvmObservedTransaction>> {
        let transaction = self
            .world
            .chain
            .lock()
            .expect("composition chain")
            .included
            .get(&transaction_hash)
            .map(|included| included.transaction.clone());
        Box::pin(std::future::ready(Ok(transaction)))
    }

    fn receipt_by_hash(&self, transaction_hash: B256) -> EvmSessionFuture<'_, Option<EvmReceipt>> {
        let receipt = self
            .world
            .chain
            .lock()
            .expect("composition chain")
            .included
            .get(&transaction_hash)
            .map(|included| included.receipt.clone());
        Box::pin(std::future::ready(Ok(receipt)))
    }

    fn read_block<'a>(
        &'a self,
        selector: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmBlockAnchor> {
        let result = match selector {
            EvmBlockSelector::Number(number) => self
                .world
                .block_by_number(*number)
                .ok_or_else(composition_provider_failure),
            _ => Err(composition_provider_failure()),
        };
        Box::pin(std::future::ready(result))
    }
}

struct CompositionReadSession {
    evidence: EvmSessionEvidence,
    world: Arc<TransactionCompositionWorld>,
    reads: Arc<AtomicUsize>,
}

struct CompositionReadSessions {
    world: Arc<TransactionCompositionWorld>,
    reads: Arc<AtomicUsize>,
}

impl EvmReadSessionSet for CompositionReadSessions {
    fn implementation_id(&self) -> &str {
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    }

    fn validate_binding<'a>(&'a self, binding: &'a EvmNetworkBinding) -> EvmSessionFuture<'a, ()> {
        Box::pin(async move { validate_composition_binding(binding) })
    }

    fn session<'a>(
        &'a self,
        binding: &'a EvmNetworkBinding,
    ) -> EvmSessionFuture<'a, Arc<dyn EvmReadSession>> {
        Box::pin(async move {
            validate_composition_binding(binding)?;
            Ok(Arc::new(CompositionReadSession::new(
                binding.clone(),
                Arc::clone(&self.world),
                Arc::clone(&self.reads),
            )) as Arc<dyn EvmReadSession>)
        })
    }
}

impl CompositionReadSession {
    fn new(
        binding: EvmNetworkBinding,
        world: Arc<TransactionCompositionWorld>,
        reads: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("composition").expect("source ref"),
                mfm_ids::LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            world,
            reads,
        }
    }

    fn record_read(&self) {
        self.reads.fetch_add(1, Ordering::SeqCst);
    }
}

impl EvmReadSession for CompositionReadSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(
        &'a self,
        selector: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmBlockAnchor> {
        self.record_read();
        let result = match selector {
            EvmBlockSelector::Number(number) => self
                .world
                .block_by_number(*number)
                .ok_or_else(composition_provider_failure),
            _ => Err(composition_provider_failure()),
        };
        Box::pin(std::future::ready(result))
    }

    fn read_balance<'a>(
        &'a self,
        _account: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        Box::pin(std::future::ready(Err(composition_provider_failure())))
    }

    fn read_code<'a>(
        &'a self,
        address: Address,
        block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        self.record_read();
        let result = if address == self.world.created
            && matches!(block, EvmBlockSelector::ExactHash(hash) if self.world.has_block_hash(*hash))
        {
            Ok(EvmCode {
                bytes: Bytes::copy_from_slice(RUNTIME_CODE),
                hash: keccak256(RUNTIME_CODE),
            })
        } else {
            Err(composition_provider_failure())
        };
        Box::pin(std::future::ready(result))
    }

    fn call<'a>(&'a self, request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        self.record_read();
        let result = if request.from() == self.world.sender
            && request.to() == self.world.created
            && request.value().is_zero()
            && request.input().as_ref() == VALIDATION_CALLDATA
            && request.gas_limit() == U256::from(100_000)
            && request.access_list().is_empty()
            && matches!(request.block(), EvmBlockSelector::ExactHash(hash) if self.world.has_block_hash(*hash))
        {
            Ok(Bytes::copy_from_slice(VALIDATION_RETURN))
        } else {
            Err(composition_provider_failure())
        };
        Box::pin(std::future::ready(result))
    }
}

struct CompositionSigner {
    signing_key: Arc<SigningKey>,
    sender: Address,
}

impl SigningProvider for CompositionSigner {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        let result = self
            .signing_key
            .sign_prehash_recoverable(request.digest().as_bytes())
            .map_err(mfm_signing::SigningError::redacted_provider_failure)
            .and_then(|(signature, recovery_id)| {
                if signature.normalize_s().is_some() {
                    return Err(mfm_signing::SigningError::redacted_provider_failure(
                        "non-canonical signature",
                    ));
                }
                let signature = PrimitiveSignature::from_signature_and_parity(
                    signature,
                    recovery_id.is_y_odd(),
                );
                let identity = PublicSigningIdentity::new(
                    request.algorithm().clone(),
                    None,
                    Some(format!("{:?}", self.sender)),
                )?;
                SigningResult::for_request(
                    request,
                    identity,
                    SignatureBytes::new(signature.as_bytes().to_vec())?,
                )
            });
        Box::pin(std::future::ready(result))
    }
}

impl DeterministicSigningProvider for CompositionSigner {
    fn implementation_id(&self) -> &'static str {
        "mfm.test.composition-signer"
    }

    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

fn signing_key_address(signing_key: &SigningKey) -> Address {
    let encoded = signing_key.verifying_key().to_encoded_point(false);
    let digest = keccak256(&encoded.as_bytes()[1..]);
    Address::from_slice(&digest.as_slice()[12..])
}
