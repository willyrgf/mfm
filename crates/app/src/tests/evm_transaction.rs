use super::*;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alloy_primitives::{
    address, b256, hex, keccak256, Address, PrimitiveSignature, TxKind, B256, U256,
};
use k256::ecdsa::SigningKey as ManualSigningKey;
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::NoCaps;
use mfm_certify::CertificationRegistry;
use mfm_events::v1 as events;
use mfm_evm::{
    evm_sender_lane_resource_claim, EvmBlockAnchor, EvmBlockSelector, EvmFeeInputs,
    EvmNetworkBinding, EvmObservedTransaction, EvmReceipt, EvmReceiptStatus, EvmSessionEvidence,
    EvmSessionFuture, EvmTransactionAction, EvmTransactionConfig, EvmTransactionEstimate,
    EvmTransactionOutcome, EvmTransactionSession, EvmTransactionSessionSet,
    SubmitEvmTransactionState, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_evm_live::register_evm_transaction_runner;
use mfm_manual_auth::{
    manual_authorization_proof_schema_id, ManualAuthorizationSignatureBytes,
    ManualResolutionAuthorizationClaim, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionEvidenceRef,
    MANUAL_RESOLUTION_DIGEST_SIGNATURE_SCHEME,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, InputBindingNodeRef, ManualAuthorizationDraft,
    ManualAuthorizationVerifierId, ManualResolutionPolicyDraft, ManualSigningSchemeSpec, NoContext,
    NonEmptyUniqueOperators, OperatorAuthorityId, OperatorAuthorityMemberSpec,
    OperatorAuthoritySnapshotDraft, OperatorId, OperatorPublicIdentity, PublicOutputKey, PureState,
    RemediationNodeParams, RemediationUnresolved, RootBuilder, ScopeKey, SeedKey,
    SideEffectNodeParams, SideEffectSagaPolicy, SideEffectVerificationSpec, StateKey,
    StateRegistryBuilder, ThresholdQuorum, ValidatedConfig,
};
use mfm_program_derive::{PublicOutputs, StateInput};
use mfm_runtime::{register_pure_state, ErasedRunnerRegistry, RunnerRegistrationBuilder};
use mfm_signing::{
    DeterministicSigningProvider, PublicSigningIdentity, SignatureBytes, SigningFuture,
    SigningProvider, SigningRequest, SigningResult, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use mfm_store::v1::{self as store, StoreScopeStore as _};

const EXPECTED_SENDER: Address = address!("dd6b8b3dc6b7ad97db52f08a275ff4483e024cea");
const DESTINATION: Address = address!("6069a6c32cf691f5982febae4faf8a6f3ab2f0f6");
const EXPECTED_HASH: B256 =
    b256!("0ec0b6a2df4d87424e5f6ad2a654e27aaeb7dac20ae9e8385cc09087ad532ee0");
const RECEIPT_BLOCK_HASH: B256 =
    b256!("1111111111111111111111111111111111111111111111111111111111111111");

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.app.test.evm_transaction_public_outputs")]
struct EvmTransactionPublicOutputs<'program, 'scope> {
    outcome: mfm_program::Handle<'program, 'scope, EvmTransactionOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.app.test.evm_transaction_sequence_input")]
struct SequenceTransactionInput {
    previous: EvmTransactionOutcome,
    next: EvmTransactionAction,
}

struct SequenceTransactionState;

impl StateSpec for SequenceTransactionState {
    type Config = SequenceTransactionConfig;
    type Context = NoContext;
    type Input = SequenceTransactionInput;
    type Output = EvmTransactionAction;
    type Effect = mfm_capabilities::Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        mfm_ids::StateKind::new(
            "mfm.app.test",
            "evm-transaction-sequence",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.app.test.evm-transaction-sequence"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        mfm_ids::StateVersion::new("mfm.app.test.evm_transaction_sequence.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.app.test.evm_transaction_sequence"
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for SequenceTransactionState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        if !matches!(
            input.previous.result(),
            mfm_evm::EvmTransactionResult::Succeeded { .. }
        ) {
            return Err(mfm_program::StateError::Message(
                "the preceding EVM transaction did not succeed".to_owned(),
            ));
        }
        Ok(input.next)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct SequenceTransactionConfig {}

#[test]
fn configuration_calls_are_separate_dependency_ordered_transaction_nodes() {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<SubmitEvmTransactionState>()
        .expect("register transaction state");
    states
        .register::<SequenceTransactionState>()
        .expect("register sequencing state");
    let first_action = CanonicalSeed::from_value(
        &EvmTransactionAction::call(DESTINATION, [0x01], U256::ZERO).expect("first call action"),
    )
    .expect("first action seed");
    let second_action = CanonicalSeed::from_value(
        &EvmTransactionAction::call(DESTINATION, [0x02], U256::ZERO).expect("second call action"),
    )
    .expect("second action seed");
    let draft = build_root_with_registries(
        ScopeKey::new("evm-call-sequence").expect("scope key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let first_action = root.seed(SeedKey::new("first-action")?, first_action.clone())?;
            let second_action = root.seed(SeedKey::new("second-action")?, second_action.clone())?;
            let first = root.scope().side_effect::<SubmitEvmTransactionState, _>(
                StateKey::new("configure-0")?,
                NoContext,
                transaction_config(),
                first_action,
                evm_sender_lane_resource_claim()?,
                SideEffectVerificationSpec::Receipt,
            )?;
            let next = root.scope().state::<SequenceTransactionState, _>(
                StateKey::new("after-configure-0")?,
                NoContext,
                SequenceTransactionConfig {},
                SequenceTransactionInputHandles {
                    previous: first.into_handle(),
                    next: second_action,
                },
            )?;
            let second = root.scope().side_effect::<SubmitEvmTransactionState, _>(
                StateKey::new("configure-1")?,
                NoContext,
                transaction_config(),
                next,
                evm_sender_lane_resource_claim()?,
                SideEffectVerificationSpec::Receipt,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &EvmTransactionPublicOutputs {
                    outcome: second.into_handle(),
                },
            )
        },
    )
    .expect("sequential transaction draft");

    let transaction_kind = SubmitEvmTransactionState::kind().expect("transaction kind");
    let transaction_nodes = draft
        .state_nodes()
        .iter()
        .filter(|node| node.state_kind == transaction_kind)
        .collect::<Vec<_>>();
    assert_eq!(transaction_nodes.len(), 2);
    assert_eq!(transaction_nodes[0].key.as_str(), "configure-0");
    assert_eq!(transaction_nodes[1].key.as_str(), "configure-1");

    let sequence = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "after-configure-0")
        .expect("sequence node");
    let first_terminal = transaction_nodes[0]
        .side_effect_verify
        .as_ref()
        .expect("first verification node")
        .output_cell_id
        .as_str();
    let sequence_inputs = state_input_cells(sequence);
    assert!(sequence_inputs.contains(&first_terminal));
    assert_eq!(
        state_input_cells(transaction_nodes[1]),
        vec![sequence.output_cell_id.as_str()]
    );
}

#[test]
fn compensation_is_a_structurally_separate_transaction_node() {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<SubmitEvmTransactionState>()
        .expect("register transaction state");
    let action = CanonicalSeed::from_value(
        &EvmTransactionAction::call(DESTINATION, [0x03], U256::ZERO).expect("transaction action"),
    )
    .expect("transaction action seed");
    let draft = build_root_with_registries(
        ScopeKey::new("evm-compensation").expect("scope key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::CompensateCompleted {
                on_remediation_unresolved: RemediationUnresolved::FailWithoutAcdcClaim,
            })?;
            let action = root.seed(SeedKey::new("action")?, action.clone())?;
            let (forward, _remediation) = root.scope().side_effect_with_compensation::<
                SubmitEvmTransactionState,
                SubmitEvmTransactionState,
                _,
                _,
                _,
            >(
                NoContext,
                NoContext,
                SideEffectNodeParams {
                    key: StateKey::new("forward")?,
                    config: transaction_config(),
                    input: action.clone(),
                    resource_claim: evm_sender_lane_resource_claim()?,
                    verification: SideEffectVerificationSpec::Receipt,
                },
                RemediationNodeParams {
                    key: StateKey::new("compensate-forward")?,
                    config: transaction_config(),
                    resource_claim: evm_sender_lane_resource_claim()?,
                    verification: SideEffectVerificationSpec::Receipt,
                },
                |_forward| Ok(action),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &EvmTransactionPublicOutputs {
                    outcome: forward.into_handle(),
                },
            )
        },
    )
    .expect("compensating transaction draft");

    assert_eq!(draft.state_nodes().len(), 1);
    assert_eq!(draft.remediation_nodes().len(), 1);
    let forward = &draft.state_nodes()[0];
    let remediation = draft
        .remediation_nodes()
        .get(&forward.node_id)
        .expect("linked transaction remediation");
    assert_eq!(
        forward.state_kind,
        SubmitEvmTransactionState::kind().unwrap()
    );
    assert_eq!(remediation.state_kind, forward.state_kind);
    assert_ne!(remediation.node_id, forward.node_id);
    assert_eq!(remediation.key.as_str(), "compensate-forward");
}

#[tokio::test]
async fn certified_transaction_recovers_lost_submit_response_without_rebroadcast() {
    let store = store::AsyncInMemoryRunStore::default();
    let session = Arc::new(TransactionSession::new());
    session.fail_next_submit_response();
    let signer_calls = Arc::new(AtomicUsize::new(0));
    let certification = transaction_certification_registry();
    let launch_services = make_run_services(
        transaction_runners(Arc::clone(&session), Arc::clone(&signer_calls)),
        Arc::new(store.clone()),
        certification.clone(),
    );
    let (draft, seed_material) = transaction_launch_material();
    let request = crate::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        launch_services.certification_registry(),
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("transaction launch request");
    let run_id = request.run_id.clone();
    mfm_runtime::BoundRuntimeContextLoader::new(transaction_runners(
        Arc::clone(&session),
        Arc::clone(&signer_calls),
    ))
    .load(&request.runtime_spec)
    .expect("bind transaction runtime");

    let launch = launch_services
        .launch_run(request)
        .await
        .expect("launch transaction graph");
    let (_, launched, _) = launch.into_response_parts();
    let launched = launched.expect("admitted transaction run");
    assert_eq!(launched.run_mode, RunModeStatus::Forward);
    assert_eq!(launched.scheduler_status, "blocked");
    let blocked_view = load_verified_run_view(&store, &certification, &run_id)
        .await
        .expect("verified blocked run");
    let blocked_lifecycle = store::current_lifecycle::read(&blocked_view);
    let mut submission_unknown = false;
    let mut attempt_failed = false;
    let _ = blocked_lifecycle.visit_records(|record| {
        match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::SideEffectSubmissionUnknown(_) => {
                submission_unknown = true;
            }
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptFailed(_) => {
                attempt_failed = true;
            }
            _ => {}
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert!(submission_unknown);
    assert!(!attempt_failed);
    assert_eq!(signer_calls.load(Ordering::SeqCst), 1);
    drop(launch_services);
    assert!(store
        .expire_execution_claim_for_test(&run_id)
        .expect("expire abandoned execution claim"));

    // Recovery observes the already accepted transaction and never reconstructs a signer or
    // invokes the mutation capability again.
    session.make_transaction_visible();
    let resume_services = make_run_services(
        transaction_runners(Arc::clone(&session), Arc::clone(&signer_calls)),
        Arc::new(store.clone()),
        certification.clone(),
    );
    let resumed = resume_services
        .resume_stored_run(&run_id)
        .await
        .expect("resume unknown transaction");
    assert_eq!(resumed.run_mode, RunModeStatus::Completed);
    assert_eq!(signer_calls.load(Ordering::SeqCst), 1);
    {
        let submissions = session.submitted.lock().expect("submitted bytes");
        assert_eq!(submissions.len(), 1);
    }
    drop(resume_services);

    // Read-only replay has no runner registry, transaction session, or signer binder.
    let replay_services = make_run_read_services(Arc::new(store.clone()), certification.clone());
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only transaction replay");
    assert_eq!(replay.run_mode, RunModeStatus::Completed);
    replay_services
        .inspect_replay_broker_for_test(&run_id, mfm_evm_live::verify_evm_transaction_replay)
        .await
        .expect("transaction replay broker")
        .expect("explicit transaction foundation replay verifier");
}

#[tokio::test]
async fn invalid_historical_manual_authorization_cannot_render_manually_resolved_status() {
    let store = store::AsyncInMemoryRunStore::default();
    let mut transaction_session = TransactionSession::new();
    transaction_session.make_visible_on_submit = AtomicBool::new(true);
    transaction_session.receipt_status = EvmReceiptStatus::Reverted;
    let transaction_session = Arc::new(transaction_session);
    let signer_calls = Arc::new(AtomicUsize::new(0));
    let certification = manual_transaction_certification_registry();
    let services = make_run_services(
        transaction_runners(Arc::clone(&transaction_session), Arc::clone(&signer_calls)),
        Arc::new(store.clone()),
        certification.clone(),
    );
    let (draft, seed_material) = manual_transaction_launch_material();
    let request = crate::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        services.certification_registry(),
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("manual transaction launch request");
    let run_id = request.run_id.clone();

    let launch = services
        .launch_run(request)
        .await
        .expect("launch reverted manual transaction");
    let (_, launched, _) = launch.into_response_parts();
    assert_eq!(
        launched.expect("admitted manual transaction").run_mode,
        RunModeStatus::ManualBlocked
    );

    let blocked_view = load_verified_run_view(&store, &certification, &run_id)
        .await
        .expect("verified manually blocked run");
    let prefix = store::current_lifecycle::read(&blocked_view)
        .manual_resolution_prefix_authority()
        .expect("manual resolution prefix");
    let evidence_bytes = br#"{"decision":"reviewed"}"#.to_vec();
    let evidence = manual_resolution_content_ref(
        prefix.manual_policy().evidence_schema.clone(),
        &evidence_bytes,
    );
    let claim = prefix
        .authorization_claim(events::ManualResolutionOutcome::ConfirmRemediated, evidence)
        .expect("manual authorization claim");
    let valid_proof = manual_authorization_proof_bytes(
        &prefix.manual_policy().authorization,
        claim.clone(),
        manual_claim_signature(&claim),
    );
    drop(blocked_view);

    let resolved = services
        .record_manual_resolution(ManualResolutionRecordRequest {
            run_id: run_id.clone(),
            outcome: ManualResolutionDecision::ConfirmRemediated,
            evidence_bytes,
            evidence_media_type: "application/json".to_owned(),
            authorization_proof_bytes: valid_proof,
            note: None,
        })
        .await
        .expect("record valid manual resolution");
    assert_eq!(resolved.run_mode, RunModeStatus::ManuallyResolved);
    let valid_status = make_run_read_services(Arc::new(store.clone()), certification.clone())
        .run_status(&run_id)
        .await
        .expect("valid historical manual resolution status");
    assert_eq!(valid_status.run_mode, RunModeStatus::ManuallyResolved);

    let invalid_proof = manual_authorization_proof_bytes(
        &prefix.manual_policy().authorization,
        claim,
        vec![0_u8; 65],
    );
    let invalid_store =
        InvalidManualHistoryStore::new(&store, &run_id, invalid_proof).expect("invalid history");
    store::RunJournalStore::load_committed_journal(&invalid_store, &run_id)
        .await
        .expect("content-addressed invalid authorization journal loads structurally");
    let error = make_run_read_services(Arc::new(invalid_store), certification)
        .run_status(&run_id)
        .await
        .expect_err("invalid historical authorization must not render status");
    assert_eq!(error.code, "RunStoreRejected");
}

#[derive(Clone, Copy)]
enum ObservationOutage {
    Receipt = 1,
    CanonicalBlock = 2,
    Head = 3,
}

#[tokio::test]
async fn post_submission_provider_outages_block_and_resume_without_rebroadcast() {
    for outage in [
        ObservationOutage::Receipt,
        ObservationOutage::CanonicalBlock,
        ObservationOutage::Head,
    ] {
        assert_post_submission_outage_resumes(outage).await;
    }
}

async fn assert_post_submission_outage_resumes(outage: ObservationOutage) {
    let store = store::AsyncInMemoryRunStore::default();
    let session = Arc::new(TransactionSession::new());
    session.make_visible_on_submit.store(true, Ordering::SeqCst);
    session.fail_once(outage);
    let signer_calls = Arc::new(AtomicUsize::new(0));
    let certification = transaction_certification_registry();
    let launch_services = make_run_services(
        transaction_runners(Arc::clone(&session), Arc::clone(&signer_calls)),
        Arc::new(store.clone()),
        certification.clone(),
    );
    let (draft, seed_material) = transaction_launch_material();
    let request = crate::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        launch_services.certification_registry(),
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("transaction launch request");
    let run_id = request.run_id.clone();

    let launch = launch_services
        .launch_run(request)
        .await
        .expect("provider outage is an operational block");
    let (_, launched, _) = launch.into_response_parts();
    let launched = launched.expect("admitted transaction run");
    assert_eq!(launched.run_mode, RunModeStatus::Forward);
    assert_eq!(launched.scheduler_status, "blocked");
    assert_eq!(session.submitted.lock().expect("submitted bytes").len(), 1);
    assert_eq!(signer_calls.load(Ordering::SeqCst), 1);
    let blocked_view = load_verified_run_view(&store, &certification, &run_id)
        .await
        .expect("verified blocked run");
    let blocked_lifecycle = store::current_lifecycle::read(&blocked_view);
    let mut submission_observed = false;
    let mut attempt_failed = false;
    let _ = blocked_lifecycle.visit_records(|record| {
        match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::SideEffectSubmissionObserved(_) => {
                submission_observed = true;
            }
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptFailed(_) => {
                attempt_failed = true;
            }
            _ => {}
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert!(submission_observed);
    assert!(!attempt_failed);
    drop(launch_services);
    assert!(store
        .expire_execution_claim_for_test(&run_id)
        .expect("expire blocked execution claim"));

    let resume_services = make_run_services(
        transaction_runners(Arc::clone(&session), Arc::clone(&signer_calls)),
        Arc::new(store.clone()),
        certification,
    );
    let resumed = resume_services
        .resume_stored_run(&run_id)
        .await
        .expect("healthy provider resumes observation");
    assert_eq!(resumed.run_mode, RunModeStatus::Completed);
    assert_eq!(session.submitted.lock().expect("submitted bytes").len(), 1);
    assert_eq!(signer_calls.load(Ordering::SeqCst), 1);
}

fn transaction_launch_material() -> (
    mfm_program::TypedProgramDraft,
    BTreeMap<mfm_ids::SeedId, mfm_canonical::PlainCanonicalJsonBytes>,
) {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<SubmitEvmTransactionState>()
        .expect("register transaction state");
    let action = CanonicalSeed::from_value(
        &EvmTransactionAction::call(DESTINATION, vector_calldata(), U256::ZERO)
            .expect("transaction action"),
    )
    .expect("transaction action seed");
    let action_bytes = action.canonical_json().clone();
    let draft = build_root_with_registries(
        ScopeKey::new("evm-transaction").expect("scope key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let action = root.seed(SeedKey::new("action")?, action.clone())?;
            let outcome = root
                .scope()
                .side_effect::<SubmitEvmTransactionState, _>(
                    StateKey::new("submit")?,
                    NoContext,
                    transaction_config(),
                    action,
                    evm_sender_lane_resource_claim()?,
                    SideEffectVerificationSpec::Finalized { depth: 2 },
                )?
                .into_handle();
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &EvmTransactionPublicOutputs { outcome },
            )
        },
    )
    .expect("transaction draft");
    let seed_material = BTreeMap::from([(draft.seeds()[0].seed_id.clone(), action_bytes)]);
    (draft, seed_material)
}

fn manual_transaction_launch_material() -> (
    mfm_program::TypedProgramDraft,
    BTreeMap<mfm_ids::SeedId, mfm_canonical::PlainCanonicalJsonBytes>,
) {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<SubmitEvmTransactionState>()
        .expect("register transaction state");
    states
        .register::<SequenceTransactionState>()
        .expect("register transaction sequence state");
    let first_action = CanonicalSeed::from_value(
        &EvmTransactionAction::call(DESTINATION, vector_calldata(), U256::ZERO)
            .expect("first transaction action"),
    )
    .expect("first transaction action seed");
    let second_action = CanonicalSeed::from_value(
        &EvmTransactionAction::call(DESTINATION, vector_calldata(), U256::ZERO)
            .expect("second transaction action"),
    )
    .expect("second transaction action seed");
    let first_bytes = first_action.canonical_json().clone();
    let second_bytes = second_action.canonical_json().clone();
    let draft = build_root_with_registries(
        ScopeKey::new("evm-manual-transaction").expect("scope key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(manual_transaction_saga_policy())?;
            let first_action = root.seed(SeedKey::new("first-action")?, first_action.clone())?;
            let second_action = root.seed(SeedKey::new("second-action")?, second_action.clone())?;
            let first = root.scope().side_effect::<SubmitEvmTransactionState, _>(
                StateKey::new("submit-first")?,
                NoContext,
                transaction_config(),
                first_action,
                evm_sender_lane_resource_claim()?,
                SideEffectVerificationSpec::Receipt,
            )?;
            let next = root.scope().state::<SequenceTransactionState, _>(
                StateKey::new("require-first-success")?,
                NoContext,
                SequenceTransactionConfig {},
                SequenceTransactionInputHandles {
                    previous: first.into_handle(),
                    next: second_action,
                },
            )?;
            let second = root.scope().side_effect::<SubmitEvmTransactionState, _>(
                StateKey::new("submit-second")?,
                NoContext,
                transaction_config(),
                next,
                evm_sender_lane_resource_claim()?,
                SideEffectVerificationSpec::Receipt,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &EvmTransactionPublicOutputs {
                    outcome: second.into_handle(),
                },
            )
        },
    )
    .expect("manual transaction draft");
    let seed_material = BTreeMap::from([
        (draft.seeds()[0].seed_id.clone(), first_bytes),
        (draft.seeds()[1].seed_id.clone(), second_bytes),
    ]);
    (draft, seed_material)
}

fn manual_transaction_saga_policy() -> SideEffectSagaPolicy {
    SideEffectSagaPolicy::ManualResolution {
        manual: Box::new(manual_transaction_policy()),
    }
}

fn manual_transaction_policy() -> ManualResolutionPolicyDraft {
    let operator = OperatorAuthorityMemberSpec {
        operator_id: OperatorId::new("operator.app-status").expect("operator id"),
        public_identity: OperatorPublicIdentity::new("0x7e5f4552091a69125d5dfcb7b8c2659029395bdf")
            .expect("operator public identity"),
    };
    let authority = OperatorAuthoritySnapshotDraft::new(
        OperatorAuthorityId::new("mfm.app.test.manual.authority").expect("authority id"),
        NonEmptyUniqueOperators::new(operator, Vec::new()).expect("operator authority"),
    );
    let authorization = ManualAuthorizationDraft::threshold(
        ManualAuthorizationVerifierId::new("mfm.app.test.manual.verifier")
            .expect("manual verifier id"),
        ManualSigningSchemeSpec::new(MANUAL_RESOLUTION_DIGEST_SIGNATURE_SCHEME)
            .expect("manual signing scheme"),
        authority,
        ThresholdQuorum::new(1).expect("manual quorum"),
    )
    .expect("manual authorization");
    let evidence_schema = mfm_ids::SchemaId::new(
        "mfm.app.test.manual_evidence",
        "1",
        mfm_ids::DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.app.test.manual_evidence"),
    )
    .expect("manual evidence schema");
    ManualResolutionPolicyDraft::new(evidence_schema, authorization)
}

fn manual_resolution_content_ref(
    schema_id: mfm_ids::SchemaId,
    bytes: &[u8],
) -> ManualResolutionEvidenceRef {
    let content_hash = mfm_ids::ContentDigest::from_digest(
        mfm_ids::DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(bytes),
    );
    ManualResolutionEvidenceRef {
        schema_id,
        artifact_id: mfm_ids::ArtifactId::from_digest(
            content_hash.algorithm(),
            *content_hash.digest(),
        ),
        content_hash,
    }
}

fn manual_claim_signature(claim: &ManualResolutionAuthorizationClaim) -> Vec<u8> {
    let mut key_bytes = [0_u8; 32];
    key_bytes[31] = 1;
    let signing_key =
        ManualSigningKey::from_slice(&key_bytes).expect("manual authorization signing key");
    let claim_digest = claim.digest().expect("manual claim digest");
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(claim_digest.digest().as_bytes())
        .expect("manual claim signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}

fn manual_authorization_proof_bytes(
    policy: &mfm_spec::v1::ManualResolutionAuthorizationSpec,
    claim: ManualResolutionAuthorizationClaim,
    signature: Vec<u8>,
) -> Vec<u8> {
    let operator = policy
        .authority
        .operators
        .first()
        .expect("manual authorization operator");
    ManualResolutionAuthorizationProof {
        verifier_id: policy.verifier_id.clone(),
        signing_scheme: policy.signing_scheme.clone(),
        claim,
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id.clone(),
            public_identity: operator.public_identity.clone(),
            signature: ManualAuthorizationSignatureBytes::new(signature)
                .expect("manual signature bytes"),
        }],
    }
    .canonical_json()
    .expect("manual authorization proof")
    .as_bytes()
    .to_vec()
}

fn manual_authorization_artifact(
    proof_bytes: &[u8],
) -> (
    ManualResolutionEvidenceRef,
    store::ArtifactEvidenceRef,
    mfm_ids::ContentDigest,
) {
    let content_ref = manual_resolution_content_ref(
        manual_authorization_proof_schema_id().expect("manual authorization schema"),
        proof_bytes,
    );
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: content_ref.artifact_id.clone(),
        digest: content_ref.content_hash.clone(),
        byte_len: proof_bytes.len() as u64,
        media_type: mfm_spec::v1::MediaType::new("application/json")
            .expect("manual authorization media type"),
        schema_id: Some(content_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::ManualResolutionAuthorization,
    };
    let evidence_hash = evidence
        .evidence_hash()
        .expect("manual authorization evidence hash");
    (content_ref, evidence, evidence_hash)
}

#[derive(Clone)]
struct InvalidManualHistoryStore {
    journal: store::test_support::StaticRunJournalBackendForTest,
    projection_source: store::AsyncInMemoryRunStore,
}

impl InvalidManualHistoryStore {
    fn new(
        source: &store::AsyncInMemoryRunStore,
        run_id: &mfm_ids::RunId,
        invalid_proof: Vec<u8>,
    ) -> Result<Self, store::StoreError> {
        let mut records = source.committed_records_for_test(run_id)?;
        let mut objects = source.committed_artifact_byte_authority_for_test(run_id)?;
        let (authorization, authorization_evidence, authorization_evidence_hash) =
            manual_authorization_artifact(&invalid_proof);
        let position = records
            .iter()
            .position(|record| {
                matches!(
                    record.payload(),
                    events::KernelEventPayload::ManualResolutionRecorded(_)
                )
            })
            .expect("manual resolution record");
        let (seq, store_commit_order, ordinal, commit_key, mut payload) = {
            let record = &records[position];
            (
                record.seq().as_u64(),
                record.store_commit_order().as_u64(),
                record.ordinal().as_u32(),
                record.commit_key().clone(),
                record.payload().clone(),
            )
        };
        let events::KernelEventPayload::ManualResolutionRecorded(manual) = &mut payload else {
            unreachable!("manual resolution position was selected");
        };
        manual.authorization_schema_id = authorization.schema_id.clone();
        manual.authorization_hash = authorization.content_hash.clone();
        manual.authorization_artifact_id = authorization.artifact_id.clone();
        manual.authorization_artifact_evidence_hash = authorization_evidence_hash.clone();
        records[position] =
            store::test_support::persisted_kernel_event_envelope_with_ordinal_for_test(
                run_id,
                seq,
                store_commit_order,
                ordinal,
                commit_key,
                payload,
            );
        objects.insert(
            (authorization.artifact_id, authorization_evidence_hash),
            (invalid_proof, authorization_evidence),
        );
        Ok(Self {
            journal: store::test_support::StaticRunJournalBackendForTest::new(
                run_id.clone(),
                records,
                objects,
            ),
            projection_source: source.clone(),
        })
    }
}

impl store::RunJournalBackend for InvalidManualHistoryStore {
    type Error = store::StoreError;

    fn backend_append<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        store::RunJournalBackend::backend_append(&self.journal, bundle)
    }

    fn backend_load<'a>(
        &'a self,
        verifier: store::JournalLoadVerifier,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunJournal, Self::Error> {
        store::RunJournalBackend::backend_load(&self.journal, verifier)
    }
}

impl store::CurrentProjectionStore for InvalidManualHistoryStore {
    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        store::CurrentProjectionStore::status_projection_snapshot(&self.projection_source, run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        store::CurrentProjectionStore::fact_projection_snapshot(&self.projection_source)
    }
}

impl store::StoreScopeStore for InvalidManualHistoryStore {
    type Error = store::StoreError;

    fn load_store_scope_id<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, mfm_ids::StoreScopeId, Self::Error> {
        store::StoreScopeStore::load_store_scope_id(&self.projection_source)
    }
}

fn transaction_config() -> EvmTransactionConfig {
    EvmTransactionConfig::new(
        "ethereum-mainnet",
        1,
        EXPECTED_SENDER,
        mfm_signing::SignerRef::new("deployer").expect("signer reference"),
        Vec::new(),
    )
    .expect("transaction config")
}

fn transaction_certification_registry() -> CertificationRegistry {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<SubmitEvmTransactionState>()
        .expect("certify transaction state");
    registry
}

fn manual_transaction_certification_registry() -> CertificationRegistry {
    let mut registry = transaction_certification_registry();
    registry
        .register_state::<SequenceTransactionState>()
        .expect("certify transaction sequence state");
    let manual = manual_transaction_policy().to_spec();
    registry
        .register_schema_role(
            manual.evidence_schema.clone(),
            mfm_certify::CertifiedSchemaRole::ManualResolutionEvidence,
        )
        .expect("manual evidence schema role");
    registry
        .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
        .expect("manual authorization verifier");
    registry
        .register_operator_authority_snapshot(manual.authorization.authority)
        .expect("manual operator authority");
    registry
}

fn transaction_runners(
    session: Arc<TransactionSession>,
    signer_calls: Arc<AtomicUsize>,
) -> ErasedRunnerRegistry {
    let mut runners = test_runner_registry();
    let pure_factory = test_factory_binding(&runners, "pure");
    register_pure_state::<SequenceTransactionState>(
        &mut RunnerRegistrationBuilder::new(&mut runners),
        &pure_factory,
        None,
    )
    .expect("register transaction sequence runner");
    let side_effect_factory = test_factory_binding(&runners, "apply_side_effect");
    let verify_factory = test_factory_binding(&runners, "read_external");
    let adapter_factory = test_factory_binding(&runners, "evm_jsonrpc_adapter");
    let signer_binder = mfm_signing::DeterministicSigningProviderBinder::new(
        "mfm.test.deterministic-signer",
        move |_signer_ref| {
            let signer = FixedSigner {
                calls: Arc::clone(&signer_calls),
            };
            Box::pin(async move { Ok(Arc::new(signer) as Arc<dyn DeterministicSigningProvider>) })
        },
    )
    .expect("signer binder");
    register_evm_transaction_runner(
        &mut runners,
        signer_binder,
        |binding, signer_ref| {
            Box::pin(async move {
                if binding.network_id().as_str() == "ethereum-mainnet"
                    && binding.expected_chain_id() == 1
                    && signer_ref.as_str() == "deployer"
                {
                    Ok(())
                } else {
                    Err(mfm_runtime::RuntimeError::RunnerBinding(
                        "unexpected transaction binding".to_owned(),
                    ))
                }
            })
        },
        Arc::new(TransactionSessions { session }),
        &side_effect_factory,
        &verify_factory,
        &adapter_factory,
    )
    .expect("register transaction runners");
    runners
}

struct TransactionSession {
    evidence: EvmSessionEvidence,
    receipt_status: EvmReceiptStatus,
    visible: AtomicBool,
    make_visible_on_submit: AtomicBool,
    submit_response_unavailable: AtomicBool,
    observation_outage: AtomicU8,
    submitted: Mutex<Vec<Vec<u8>>>,
}

struct TransactionSessions {
    session: Arc<TransactionSession>,
}

impl EvmTransactionSessionSet for TransactionSessions {
    fn implementation_id(&self) -> &str {
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    }

    fn session<'a>(
        &'a self,
        binding: &'a EvmNetworkBinding,
    ) -> EvmSessionFuture<'a, Arc<dyn EvmTransactionSession>> {
        Box::pin(async move {
            if !self.session.evidence.matches_binding(binding) {
                return Err(provider_failure());
            }
            Ok(Arc::clone(&self.session) as Arc<dyn EvmTransactionSession>)
        })
    }
}

impl TransactionSession {
    fn new() -> Self {
        let binding = EvmNetworkBinding::new(
            mfm_ids::LocalPublicId::new("ethereum-mainnet").expect("network id"),
            1,
        )
        .expect("network binding");
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("primary").expect("source id"),
                mfm_ids::LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            receipt_status: EvmReceiptStatus::Success,
            visible: AtomicBool::new(false),
            make_visible_on_submit: AtomicBool::new(false),
            submit_response_unavailable: AtomicBool::new(false),
            observation_outage: AtomicU8::new(0),
            submitted: Mutex::new(Vec::new()),
        }
    }

    fn fail_once(&self, outage: ObservationOutage) {
        self.observation_outage
            .store(outage as u8, Ordering::SeqCst);
    }

    fn fail_next_submit_response(&self) {
        self.submit_response_unavailable
            .store(true, Ordering::SeqCst);
    }

    fn make_transaction_visible(&self) {
        self.visible.store(true, Ordering::SeqCst);
    }

    fn take_outage(&self, outage: ObservationOutage) -> bool {
        self.observation_outage
            .compare_exchange(outage as u8, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn transaction(&self) -> EvmObservedTransaction {
        EvmObservedTransaction {
            transaction_hash: EXPECTED_HASH,
            chain_id: U256::from(1),
            nonce: U256::from(0x42),
            from: EXPECTED_SENDER,
            to: TxKind::Call(DESTINATION),
            value: U256::ZERO,
            input: vector_calldata().into(),
            gas_limit: U256::from(44_386),
            max_fee_per_gas: U256::from(20_000_000_000_u64),
            max_priority_fee_per_gas: U256::from(1_000_000_000_u64),
            access_list: Default::default(),
            placement: None,
        }
    }

    fn receipt(&self) -> EvmReceipt {
        EvmReceipt {
            transaction_hash: EXPECTED_HASH,
            transaction_index: U256::from(3),
            block: EvmBlockAnchor::new(U256::from(100), RECEIPT_BLOCK_HASH),
            from: EXPECTED_SENDER,
            to: Some(DESTINATION),
            contract_address: None,
            status: self.receipt_status,
            gas_used: U256::from(40_000),
            cumulative_gas_used: U256::from(80_000),
            logs: Vec::new(),
        }
    }
}

impl EvmTransactionSession for TransactionSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn pending_nonce<'a>(&'a self, account: Address) -> EvmSessionFuture<'a, U256> {
        assert_eq!(account, EXPECTED_SENDER);
        Box::pin(async { Ok(U256::from(0x42)) })
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
        assert_eq!(request.from(), EXPECTED_SENDER);
        assert_eq!(request.to(), TxKind::Call(DESTINATION));
        assert_eq!(request.input().as_ref(), vector_calldata());
        Box::pin(async { Ok(U256::from(44_386)) })
    }

    fn submit_raw_transaction<'a>(
        &'a self,
        signed_bytes: &'a [u8],
        expected_hash: B256,
    ) -> EvmSessionFuture<'a, B256> {
        assert_eq!(expected_hash, EXPECTED_HASH);
        assert_eq!(keccak256(signed_bytes), EXPECTED_HASH);
        self.submitted
            .lock()
            .expect("submitted bytes")
            .push(signed_bytes.to_vec());
        if self.make_visible_on_submit.load(Ordering::SeqCst) {
            self.visible.store(true, Ordering::SeqCst);
        }
        if self
            .submit_response_unavailable
            .swap(false, Ordering::SeqCst)
        {
            return Box::pin(async { Err(observation_provider_failure()) });
        }
        Box::pin(async move { Ok(expected_hash) })
    }

    fn transaction_by_hash(
        &self,
        transaction_hash: B256,
    ) -> EvmSessionFuture<'_, Option<EvmObservedTransaction>> {
        assert_eq!(transaction_hash, EXPECTED_HASH);
        let transaction = self
            .visible
            .load(Ordering::SeqCst)
            .then(|| self.transaction());
        Box::pin(async move { Ok(transaction) })
    }

    fn receipt_by_hash(&self, transaction_hash: B256) -> EvmSessionFuture<'_, Option<EvmReceipt>> {
        assert_eq!(transaction_hash, EXPECTED_HASH);
        if self.take_outage(ObservationOutage::Receipt) {
            return Box::pin(async { Err(observation_provider_failure()) });
        }
        let receipt = self.visible.load(Ordering::SeqCst).then(|| self.receipt());
        Box::pin(async move { Ok(receipt) })
    }

    fn read_block<'a>(
        &'a self,
        selector: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmBlockAnchor> {
        let outage = match selector {
            EvmBlockSelector::Number(_) => ObservationOutage::CanonicalBlock,
            EvmBlockSelector::Latest => ObservationOutage::Head,
            EvmBlockSelector::ExactHash(_) => panic!("unexpected exact-hash block selector"),
        };
        if self.take_outage(outage) {
            return Box::pin(async { Err(observation_provider_failure()) });
        }
        let block = match selector {
            EvmBlockSelector::Number(number) if *number == U256::from(100) => {
                EvmBlockAnchor::new(U256::from(100), RECEIPT_BLOCK_HASH)
            }
            EvmBlockSelector::Latest => {
                EvmBlockAnchor::new(U256::from(101), B256::from([0x22; 32]))
            }
            _ => panic!("unexpected block selector"),
        };
        Box::pin(async move { Ok(block) })
    }
}

