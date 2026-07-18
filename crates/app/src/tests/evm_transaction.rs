use super::*;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alloy_primitives::{
    address, b256, hex, keccak256, Address, PrimitiveSignature, TxKind, B256, U256,
};
use mfm_adapters_evm::{register_evm_transaction_runner, EvmTransactionRunnerCapabilities};
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::NoCaps;
use mfm_certify::CertificationRegistry;
use mfm_events::v1::KernelEventPayload;
use mfm_evm_capabilities::{
    EvmBlock, EvmBlockSelector, EvmFeeInputs, EvmNetworkBinding, EvmObservedTransaction,
    EvmReceipt, EvmReceiptStatus, EvmSessionEvidence, EvmSessionFuture, EvmTransactionEstimate,
    EvmTransactionSession, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, InputBindingNodeRef, NoContext, PublicOutputKey,
    PureState, RemediationNodeParams, RemediationUnresolved, RootBuilder, ScopeKey, SeedKey,
    SideEffectNodeParams, SideEffectSagaPolicy, SideEffectVerificationSpec, StateKey,
    StateRegistryBuilder, ValidatedConfig,
};
use mfm_program_derive::{PublicOutputs, StateInput};
use mfm_runtime::{CapabilityImplementationId, ErasedRunnerRegistry};
use mfm_signing::{
    DeterministicSigningProvider, PublicSigningIdentity, SignatureBytes, SigningFuture,
    SigningProvider, SigningRequest, SigningResult, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use mfm_states_evm::{
    evm_sender_lane_resource_claim, EvmTransactionAction, EvmTransactionConfig,
    EvmTransactionOutcome, SubmitEvmTransactionState,
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
    type Effect = mfm_effects::Pure;
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
            mfm_states_evm::EvmTransactionResult::Succeeded { .. }
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
async fn certified_transaction_resumes_unknown_submission_and_replays_without_live_authority() {
    let store = store::AsyncInMemoryRunStore::default();
    let session = Arc::new(TransactionSession::new());
    let signer_calls = Arc::new(AtomicUsize::new(0));
    let certification = transaction_certification_registry();
    let launch_services = make_run_services(
        transaction_runners(&store, Arc::clone(&session), Arc::clone(&signer_calls)),
        store.clone(),
        store.clone(),
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
    let runtime_spec = mfm_runtime::CertifiedRuntimeSpec::new(request.certified_spec.clone())
        .expect("transaction runtime spec");
    mfm_runtime::BoundRuntimeContextLoader::new(transaction_runners(
        &store,
        Arc::clone(&session),
        Arc::clone(&signer_calls),
    ))
    .load(&runtime_spec)
    .expect("bind transaction runtime");

    let launch = launch_services
        .launch_run(request)
        .await
        .expect("launch transaction graph");
    let (_, launched, _) = launch.into_response_parts();
    let launched = launched.expect("admitted transaction run");
    assert_eq!(launched.run_mode, RunModeStatus::Forward);
    assert_eq!(launched.scheduler_status, "blocked");
    assert!(store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .iter()
        .any(|event| matches!(
            event.payload(),
            KernelEventPayload::SideEffectSubmissionUnknown(_)
        )));
    assert_eq!(signer_calls.load(Ordering::SeqCst), 2);
    drop(launch_services);
    assert!(store
        .expire_execution_claim_for_test(&run_id)
        .expect("expire abandoned execution claim"));

    // A new runner has an empty signed-envelope cache. Recovery must regenerate the exact
    // deterministic envelope, rebroadcast only those bytes, then finish receipt/finality.
    session.make_visible_on_submit.store(true, Ordering::SeqCst);
    let resume_services = make_run_services(
        transaction_runners(&store, Arc::clone(&session), Arc::clone(&signer_calls)),
        store.clone(),
        store.clone(),
        certification.clone(),
    );
    let resumed = resume_services
        .resume_stored_run(&run_id)
        .await
        .expect("resume unknown transaction");
    assert_eq!(resumed.run_mode, RunModeStatus::Completed);
    assert_eq!(signer_calls.load(Ordering::SeqCst), 3);
    {
        let submissions = session.submitted.lock().expect("submitted bytes");
        assert_eq!(submissions.len(), 3);
        assert!(submissions.windows(2).all(|pair| pair[0] == pair[1]));
    }
    drop(resume_services);

    // Read-only replay has no runner registry, transaction session, or signer binder.
    let replay_services = make_run_read_services(store.clone(), store, certification);
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only transaction replay");
    assert_eq!(replay.run_mode, RunModeStatus::Completed);
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

fn transaction_runners(
    store: &store::AsyncInMemoryRunStore,
    session: Arc<TransactionSession>,
    signer_calls: Arc<AtomicUsize>,
) -> ErasedRunnerRegistry {
    let mut runners = ErasedRunnerRegistry::new();
    let bind_session = Arc::clone(&session);
    register_evm_transaction_runner(
        &mut runners,
        EvmTransactionRunnerCapabilities::new(
            Arc::new(store.clone()),
            CapabilityImplementationId::new("mfm.test.deterministic-signer")
                .expect("signing implementation"),
            |binding, signer_ref| {
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
            },
            move |binding| {
                let session = Arc::clone(&bind_session);
                Box::pin(async move {
                    if !session.evidence.matches_binding(&binding) {
                        return Err(provider_failure());
                    }
                    Ok(session as Arc<dyn EvmTransactionSession>)
                })
            },
            move |_signer_ref| {
                let signer = FixedSigner {
                    calls: Arc::clone(&signer_calls),
                };
                Box::pin(
                    async move { Ok(Arc::new(signer) as Arc<dyn DeterministicSigningProvider>) },
                )
            },
        ),
    )
    .expect("register transaction runners");
    runners
}

struct TransactionSession {
    evidence: EvmSessionEvidence,
    visible: AtomicBool,
    make_visible_on_submit: AtomicBool,
    submitted: Mutex<Vec<Vec<u8>>>,
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
            visible: AtomicBool::new(false),
            make_visible_on_submit: AtomicBool::new(false),
            submitted: Mutex::new(Vec::new()),
        }
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
            block_number: U256::from(100),
            block_hash: RECEIPT_BLOCK_HASH,
            from: EXPECTED_SENDER,
            to: Some(DESTINATION),
            contract_address: None,
            status: EvmReceiptStatus::Success,
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
        let receipt = self.visible.load(Ordering::SeqCst).then(|| self.receipt());
        Box::pin(async move { Ok(receipt) })
    }

    fn read_block<'a>(&'a self, selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock> {
        let block = match selector {
            EvmBlockSelector::Number(number) if *number == U256::from(100) => EvmBlock {
                number: U256::from(100),
                hash: RECEIPT_BLOCK_HASH,
            },
            EvmBlockSelector::Latest => EvmBlock {
                number: U256::from(101),
                hash: B256::from([0x22; 32]),
            },
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
    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

fn vector_calldata() -> Vec<u8> {
    hex!("a22cb4650000000000000000000000005eee75727d804a2b13038928d36f8b188945a57a0000000000000000000000000000000000000000000000000000000000000000").to_vec()
}

fn provider_failure() -> mfm_evm_capabilities::EvmCapabilityError {
    mfm_evm_capabilities::EvmCapabilityError::provider_failure(
        mfm_evm_capabilities::evm_diagnostic(
            mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationInvalid,
        ),
    )
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
