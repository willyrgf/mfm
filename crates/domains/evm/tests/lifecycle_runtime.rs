//! Actual shared lifecycle States and native EVM implementations on Runtime, with scripted
//! custody/provider responses. This proves dataflow and persistence, not signed-wire or chain IO.
use mfm_capabilities::{
    AdapterError, EffectAdapter, EffectAdapterOutcome, EffectCapabilityContract,
    EffectImplementation, ReadAdapter, ReadImplementation,
};
use mfm_chain::transaction::*;
use mfm_chain::{ContractArtifact, LedgerIdentity};
use mfm_evm::*;
use mfm_ids::{ContentRef, DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_program::{
    compile, load, BindEffect, BindRead, CapabilityFamily, Classification, ClassifyError, Effect,
    Operation, ProgramEnvironment, ProgramLimits, ProposedStateOutcome, Pure, PureState, Resolve,
    State,
};
use mfm_runtime::Runtime;
use mfm_store::MemoryStore;
use mfm_values::{InvocationDiagnostic, Object};
use std::{
    future::Future,
    num::NonZeroU64,
    pin::Pin,
    sync::{Arc, Mutex},
};

type Composed = Operation<
    (
        Effect<Deploy, TransactionEffect<DeploymentRequest>>,
        Pure<CheckedAddConfigurationValue>,
        ConfigureAndObserve,
        Pure<Validate>,
        Pure<Report>,
    ),
    LifecycleDefaults,
>;
type Extended = Operation<
    (
        Effect<Deploy, TransactionEffect<DeploymentRequest>>,
        Pure<CheckedAddConfigurationValue>,
        Pure<RequireNonZero>,
        ConfigureAndObserve,
        Pure<Validate>,
        Pure<Report>,
    ),
    LifecycleDefaults,
>;
#[derive(Debug, serde::Serialize, serde::Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
struct ZeroConfiguration {}
impl ClassifyError for ZeroConfiguration {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}
struct RequireNonZero;
impl State for RequireNonZero {
    type Input = DeployedContract;
    type Output = DeployedContract;
    type Failure = ZeroConfiguration;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("example.require-nonzero-configuration@1")?)
    }
}
impl PureState for RequireNonZero {
    fn evaluate(
        input: DeployedContract,
    ) -> Result<ProposedStateOutcome<DeployedContract, ZeroConfiguration>, InvocationDiagnostic>
    {
        Ok(if input.effective().is_zero() {
            ProposedStateOutcome::Failure {
                failure: ZeroConfiguration {},
            }
        } else {
            ProposedStateOutcome::Success { output: input }
        })
    }
}
#[derive(Default)]
struct Script {
    nonce: u64,
    getter: Option<Vec<u8>>,
    calls: usize,
}
// Count Store calls, including rejected attempts, independently of retained frame inspection.
struct CountingStore(MemoryStore, std::sync::atomic::AtomicUsize);
impl mfm_store::Store for CountingStore {
    fn load_run<'a>(
        &'a self,
        run: &'a RunId,
        probe: Option<u64>,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<mfm_store::LoadedRun>, mfm_store::StoreError>>
                + Send
                + 'a,
        >,
    > {
        mfm_store::Store::load_run(&self.0, run, probe)
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<mfm_store::AppendResult, mfm_store::StoreError>> + Send + 'a,
        >,
    > {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        mfm_store::Store::append_run(&self.0, frame)
    }
}
struct Resources<const COLD: bool>(Arc<Mutex<Script>>);
impl<const COLD: bool> ProgramEnvironment for Resources<COLD> {
    type Sources = (
        ContractDeploymentLifecycle,
        Composed,
        Extended,
        Pure<mfm_chain::CheckedAdd>,
    );
}
impl<R: EvmTransactionRecipe, const COLD: bool> CapabilityFamily<TransactionEffect<R>>
    for Resources<COLD>
{
    type Implementations = (EvmTransactionImplementation,);
}
impl<const COLD: bool> CapabilityFamily<EvmNonceReservationEffect> for Resources<COLD> {
    type Implementations = (EvmNonceReservationImplementation,);
}
impl<const COLD: bool> CapabilityFamily<EvmTransactionPreparationEffect> for Resources<COLD> {
    type Implementations = (EvmTransactionPreparationImplementation,);
}
impl<const COLD: bool> CapabilityFamily<ContractRead> for Resources<COLD> {
    type Implementations = (EvmContractReadImplementation,);
}
impl<C: LifecyclePlanning + ?Sized, R: EvmTransactionRecipe> Resolve<C, TransactionEffect<R>>
    for Resources<false>
{
    fn implementation(config: &C) -> mfm_program::Result<StableId> {
        Ok(config
            .deployment_request()
            .execution()
            .transaction_implementation()
            .clone())
    }
}
impl<C: LifecyclePlanning + ?Sized> Resolve<C, ContractRead> for Resources<false> {
    fn implementation(config: &C) -> mfm_program::Result<StableId> {
        Ok(config
            .deployment_request()
            .execution()
            .read_implementation()
            .clone())
    }
}
impl<C, I, const COLD: bool> BindEffect<C, I> for Resources<COLD>
where
    C: EffectCapabilityContract,
    I: EffectImplementation<C, Binding = EvmTransactionBinding>,
    Self: EffectAdapter<I::NativeCommand, I::NativeEvidence, I::OperationalError>,
{
    type Adapter = Self;
    fn bind_effect(&self, _: &EvmTransactionBinding) -> Result<Self, InvocationDiagnostic> {
        Ok(Self(self.0.clone()))
    }
}
impl<const COLD: bool> BindRead<ContractRead, EvmContractReadImplementation> for Resources<COLD> {
    type Adapter = Self;
    fn bind_read(&self, _: &EvmTransactionRoute) -> Result<Self, InvocationDiagnostic> {
        Ok(Self(self.0.clone()))
    }
}
type EffectFuture<'a, E> = Pin<
    Box<
        dyn Future<
                Output = Result<
                    EffectAdapterOutcome<E>,
                    AdapterError<EvmTransactionOperationalError>,
                >,
            > + Send
            + 'a,
    >,
>;
impl<const COLD: bool>
    EffectAdapter<Eip1559TransactionCommand, Reservation, EvmTransactionOperationalError>
    for Resources<COLD>
{
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        command_ref: &'a ContentRef,
        command: &'a Eip1559TransactionCommand,
    ) -> EffectFuture<'a, Reservation> {
        Box::pin(async move {
            let mut script = self.0.lock().unwrap();
            let nonce = script.nonce;
            script.nonce += 1;
            script.calls += 1;
            let binding = command.binding();
            Ok(EffectAdapterOutcome::Settled(
                Reservation::new(
                    effect.clone(),
                    command_ref.clone(),
                    NonceDomain {
                        authority_epoch: binding.authority_epoch.clone(),
                        chain_instance: binding.route.chain_instance.clone(),
                        sender: binding.sender.clone(),
                    },
                    nonce,
                )
                .unwrap(),
            ))
        })
    }
}
impl<const COLD: bool>
    EffectAdapter<
        ReservedEvmTransaction,
        PreparedEvmTransactionEvidence,
        EvmTransactionOperationalError,
    > for Resources<COLD>
{
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        command: &'a ReservedEvmTransaction,
    ) -> EffectFuture<'a, PreparedEvmTransactionEvidence> {
        Box::pin(async move {
            self.0.lock().unwrap().calls += 1;
            Ok(EffectAdapterOutcome::Settled(
                PreparedEvmTransactionEvidence {
                    effect_id: effect.clone(),
                    transaction_hash: EvmHash::from_bytes(
                        [command.reservation().nonce() as u8 + 1; 32],
                    ),
                },
            ))
        })
    }
}
impl<const COLD: bool>
    EffectAdapter<PreparedEvmTransaction, EvmTransactionSettlement, EvmTransactionOperationalError>
    for Resources<COLD>
{
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        command: &'a PreparedEvmTransaction,
    ) -> EffectFuture<'a, EvmTransactionSettlement> {
        Box::pin(async move {
            let mut script = self.0.lock().unwrap();
            script.calls += 1;
            let receipt = EvmTransactionReceipt {
                block_anchor: EvmBlockAnchor {
                    number: EvmU256::from_u64(7),
                    hash: EvmHash::from_bytes([7; 32]),
                },
                transaction_hash: command.transaction_hash().clone(),
            };
            let nonce = command.reserved().reservation().nonce();
            let evidence = if command.reserved().command().to().is_none() {
                EvmTransactionSettlement::created(
                    effect.clone(),
                    nonce,
                    receipt,
                    EvmAddress::from_bytes([4; 20]),
                )
            } else {
                let calldata = command.reserved().command().input();
                assert_eq!(&calldata[..4], &[0x1e, 0xb2, 0x5e, 0x0a]);
                script.getter = Some(calldata[4..].to_vec());
                EvmTransactionSettlement::called(effect.clone(), nonce, receipt)
            };
            Ok(EffectAdapterOutcome::Settled(evidence))
        })
    }
}
impl<const COLD: bool>
    ReadAdapter<AnchoredContractCallIntent, AnchoredContractCallEvidence, EvmOperationalError>
    for Resources<COLD>
{
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        intent_ref: &'a ContentRef,
        intent: &'a AnchoredContractCallIntent,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        AnchoredContractCallEvidence,
                        AdapterError<EvmOperationalError>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let mut script = self.0.lock().unwrap();
            script.calls += 1;
            assert_eq!(intent.target(), &EvmAddress::from_bytes([4; 20]));
            assert_eq!(intent.calldata(), [0x3f, 0xa4, 0xf2, 0x45]);
            let word = script
                .getter
                .clone()
                .expect("getter must follow actual configuration command");
            Ok(AnchoredContractCallEvidence::returned(
                intent_ref.clone(),
                AnchoredContractCallResult::new(intent.anchor().clone(), word).unwrap(),
            ))
        })
    }
}

