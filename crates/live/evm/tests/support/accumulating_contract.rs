use super::*;
use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallFailureReason, AnchoredContractCallResult,
    EvmBlockAnchor, EvmChainInstance, EvmNonceReservationEffect, EvmTransactionEffect,
    EvmTransactionPreparationEffect, EvmTransactionReceipt, EvmTransactionSettlement, NonceDomain,
    PreparedEvmTransactionEvidence, TransactionReportOutcome,
};
use mfm_journal::{EncodedRunFrame, StoredRunBytes};
use mfm_runtime::EffectAdapterOutcome;
use mfm_store::{AppendResult, MemoryStore, StoreError};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

#[derive(Default)]
pub struct RecordingStore {
    inner: MemoryStore,
    pub frames: Mutex<Vec<Vec<u8>>>,
}
impl Store for RecordingStore {
    fn load_run<'a>(
        &'a self,
        id: &'a RunId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<StoredRunBytes>, StoreError>> + Send + 'a>> {
        self.inner.load_run(id)
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let result = self.inner.append_run(frame).await?;
            if matches!(result, AppendResult::Inserted) {
                self.frames
                    .lock()
                    .unwrap()
                    .push(frame.canonical_bytes().to_vec());
            }
            Ok(result)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Fault {
    None,
    Revert(u64),
    Observation(AnchoredContractCallFailureReason),
}

/// A deterministic external evidence source, recording the exact authorized effect identities.
pub struct ScriptedEvidence {
    pub fault: Fault,
    pub returned: Vec<u8>,
    pub calls: AtomicUsize,
    pub effects: Mutex<Vec<(EffectId, ContentRef)>>,
    pub reservations: Mutex<BTreeMap<EffectId, Reservation>>,
}
impl ScriptedEvidence {
    pub fn new(fault: Fault, returned: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            fault,
            returned,
            calls: AtomicUsize::new(0),
            effects: Mutex::new(vec![]),
            reservations: Mutex::new(BTreeMap::new()),
        })
    }
    pub fn register(
        self: &Arc<Self>,
        builder: &mut RuntimeAssemblyBuilder,
        binding: &EvmTransactionBinding,
    ) {
        builder
            .register_effect_adapter::<EvmNonceReservationEffect, _, _>(binding.clone(), {
                let source = self.clone();
                move |id, reference, command| {
                    source.calls.fetch_add(1, Ordering::SeqCst);
                    source
                        .effects
                        .lock()
                        .unwrap()
                        .push((id.clone(), reference.clone()));
                    let mut reservations = source.reservations.lock().unwrap();
                    let next = u64::try_from(reservations.len()).unwrap();
                    let reservation = reservations
                        .entry(id.clone())
                        .or_insert_with(|| {
                            Reservation::new(
                                id.clone(),
                                reference.clone(),
                                NonceDomain::from_binding(command.binding()),
                                next,
                            )
                            .unwrap()
                        })
                        .clone();
                    Box::pin(async move { Ok(EffectAdapterOutcome::Settled(reservation)) })
                }
            })
            .unwrap();
        builder
            .register_effect_adapter::<EvmTransactionPreparationEffect, _, _>(binding.clone(), {
                let source = self.clone();
                move |id, reference, command| {
                    source.calls.fetch_add(1, Ordering::SeqCst);
                    source
                        .effects
                        .lock()
                        .unwrap()
                        .push((id.clone(), reference.clone()));
                    let evidence = PreparedEvmTransactionEvidence {
                        effect_id: id.clone(),
                        transaction_hash: EvmHash::from_bytes(
                            [u8::try_from(command.reservation().nonce()).unwrap() + 10; 32],
                        ),
                    };
                    Box::pin(async move { Ok(EffectAdapterOutcome::Settled(evidence)) })
                }
            })
            .unwrap();
        builder
            .register_effect_adapter::<EvmTransactionEffect, _, _>(binding.clone(), {
                let source = self.clone();
                move |id, reference, command| {
                    source.calls.fetch_add(1, Ordering::SeqCst);
                    source
                        .effects
                        .lock()
                        .unwrap()
                        .push((id.clone(), reference.clone()));
                    let nonce = command.reserved().reservation().nonce();
                    let receipt = EvmTransactionReceipt {
                        block_anchor: EvmBlockAnchor {
                            number: EvmU256::from_u64(nonce + 100),
                            hash: EvmHash::from_bytes([20; 32]),
                        },
                        transaction_hash: command.transaction_hash().clone(),
                    };
                    let evidence = if matches!(source.fault, Fault::Revert(n) if n == nonce) {
                        EvmTransactionSettlement::reverted(id.clone(), nonce, receipt)
                    } else if command.reserved().command().to().is_none() {
                        EvmTransactionSettlement::created(
                            id.clone(),
                            nonce,
                            receipt,
                            EvmAddress::from_bytes([u8::try_from(nonce).unwrap() + 30; 20]),
                        )
                    } else {
                        EvmTransactionSettlement::called(id.clone(), nonce, receipt)
                    };
                    Box::pin(async move { Ok(EffectAdapterOutcome::Settled(evidence)) })
                }
            })
            .unwrap();
        builder
            .register_adapter::<EvmAnchoredContractCallRead, _, _>(binding.route.clone(), {
                let source = self.clone();
                move |reference, intent| {
                    source.calls.fetch_add(1, Ordering::SeqCst);
                    let evidence = match source.fault {
                        Fault::Observation(AnchoredContractCallFailureReason::Rejected) => {
                            AnchoredContractCallEvidence::rejected(reference.clone())
                        }
                        Fault::Observation(AnchoredContractCallFailureReason::SafeFailure) => {
                            AnchoredContractCallEvidence::safe_failure(reference.clone())
                        }
                        Fault::Observation(AnchoredContractCallFailureReason::IntegrityBlocked) => {
                            AnchoredContractCallEvidence::integrity_blocked(reference.clone())
                        }
                        _ => AnchoredContractCallEvidence::returned(
                            reference.clone(),
                            AnchoredContractCallResult::new(
                                intent.anchor().clone(),
                                source.returned.clone(),
                            )
                            .unwrap(),
                        ),
                    };
                    Box::pin(async move { Ok(evidence) })
                }
            })
            .unwrap();
    }
}