#[derive(Clone)]
struct FixedSigner {
    calls: Arc<AtomicUsize>,
}

impl SigningProvider for FixedSigner {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let signature = PrimitiveSignature::from_scalars_and_parity(
            b256!("840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca90058565"),
            b256!("25e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1"),
            false,
        );
        let identity = PublicSigningIdentity::new(
            request.algorithm().clone(),
            None,
            Some(format!("{EXPECTED_SENDER:?}")),
        )
        .expect("public signing identity");
        let result = SigningResult::for_request(
            request,
            identity,
            SignatureBytes::new(signature.as_bytes().to_vec()).expect("signature bytes"),
        );
        Box::pin(async move { result })
    }
}

impl DeterministicSigningProvider for FixedSigner {
    fn implementation_id(&self) -> &'static str {
        "mfm.test.deterministic-signer"
    }

    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

fn vector_calldata() -> Vec<u8> {
    hex!("a22cb4650000000000000000000000005eee75727d804a2b13038928d36f8b188945a57a0000000000000000000000000000000000000000000000000000000000000000").to_vec()
}

fn provider_failure() -> mfm_evm::EvmCapabilityError {
    mfm_evm::EvmCapabilityError::provider_failure(mfm_evm::evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationInvalid,
    ))
}

fn observation_provider_failure() -> mfm_evm::EvmCapabilityError {
    mfm_evm::EvmCapabilityError::provider_failure(mfm_evm::evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::TransportFailed,
    ))
}

fn state_input_cells(node: &mfm_program::StateNodeSpec) -> Vec<&str> {
    fn collect<'a>(node: InputBindingNodeRef<'a>, cells: &mut Vec<&'a str>) {
        match node {
            InputBindingNodeRef::Unit => {}
            InputBindingNodeRef::Cell(cell) => cells.push(cell.cell_id().as_str()),
            InputBindingNodeRef::Tuple(elements) => {
                for element in elements {
                    collect(element.as_ref(), cells);
                }
            }
            InputBindingNodeRef::Struct(fields) => {
                for field in fields {
                    collect(field.node.as_ref(), cells);
                }
            }
            InputBindingNodeRef::Vec { elements, .. }
            | InputBindingNodeRef::NonEmptyVec { elements, .. } => {
                for element in elements {
                    collect(element.as_ref(), cells);
                }
            }
        }
    }

    let mut cells = Vec::new();
    collect(node.input.root.as_ref(), &mut cells);
    cells
}