#[tokio::test]
#[ignore = "requires pinned solc fixture output in MFM_EFFECT_E2E_INITCODE_PATH"]
async fn lifecycle_callers_execute_predecessor_data_and_new_state_failure_survives_cold_inspection()
{
    let text =
        std::fs::read_to_string(std::env::var_os("MFM_EFFECT_E2E_INITCODE_PATH").unwrap()).unwrap();
    let text = text.trim();
    let bytes = (0..text.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&text[offset..offset + 2], 16).unwrap())
        .collect();
    let artifact = Object::from_value(&EvmScalarContractArtifact::new(bytes).unwrap()).unwrap();
    let chain = EvmChainInstance {
        chain_id: NonZeroU64::new(1).unwrap(),
        expected_genesis_hash: EvmHash::from_bytes([1; 32]),
    };
    let binding = EvmTransactionBinding {
        authority_epoch: EvmAuthorityEpoch::new([2; 32]),
        route: EvmTransactionRoute {
            chain_instance: chain.clone(),
            endpoint_ref: Object::from_value(&EvmU256::from_u64(1))
                .unwrap()
                .value_ref()
                .clone(),
        },
        sender: EvmAddress::from_bytes([3; 20]),
    };
    let native_config = EvmContractExecutionConfig::new(
        binding,
        Eip1559Options::new(NonZeroU64::new(1_000_000).unwrap(), 1, 2).unwrap(),
        Eip1559Options::new(NonZeroU64::new(100_000).unwrap(), 1, 2).unwrap(),
    );
    let request = DeploymentRequest::new(
        ContractArtifact::new(
            LedgerIdentity::new(Object::from_value(&chain).unwrap()),
            artifact,
        ),
        ContractExecutionConfig::new(
            <EvmTransactionImplementation as EffectImplementation<
                TransactionEffect<DeploymentRequest>,
            >>::implementation_id()
            .unwrap(),
            EvmContractReadImplementation::implementation_id().unwrap(),
            Object::from_value(&native_config.binding().route)
                .unwrap()
                .value_ref()
                .clone(),
            Object::from_value(&native_config).unwrap(),
        ),
        ConfigurationValue::new("42").unwrap(),
        ConfigurationValue::new("42").unwrap(),
        None,
        None,
    );
    drop(native_config);
    let zero = DeploymentRequest::new(
        request.artifact().clone(),
        request.execution().clone(),
        ConfigurationValue::new("0").unwrap(),
        ConfigurationValue::new("0").unwrap(),
        None,
        None,
    );
    // Construction qualifies the caller's expectation, rather than replacing it with its binding.
    let mut wrong_request = serde_json::to_value(&request).unwrap();
    wrong_request["execution"]["observation_route_ref"] = serde_json::to_value(
        Object::from_value(&EvmU256::from_u64(999))
            .unwrap()
            .value_ref(),
    )
    .unwrap();
    let wrong_request: DeploymentRequest = serde_json::from_value(wrong_request).unwrap();
    let unused = Arc::new(Mutex::new(Script::default()));
    let error = compile(
        EntryPointId::new("mfm.test/wrong-observation-route@1").unwrap(),
        &ContractDeploymentLifecycle::default(),
        &wrong_request,
        &Resources::<false>(unused.clone()),
        ProgramLimits::new(0),
    )
    .err()
    .expect("construction rejects unrelated expected route");
    let mfm_program::ProgramError::Diagnostic(cause) = error else {
        panic!("retain route diagnostic")
    };
    assert_eq!(cause.operation(), "qualify_scalar_route");
    assert_eq!(unused.lock().unwrap().calls, 0);

    for (case, expected) in [(0, "42"), (1, "84"), (2, "42"), (3, "84"), (4, "0")] {
        let request = if case == 4 { &zero } else { &request };
        let script = Arc::new(Mutex::new(Script::default()));
        let resources = Resources::<false>(script.clone());
        let entry = EntryPointId::new("mfm.test/lifecycle@1").unwrap();
        let program = match case {
            0 => compile(
                entry,
                &ContractDeploymentLifecycle::default(),
                request,
                &resources,
                ProgramLimits::new(0),
            ),
            1 => compile(
                entry,
                &Composed::default(),
                request,
                &resources,
                ProgramLimits::new(0),
            ),
            2 => compile(
                entry,
                &Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default(),
                request,
                &resources,
                ProgramLimits::new(0),
            ),
            _ => compile(
                entry,
                &Extended::default(),
                request,
                &resources,
                ProgramLimits::new(0),
            ),
        }
        .unwrap();
        let document = program.canonical_bytes().to_vec();
        drop(program);
        drop(resources);
        let program = load(&document, &Resources::<true>(script.clone())).unwrap();
        let runtime = Runtime::new(Arc::new(MemoryStore::new()));
        let run = RunId::from_digest(DigestBytes::from_array([100 + case; 32]));
        let result = runtime
            .execute(run.clone(), &program, request)
            .await
            .unwrap();
        if case == 2 {
            let deployed = result
                .success()
                .unwrap()
                .decode::<DeployedContract>()
                .unwrap();
            assert_eq!(deployed.effective().to_string(), "42");
            assert_eq!(
                deployed
                    .contract()
                    .unwrap()
                    .native()
                    .decode::<EvmAddress>()
                    .unwrap(),
                EvmAddress::from_bytes([4; 20])
            );
            assert!(script.lock().unwrap().getter.is_none());
            // The maintained child also compiles alone from the actual deployed predecessor.
            let child = compile(
                EntryPointId::new("mfm.test/configure-child@1").unwrap(),
                &ConfigureAndObserve::default(),
                &deployed,
                &Resources::<false>(script.clone()),
                ProgramLimits::new(0),
            )
            .unwrap();
            let child_document = child.canonical_bytes().to_vec();
            drop(child);
            let child = load(&child_document, &Resources::<true>(script.clone())).unwrap();
            let child_run = RunId::from_digest(DigestBytes::from_array([111; 32]));
            let configured = runtime
                .execute(child_run.clone(), &child, &deployed)
                .await
                .unwrap();
            let observed = configured
                .success()
                .unwrap()
                .decode::<ObservedConfiguration>()
                .unwrap();
            let ContractValueOutcome::Observed { value, .. } = observed.observation().outcome()
            else {
                panic!("standalone child observation")
            };
            assert_eq!(value.to_string(), "42");
            let calls = script.lock().unwrap().calls;
            let cold_child = runtime.read(&child_run, &child).await.unwrap();
            assert_eq!(cold_child.success(), configured.success());
            assert_eq!(script.lock().unwrap().calls, calls);

            // Use the actual configured predecessor to isolate the designated Read boundary.
            let input = observed.configured().clone();
            let read = compile(
                EntryPointId::new("mfm.test/route-observe@1").unwrap(),
                &mfm_program::Read::<Observe, ContractRead>::default(),
                &input,
                &Resources::<false>(script.clone()),
                ProgramLimits::new(0),
            )
            .unwrap();
            let expected = input
                .configured()
                .request()
                .execution()
                .observation_route_ref();
            let intent =
                <Observe as mfm_program::ReadState<ContractRead>>::prepare(&input).unwrap();
            assert_eq!(intent.route_ref(), expected);
            let accepted = runtime
                .execute(
                    RunId::from_digest(DigestBytes::from_array([112; 32])),
                    &read,
                    &input,
                )
                .await
                .unwrap();
            assert_eq!(
                accepted
                    .success()
                    .unwrap()
                    .decode::<ObservedConfiguration>()
                    .unwrap(),
                observed
            );
            let mut route = read.bindings()[0].decode::<EvmTransactionRoute>().unwrap();
            route.endpoint_ref = Object::from_value(&EvmU256::from_u64(2))
                .unwrap()
                .value_ref()
                .clone();
            let alternate = Object::from_value(&route).unwrap();
            let mut document: serde_json::Value =
                serde_json::from_slice(read.canonical_bytes()).unwrap();
            document["declarations"][0]["execution"]["binding_ref"] =
                serde_json::to_value(alternate.value_ref()).unwrap();
            document["bindings"] = serde_json::to_value([alternate]).unwrap();
            let canonical =
                mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&document.to_string())
                    .unwrap();
            drop(read);
            let mixed = load(canonical.as_bytes(), &Resources::<true>(script.clone())).unwrap();
            let store = Arc::new(CountingStore(
                MemoryStore::new(),
                std::sync::atomic::AtomicUsize::new(0),
            ));
            let isolated = Runtime::new(store.clone());
            let mixed_run = RunId::from_digest(DigestBytes::from_array([113; 32]));
            let calls = script.lock().unwrap().calls;
            let Err(mfm_runtime::InvocationFailure::Execution {
                error: mfm_runtime::RuntimeError::Native { cause, .. },
                last_observed: Some(admitted),
                ..
            }) = isolated.execute(mixed_run.clone(), &mixed, &input).await
            else {
                panic!("cold mixed route must fail locally")
            };
            assert_eq!(cause.operation(), "qualify_scalar_route");
            assert_eq!(script.lock().unwrap().calls, calls);
            // Admission is the only frame: rejection appends no observation or integrity block.
            assert_eq!(admitted.head_sequence(), 1);
            assert_eq!(store.1.load(std::sync::atomic::Ordering::SeqCst), 1);
            let retained = isolated.program_document(&mixed_run).await.unwrap();
            drop(mixed);
            let cold = load(
                retained.canonical_bytes(),
                &Resources::<true>(script.clone()),
            )
            .unwrap();
            let Err(mfm_runtime::InvocationFailure::Execution {
                error: mfm_runtime::RuntimeError::Native { cause, .. },
                last_observed: Some(unchanged),
                ..
            }) = isolated.resume(&mixed_run, &cold).await
            else {
                panic!("cold resume must preserve route qualification")
            };
            assert_eq!(cause.operation(), "qualify_scalar_route");
            assert_eq!(unchanged.head_digest(), admitted.head_digest());
            let checked = isolated.read(&mixed_run, &cold).await.unwrap();
            assert_eq!(checked.head_digest(), admitted.head_digest());
            assert!(checked.failure().is_none());
            assert_eq!(store.1.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(script.lock().unwrap().calls, calls);
        } else if case == 4 {
            result
                .failure()
                .unwrap()
                .failure()
                .original()
                .decode::<ZeroConfiguration>()
                .unwrap();
            assert!(script.lock().unwrap().getter.is_none());
            assert_eq!(script.lock().unwrap().nonce, 1);
        } else {
            let report = result
                .success()
                .expect("terminal success")
                .decode::<ContractDeploymentReport>()
                .unwrap();
            assert_eq!(report.requested_value().to_string(), "42");
            assert_eq!(report.effective_value().to_string(), expected);
            assert_eq!(report.observed_value().to_string(), expected);
            report
                .deployment()
                .original()
                .decode::<EvmTransactionSettlement>()
                .unwrap();
            report
                .configuration()
                .original()
                .decode::<EvmTransactionSettlement>()
                .unwrap();
            report
                .observation()
                .original()
                .decode::<AnchoredContractCallEvidence>()
                .unwrap();
        }
        let calls = script.lock().unwrap().calls;
        let document = runtime.program_document(&run).await.unwrap();
        drop(program);
        let program = load(
            document.canonical_bytes(),
            &Resources::<true>(script.clone()),
        )
        .unwrap();
        let cold = runtime.read(&run, &program).await.unwrap();
        let resumed = runtime.resume(&run, &program).await.unwrap();
        assert_eq!(cold.success(), result.success());
        assert_eq!(resumed.success(), result.success());
        assert_eq!(
            cold.failure().map(|report| report.canonical_bytes()),
            result.failure().map(|report| report.canonical_bytes())
        );
        assert_eq!(
            resumed.failure().map(|report| report.canonical_bytes()),
            result.failure().map(|report| report.canonical_bytes())
        );
        assert_eq!(script.lock().unwrap().calls, calls);
    }
}

#[tokio::test]
async fn existing_pure_caller_executes_without_binding_any_native_adapter() {
    let script = Arc::new(Mutex::new(Script::default()));
    let input = mfm_chain::CheckedAddition::new("42", "42").unwrap();
    let program = compile(
        EntryPointId::new("mfm.test/pure-caller@1").unwrap(),
        &Pure::<mfm_chain::CheckedAdd>::default(),
        &input,
        &Resources::<false>(script.clone()),
        ProgramLimits::new(0),
    )
    .unwrap();
    assert!(program.bindings().is_empty());
    let program = load(
        program.canonical_bytes(),
        &Resources::<true>(script.clone()),
    )
    .unwrap();
    let result = Runtime::new(Arc::new(MemoryStore::new()))
        .execute(
            RunId::from_digest(DigestBytes::from_array([110; 32])),
            &program,
            &input,
        )
        .await
        .unwrap();
    assert_eq!(
        result
            .success()
            .unwrap()
            .decode::<mfm_values::Unsigned256>()
            .unwrap()
            .to_string(),
        "84"
    );
    assert_eq!(script.lock().unwrap().calls, 0);
}
