use super::*;

use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

use alloy_primitives::{keccak256, Address, Bytes, PrimitiveSignature, TxKind, B256, U256};
use k256::ecdsa::SigningKey;
use k256::elliptic_curve::rand_core::OsRng;
use mfm_adapters_evm::{
    register_evm_transaction_runner, register_evm_validation_runner,
    EvmTransactionRunnerCapabilities, EvmValidationRunnerCapabilities,
};
use mfm_certify::CertificationRegistry;
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    evm_diagnostic, EvmBlock, EvmBlockSelector, EvmCall, EvmCapabilityError, EvmCode, EvmFeeInputs,
    EvmNetworkBinding, EvmObservedTransaction, EvmReadSession, EvmReceipt, EvmReceiptStatus,
    EvmSessionEvidence, EvmSessionFuture, EvmTransactionEstimate, EvmTransactionSession,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, NoContext, PublicOutputKey, PureState, RootBuilder,
    ScopeKey, SeedKey, SideEffectSagaPolicy, SideEffectVerificationSpec, StateError, StateKey,
    StateRegistryBuilder, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, PublicOutputs, StateInput};
use mfm_runtime::{
    load_materialized_input, load_runner_config_for_node, CapabilityImplementationId,
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    RunnerExecutableIdentityTemplate, RunnerRegistrationBuilder,
};
use mfm_signing::{
    DeterministicSigningProvider, PublicSigningIdentity, SignatureBytes, SigningFuture,
    SigningProvider, SigningRequest, SigningResult, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use mfm_states_evm::{
    evm_sender_lane_resource_claim, EvmContractCallCheck, EvmContractValidationConfig,
    EvmContractValidationTarget, EvmTransactionAction, EvmTransactionConfig, EvmTransactionOutcome,
    EvmTransactionResult, EvmTransactionSuccess, SubmitEvmTransactionState,
    ValidateEvmContractState, VerifiedEvmContract,
};
use mfm_store::v1::{self as store, StoreScopeStore as _};
use serde::de::DeserializeOwned;

const BASE_NONCE: u64 = 7;
const RUNTIME_CODE: &[u8] = &[0x60, 0x00, 0x60, 0x01];
const VALIDATION_CALLDATA: &[u8] = &[0x99];
const VALIDATION_RETURN: &[u8] = &[0x01];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.app.test.evm_contract_workflow_projection_config")]
struct ProjectionConfig {}

struct FirstCallActionState;
struct SecondCallActionState;
struct ValidationTargetState;

fn workflow_state_kind(name: &str) -> mfm_program::Result<mfm_ids::StateKind> {
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
            type Effect = mfm_effects::Pure;
            type Caps = mfm_capabilities::NoCaps;

            fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
                workflow_state_kind($kind)
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
        EvmContractValidationTarget::new(created, input.final_call.receipt().block_anchor())
            .map_err(StateError::from)
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
#[mfm(schema = "mfm.app.test.evm_contract_workflow_public_outputs")]
struct ContractWorkflowPublicOutputs<'program, 'scope> {
    verified: mfm_program::Handle<'program, 'scope, VerifiedEvmContract>,
}

#[tokio::test]
async fn create_call_call_validate_graph_replays_from_evidence_only() {
    let store = store::AsyncInMemoryRunStore::default();
    let signing_key = Arc::new(SigningKey::random(&mut OsRng));
    let sender = signing_key_address(&signing_key);
    let created = sender.create(BASE_NONCE);
    let world = Arc::new(ContractWorkflowWorld::new(sender, created));
    let live_validation_reads = Arc::new(AtomicUsize::new(0));
    let certification = workflow_certification_registry();
    let launch_services = make_run_services(
        workflow_runners(
            &store,
            Arc::clone(&world),
            Arc::clone(&signing_key),
            Arc::clone(&live_validation_reads),
        ),
        store.clone(),
        store.clone(),
        certification.clone(),
    );
    let (draft, seed_material) = workflow_launch_material(sender, created);
    let lowered = mfm_certify::lower_program_draft(&draft).expect("lower workflow draft");
    let scoped = certification
        .scoped_for_spec(lowered.spec())
        .expect("scope workflow certification registry");
    mfm_certify::certify_typed_spec(lowered, &scoped).expect("certify workflow draft");
    let request = crate::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        launch_services.certification_registry(),
        store.load_store_scope_id().await.expect("store scope"),
        None,
    )
    .expect("workflow launch request");
    let run_id = request.run_id.clone();

    let launch = launch_services
        .launch_run(request)
        .await
        .expect("launch create/call/call/validate graph");
    let (_, launched, _) = launch.into_response_parts();
    assert_eq!(
        launched.expect("completed contract workflow").run_mode,
        RunModeStatus::Completed
    );
    assert_eq!(world.included_count(), 3);
    assert_eq!(live_validation_reads.load(Ordering::SeqCst), 3);
    drop(launch_services);
    drop(signing_key);

    // The replay service has no transaction session, read session, signer, or runner registry.
    let replay_services = make_run_read_services(store.clone(), store, certification);
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only contract workflow replay");
    assert_eq!(replay.run_mode, RunModeStatus::Completed);
    assert_eq!(live_validation_reads.load(Ordering::SeqCst), 3);
}

fn workflow_launch_material(
    sender: Address,
    created: Address,
) -> (
    mfm_program::TypedProgramDraft,
    BTreeMap<mfm_ids::SeedId, mfm_canonical::PlainCanonicalJsonBytes>,
) {
    let create_action = CanonicalSeed::from_value(
        &EvmTransactionAction::create([0x60, 0x00], U256::ZERO).expect("create action"),
    )
    .expect("create action seed");
    let create_bytes = create_action.canonical_json().clone();
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
        .register::<ValidateEvmContractState>()
        .expect("register validation state");

    let draft = build_root_with_registries(
        ScopeKey::new("evm-contract-workflow").expect("scope key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let create_action = root.seed(SeedKey::new("create-action")?, create_action.clone())?;
            let created_outcome = root
                .scope()
                .side_effect::<SubmitEvmTransactionState, _>(
                    StateKey::new("create")?,
                    NoContext,
                    workflow_transaction_config(sender),
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
                    workflow_transaction_config(sender),
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
                    workflow_transaction_config(sender),
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
                workflow_validation_config(sender, created),
                target,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &ContractWorkflowPublicOutputs { verified },
            )
        },
    )
    .expect("contract workflow draft");
    let seed_material = BTreeMap::from([(draft.seeds()[0].seed_id.clone(), create_bytes)]);
    (draft, seed_material)
}

fn workflow_transaction_config(sender: Address) -> EvmTransactionConfig {
    EvmTransactionConfig::new(
        "ethereum-mainnet",
        1,
        sender,
        mfm_signing::SignerRef::new("workflow-signer").expect("signer ref"),
        Vec::new(),
    )
    .expect("transaction config")
}

fn workflow_validation_config(sender: Address, created: Address) -> EvmContractValidationConfig {
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

fn workflow_certification_registry() -> CertificationRegistry {
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
        .register_state::<ValidateEvmContractState>()
        .expect("certify validation state");
    registry
}

fn workflow_runners(
    store: &store::AsyncInMemoryRunStore,
    world: Arc<ContractWorkflowWorld>,
    signing_key: Arc<SigningKey>,
    live_validation_reads: Arc<AtomicUsize>,
) -> ErasedRunnerRegistry {
    let mut runners = ErasedRunnerRegistry::new();
    let artifacts: Arc<dyn store::RetainedArtifactReadProvider> = Arc::new(store.clone());
    register_projection_runners(&mut runners, Arc::clone(&artifacts));

    let sender = world.sender;
    let transaction_session = Arc::new(WorkflowTransactionSession::new(Arc::clone(&world)));
    let bind_transaction_session = Arc::clone(&transaction_session);
    let signer = Arc::new(WorkflowSigner {
        signing_key,
        sender,
    });
    register_evm_transaction_runner(
        &mut runners,
        EvmTransactionRunnerCapabilities::new(
            Arc::clone(&artifacts),
            CapabilityImplementationId::new("mfm.test.workflow-signer")
                .expect("signing implementation"),
            move |binding, signer_ref| {
                if binding.network_id().as_str() == "ethereum-mainnet"
                    && binding.expected_chain_id() == 1
                    && signer_ref.as_str() == "workflow-signer"
                {
                    Ok(())
                } else {
                    Err(mfm_runtime::RuntimeError::RunnerBinding(
                        "unexpected workflow transaction binding".to_owned(),
                    ))
                }
            },
            move |binding| {
                let transaction_session = Arc::clone(&bind_transaction_session);
                Box::pin(async move {
                    validate_workflow_binding(&binding)?;
                    Ok(transaction_session as Arc<dyn EvmTransactionSession>)
                })
            },
            move |signer_ref| {
                let signer = Arc::clone(&signer);
                Box::pin(async move {
                    if signer_ref.as_str() != "workflow-signer" {
                        return Err(mfm_signing::SigningError::redacted_provider_failure(
                            "unexpected signer reference",
                        ));
                    }
                    Ok(signer as Arc<dyn DeterministicSigningProvider>)
                })
            },
        ),
    )
    .expect("register transaction runner");

    register_evm_validation_runner(
        &mut runners,
        EvmValidationRunnerCapabilities::new(
            artifacts,
            validate_workflow_binding,
            move |binding| {
                let world = Arc::clone(&world);
                let reads = Arc::clone(&live_validation_reads);
                Box::pin(async move {
                    validate_workflow_binding(&binding)?;
                    Ok(Arc::new(WorkflowReadSession::new(binding, world, reads))
                        as Arc<dyn EvmReadSession>)
                })
            },
        ),
    )
    .expect("register validation runner");
    runners
}

fn register_projection_runners(
    registry: &mut ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
) {
    let identities = RunnerExecutableIdentityTemplate::new(
        "mfm-app",
        "evm-contract-workflow-test",
        env!("CARGO_PKG_VERSION"),
    )
    .expect("projection executable identities");
    let factory = identities
        .factory_binding(events::RunnerFactoryId::new("pure").expect("pure projection factory"));
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    registrations
        .register_state_runner_with_factory::<FirstCallActionState>(
            &factory,
            Arc::new(ProjectionRunner::<FirstCallActionState>::new(Arc::clone(
                &artifacts,
            ))),
        )
        .expect("register first-call projection runner");
    registrations
        .register_state_runner_with_factory::<SecondCallActionState>(
            &factory,
            Arc::new(ProjectionRunner::<SecondCallActionState>::new(Arc::clone(
                &artifacts,
            ))),
        )
        .expect("register second-call projection runner");
    registrations
        .register_state_runner_with_factory::<ValidationTargetState>(
            &factory,
            Arc::new(ProjectionRunner::<ValidationTargetState>::new(artifacts)),
        )
        .expect("register validation-target projection runner");
}

struct ProjectionRunner<S> {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    _state: PhantomData<fn() -> S>,
}

impl<S> ProjectionRunner<S> {
    fn new(artifacts: Arc<dyn store::RetainedArtifactReadProvider>) -> Self {
        Self {
            artifacts,
            _state: PhantomData,
        }
    }
}

impl<S> ErasedNodeRunner for ProjectionRunner<S>
where
    S: PureState,
    S::Config: DeserializeOwned,
    S::Input: DeserializeOwned,
{
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config_for_node::<S::Config>(ctx.node(), self.artifacts.as_ref())
                    .await?;
            let state = S::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input =
                load_materialized_input::<S::Input>(ctx.inputs(), self.artifacts.as_ref()).await?;
            let context = ctx.certified_context::<S::Context>()?;
            let output = state.run(input, &context).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            ErasedRunnerOutput::state_output(&ctx, &output)
        })
    }
}