#[tokio::test]
async fn accumulated_fixture_preserves_all_success_failure_and_cold_facts() {
    for (case, fault, returned, reason) in [
        (0, Fault::None, abi_word(42).to_vec(), None),
        (
            1,
            Fault::Revert(0),
            vec![],
            Some(FixtureFailureReason::DeploymentReverted),
        ),
        (
            2,
            Fault::Revert(1),
            vec![],
            Some(FixtureFailureReason::ConfigurationReverted),
        ),
        (
            3,
            Fault::Observation(AnchoredContractCallFailureReason::Rejected),
            vec![],
            Some(FixtureFailureReason::ObservationFailed(
                AnchoredContractCallFailureReason::Rejected,
            )),
        ),
        (
            4,
            Fault::Observation(AnchoredContractCallFailureReason::SafeFailure),
            vec![],
            Some(FixtureFailureReason::ObservationFailed(
                AnchoredContractCallFailureReason::SafeFailure,
            )),
        ),
        (
            5,
            Fault::Observation(AnchoredContractCallFailureReason::IntegrityBlocked),
            vec![],
            Some(FixtureFailureReason::ObservationFailed(
                AnchoredContractCallFailureReason::IntegrityBlocked,
            )),
        ),
        (
            6,
            Fault::None,
            vec![0; 31],
            Some(FixtureFailureReason::InvalidReturnData),
        ),
    ] {
        let binding = EvmTransactionBinding {
            route: EvmTransactionRoute {
                chain_instance: EvmChainInstance {
                    chain_id: nonzero(1),
                    expected_genesis_hash: EvmHash::from_bytes([1; 32]),
                },
                endpoint_ref: EvmEndpoint::new("context-fixture")
                    .unwrap()
                    .endpoint_ref()
                    .unwrap(),
            },
            authority_epoch: EvmAuthorityEpoch::new([2; 32]),
            sender: EvmAddress::from_bytes([3; 20]),
        };
        let input = ContractWorkflow {
            request: FixtureRequest { label: 17 },
            deployment: CheckedCreatePlan::new(
                binding.clone(),
                vec![1, 2, 3],
                EvmU256::from_u64(0),
                nonzero(DEPLOYMENT_GAS),
                (PRIORITY_FEE) as u128,
                (MAX_FEE) as u128,
            )
            .unwrap(),
            configuration: CheckedCallPlan::new(
                binding.clone(),
                fixture_configure_calldata(),
                EvmU256::from_u64(0),
                nonzero(CONFIGURATION_GAS),
                (PRIORITY_FEE) as u128,
                (MAX_FEE) as u128,
            )
            .unwrap(),
            observation: CheckedObservationPlan::new(
                binding.route.clone(),
                VALUE_SELECTOR.to_vec(),
            )
            .unwrap(),
        };
        let source = ScriptedEvidence::new(fault, returned);
        let store = Arc::new(RecordingStore::default());
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        register_fixture_states(&mut builder).unwrap();
        source.register(&mut builder, &binding);
        let runtime = Runtime::new(builder.finish(), store.clone());
        let id = RunId::from_digest(DigestBytes::from_array([case; 32]));
        let program = expand_program(
            EntryPointId::new("mfm.test/accumulated-contract@1").unwrap(),
            &EffectFixtureOperation {
                binding: binding.clone(),
            },
        )
        .unwrap();
        let terminal = runtime
            .start(id.clone(), program, input.clone())
            .await
            .unwrap();
        match (terminal.state(), reason) {
            (RunViewState::Succeeded(value), None) => {
                let report: FixtureReport =
                    serde_json::from_slice(value.canonical_bytes()).unwrap();
                assert_eq!(report.context().request, input.request);
                assert_eq!(report.decoded(), &EvmU256::from_u64(42));
                assert_eq!(
                    report.context().deployment.command(),
                    &input.deployment.command()
                );
                assert_eq!(
                    report.context().configuration.command(),
                    &input.configuration.command_for(
                        report
                            .context()
                            .deployment
                            .outcome()
                            .created_address()
                            .clone()
                    )
                );
                assert_eq!(report.context().deployment.reservation().nonce(), 0);
                assert_eq!(report.context().configuration.reservation().nonce(), 1);
                assert_eq!(
                    report
                        .context()
                        .observation
                        .result()
                        .unwrap()
                        .return_bytes(),
                    abi_word(42)
                );
                let effects = source.effects.lock().unwrap();
                for (offset, facts) in [
                    (0, report.context().deployment.executed()),
                    (3, report.context().configuration.executed()),
                ] {
                    assert_eq!(facts.reservation().effect_id(), &effects[offset].0);
                    assert_eq!(facts.reservation().command_value_ref(), &effects[offset].1);
                    assert_eq!((&facts.preparation().effect_id), &effects[offset + 1].0);
                    assert_eq!(facts.settlement().effect_id(), &effects[offset + 2].0);
                }
                let mut hostile = serde_json::to_value(&report).unwrap();
                hostile["decoded"] = serde_json::json!("0");
                assert!(serde_json::from_value::<FixtureReport>(hostile).is_err());
            }
            (RunViewState::Failed(value), Some(reason)) => {
                let failure: FixtureFailure =
                    serde_json::from_slice(value.canonical_bytes()).unwrap();
                assert_eq!(failure.request(), &input.request);
                assert_eq!(failure.reason(), reason);
                let entries = failure.entries();
                let workflow::FixtureEntryData::Transaction(deployment) = &entries[0].data else {
                    panic!("deployment evidence")
                };
                assert_eq!(deployment.executed().command(), &input.deployment.command());
                assert_eq!(deployment.executed().reservation().nonce(), 0);
                if let workflow::FixtureEntryData::CallPlan(plan) = &entries[1].data {
                    assert_eq!(plan, &input.configuration);
                }
                if let workflow::FixtureEntryData::ObservationPlan(plan) = &entries[2].data {
                    assert_eq!(plan, &input.observation);
                }
                if let workflow::FixtureEntryData::Transaction(configuration) = &entries[1].data {
                    let TransactionReportOutcome::Created(created) = deployment.outcome() else {
                        panic!("created dependency")
                    };
                    assert_eq!(
                        configuration.executed().command(),
                        &input
                            .configuration
                            .command_for(created.created_address().clone())
                    );
                    assert_eq!(configuration.executed().reservation().nonce(), 1);
                }
                let wire = serde_json::to_value(&failure).unwrap();
                let mut wrong_reason = wire.clone();
                wrong_reason["reason"] = serde_json::json!({"kind":"invalid_return_data"});
                if reason != FixtureFailureReason::InvalidReturnData {
                    assert!(serde_json::from_value::<FixtureFailure>(wrong_reason).is_err());
                }
                for hostile in [
                    {
                        let mut v = wire.clone();
                        v["entries"].as_array_mut().unwrap().swap(0, 1);
                        v
                    },
                    {
                        let mut v = wire.clone();
                        v["entries"].as_array_mut().unwrap().pop();
                        v
                    },
                    {
                        let mut v = wire.clone();
                        v["entries"]
                            .as_array_mut()
                            .unwrap()
                            .push(wire["entries"][0].clone());
                        v
                    },
                ] {
                    assert!(serde_json::from_value::<FixtureFailure>(hostile).is_err());
                }
            }
            _ => panic!("unexpected terminal branch"),
        }
        let frames = store.frames.lock().unwrap().clone();
        assert!(frames.iter().all(|frame| frame.len() < 25_231_360));
        assert!(frames.iter().map(Vec::len).sum::<usize>() < 536_870_912);
        let calls = source.calls.load(Ordering::SeqCst);
        let mut cold_builder = RuntimeAssemblyBuilder::new().unwrap();
        register_fixture_states(&mut cold_builder).unwrap();
        source.register(&mut cold_builder, &binding);
        let cold = Runtime::new(cold_builder.finish(), store.clone());
        for view in [
            cold.read(&id).await.unwrap(),
            cold.resume(&id).await.unwrap(),
        ] {
            assert_eq!(view.head_digest(), terminal.head_digest());
            assert_eq!(view.head_sequence(), terminal.head_sequence());
            let bytes = match view.state() {
                RunViewState::Succeeded(v) | RunViewState::Failed(v) => v.canonical_bytes(),
                _ => panic!("terminal"),
            };
            let expected = match terminal.state() {
                RunViewState::Succeeded(v) | RunViewState::Failed(v) => v.canonical_bytes(),
                _ => panic!("terminal"),
            };
            assert_eq!(bytes, expected);
        }
        assert_eq!(source.calls.load(Ordering::SeqCst), calls);
        assert_eq!(*store.frames.lock().unwrap(), frames);
        if case == 2 {
            tokio::task::LocalSet::new()
                .run_until(async move {
                    let progress = tokio::task::spawn_local(async move {
                        let mut unavailable = true;
                        let mut failure_view = Some(terminal);
                        drive_to_success::<FixtureFailure>(async || {
                            if std::mem::take(&mut unavailable) {
                                return Err(RuntimeError::Unavailable);
                            }
                            Ok(failure_view
                                .take()
                                .expect("a terminal failure must not be retried"))
                        })
                        .await
                    });
                    let panic = tokio::time::timeout(Duration::from_secs(2), progress)
                        .await
                        .expect("typed failure must be immediate")
                        .err()
                        .expect("typed failure must panic");
                    let message = panic
                        .into_panic()
                        .downcast::<String>()
                        .expect("typed failure diagnostic");
                    assert!(message.contains("ConfigurationReverted"));
                })
                .await;
        }
    }
}