fn validate_workflow_binding(binding: &EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> {
    if binding.network_id().as_str() == "ethereum-mainnet" && binding.expected_chain_id() == 1 {
        Ok(())
    } else {
        Err(workflow_provider_failure())
    }
}

fn workflow_provider_failure() -> EvmCapabilityError {
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
struct WorkflowChain {
    pending: Option<PendingTransaction>,
    included: BTreeMap<B256, IncludedTransaction>,
}

struct ContractWorkflowWorld {
    sender: Address,
    created: Address,
    chain: Mutex<WorkflowChain>,
}

impl ContractWorkflowWorld {
    fn new(sender: Address, created: Address) -> Self {
        Self {
            sender,
            created,
            chain: Mutex::new(WorkflowChain::default()),
        }
    }

    fn included_count(&self) -> usize {
        self.chain.lock().expect("workflow chain").included.len()
    }

    fn block_by_number(&self, number: U256) -> Option<EvmBlock> {
        self.chain
            .lock()
            .expect("workflow chain")
            .included
            .values()
            .find(|included| included.receipt.block_number == number)
            .map(|included| EvmBlock {
                number,
                hash: included.receipt.block_hash,
            })
    }

    fn has_block_hash(&self, hash: B256) -> bool {
        self.chain
            .lock()
            .expect("workflow chain")
            .included
            .values()
            .any(|included| included.receipt.block_hash == hash)
    }
}

struct WorkflowTransactionSession {
    evidence: EvmSessionEvidence,
    world: Arc<ContractWorkflowWorld>,
}

impl WorkflowTransactionSession {
    fn new(world: Arc<ContractWorkflowWorld>) -> Self {
        let binding = EvmNetworkBinding::new(
            mfm_ids::LocalPublicId::new("ethereum-mainnet").expect("network id"),
            1,
        )
        .expect("network binding");
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("workflow").expect("source ref"),
                mfm_ids::LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            world,
        }
    }
}

impl EvmTransactionSession for WorkflowTransactionSession {
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
        self.world.chain.lock().expect("workflow chain").pending = Some(PendingTransaction {
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
        let mut chain = self.world.chain.lock().expect("workflow chain");
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
            block_number,
            block_hash,
            from: self.world.sender,
            to,
            contract_address,
            status: EvmReceiptStatus::Success,
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
            .expect("workflow chain")
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
            .expect("workflow chain")
            .included
            .get(&transaction_hash)
            .map(|included| included.receipt.clone());
        Box::pin(std::future::ready(Ok(receipt)))
    }

    fn read_block<'a>(&'a self, selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock> {
        let result = match selector {
            EvmBlockSelector::Number(number) => self
                .world
                .block_by_number(*number)
                .ok_or_else(workflow_provider_failure),
            _ => Err(workflow_provider_failure()),
        };
        Box::pin(std::future::ready(result))
    }
}

struct WorkflowReadSession {
    evidence: EvmSessionEvidence,
    world: Arc<ContractWorkflowWorld>,
    reads: Arc<AtomicUsize>,
}

impl WorkflowReadSession {
    fn new(
        binding: EvmNetworkBinding,
        world: Arc<ContractWorkflowWorld>,
        reads: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("workflow").expect("source ref"),
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

impl EvmReadSession for WorkflowReadSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(&'a self, selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock> {
        self.record_read();
        let result = match selector {
            EvmBlockSelector::Number(number) => self
                .world
                .block_by_number(*number)
                .ok_or_else(workflow_provider_failure),
            _ => Err(workflow_provider_failure()),
        };
        Box::pin(std::future::ready(result))
    }

    fn read_balance<'a>(
        &'a self,
        _account: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        Box::pin(std::future::ready(Err(workflow_provider_failure())))
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
            Err(workflow_provider_failure())
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
            Err(workflow_provider_failure())
        };
        Box::pin(std::future::ready(result))
    }
}

struct WorkflowSigner {
    signing_key: Arc<SigningKey>,
    sender: Address,
}

impl SigningProvider for WorkflowSigner {
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

impl DeterministicSigningProvider for WorkflowSigner {
    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

fn signing_key_address(signing_key: &SigningKey) -> Address {
    let encoded = signing_key.verifying_key().to_encoded_point(false);
    let digest = keccak256(&encoded.as_bytes()[1..]);
    Address::from_slice(&digest.as_slice()[12..])
}
