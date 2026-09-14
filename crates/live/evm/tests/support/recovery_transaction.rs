use mfm_capabilities::AdapterError;
use mfm_evm::*;
use mfm_ids::{DigestBytes, EntryPointId, RunId};
use mfm_program::ProgramLimits;
use mfm_program_derive::{MfmContext, MfmValue};
use mfm_runtime::{
    EffectAdapterOutcome, InvocationFailure, RunViewState, Runtime, RuntimeAssemblyBuilder,
};
use mfm_store::Store;
use serde::{Deserialize, Serialize};
use std::{
    num::NonZeroU64,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
struct Caller {
    #[mfm(minimum_bytes = 1, maximum_bytes = 65536)]
    accumulated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.recovery-transaction")]
#[serde(deny_unknown_fields)]
struct Context<T> {
    caller: Caller,
    transaction: T,
}
type Input = Context<CheckedCreatePlan>;
type Recipe = CreateAt<ContextTransactionSlot>;

#[tokio::test]
async fn maximum_transaction_closures_fit_and_pending_operational_failure_preserves_authority() {
    let maximum = EvmU256::new(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .unwrap();
    let binding = EvmTransactionBinding {
        route: EvmTransactionRoute {
            chain_instance: EvmChainInstance {
                chain_id: NonZeroU64::new(u64::MAX).unwrap(),
                expected_genesis_hash: EvmHash::from_bytes([255; 32]),
            },
            endpoint_ref: EvmEndpoint::new("\"".repeat(256))
                .unwrap()
                .endpoint_ref()
                .unwrap(),
        },
        authority_epoch: EvmAuthorityEpoch::new([255; 32]),
        sender: EvmAddress::from_bytes([255; 20]),
    };
    let input = Context {
        caller: Caller {
            accumulated: "\"".repeat(65536),
        },
        transaction: CheckedCreatePlan::new(
            binding.clone(),
            vec![255; MAX_EVM_INITCODE_BYTES],
            maximum.clone(),
            NonZeroU64::new(u64::MAX).unwrap(),
            u128::MAX,
            u128::MAX,
        )
        .unwrap(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/recovery-transaction@1").unwrap(),
        &EvmTransaction::<Input, Recipe>::new(binding.clone()),
        &input,
        ProgramLimits::new(1),
    )
    .unwrap();
    for scenario in 0..3 {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        mfm_evm_live::register_evm_transaction_states::<Input, Recipe>(&mut builder).unwrap();
        builder
            .register_effect_adapter::<EvmNonceReservationEffect, _, _>(
                binding.clone(),
                |id, reference, command| {
                    Box::pin(async move {
                        Ok(EffectAdapterOutcome::Settled(
                            Reservation::new(
                                id.clone(),
                                reference.clone(),
                                NonceDomain::from_binding(command.binding()),
                                u64::MAX - 1,
                            )
                            .unwrap(),
                        ))
                    })
                },
            )
            .unwrap();
        builder
            .register_effect_adapter::<EvmTransactionPreparationEffect, _, _>(
                binding.clone(),
                |id, _, _| {
                    Box::pin(async move {
                        Ok(EffectAdapterOutcome::Settled(
                            PreparedEvmTransactionEvidence {
                                effect_id: id.clone(),
                                transaction_hash: EvmHash::from_bytes([255; 32]),
                            },
                        ))
                    })
                },
            )
            .unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed_attempts = Arc::clone(&attempts);
        let number = maximum.clone();
        builder
            .register_effect_adapter::<EvmTransactionEffect, _, _>(
                binding.clone(),
                move |id, _, command| {
                    let attempt = observed_attempts.fetch_add(1, Ordering::SeqCst);
                    let number = number.clone();
                    Box::pin(async move {
                        if scenario == 2 && attempt == 0 {
                            return Err(AdapterError::Operational(
                                EvmTransactionOperationalError::Provider {
 operation: mfm_evm::TransactionProviderOperation::Submit,
                                    cause: EvmOperationalError::new(mfm_evm::EvmOperationalKind::Timeout, mfm_evm::ProviderFailure {
        method: mfm_evm::EvmRpcMethod::SendRawTransaction, stage: mfm_evm::RpcStage::Send,
        failure: mfm_evm::ProviderFailureKind::Client,
        diagnostics: serde_json::from_str(r#"{"response":null,"sources":{"layers":[],"end":"unavailable"},"omissions":[],"omissions_truncated":false}"#).unwrap(),
    }),
                                },
                            ));
                        }
                        let receipt = EvmTransactionReceipt {
                            block_anchor: EvmBlockAnchor {
                                number,
                                hash: EvmHash::from_bytes([255; 32]),
                            },
                            transaction_hash: command.transaction_hash().clone(),
                        };
                        let nonce = command.reserved().reservation().nonce();
                        let evidence = if scenario == 1 {
                            EvmTransactionSettlement::reverted(id.clone(), nonce, receipt)
                        } else {
                            EvmTransactionSettlement::created(
                                id.clone(),
                                nonce,
                                receipt,
                                EvmAddress::from_bytes([255; 20]),
                            )
                        };
                        Ok(EffectAdapterOutcome::Settled(evidence))
                    })
                },
            )
            .unwrap();
        let store = Arc::new(mfm_store::MemoryStore::new());
        let runtime = Runtime::new(builder.finish(), store.clone());
        let run = RunId::from_digest(DigestBytes::from_array([scenario + 1; 32]));
        let hot = match runtime
            .start(run.clone(), program.clone(), input.clone())
            .await
        {
            Ok(view) if scenario != 2 => view,
            Err(InvocationFailure::RecoveryStopped { observed, .. }) if scenario == 2 => {
                assert_eq!(observed.head_sequence(), 10);
                let RunViewState::EffectPending {
                    effect,
                    latest_failure: Some(_),
                } = observed.state()
                else {
                    panic!("retained Effect invocation facts")
                };
                effect
                    .call()
                    .input()
                    .decode::<PreparedContext<Input, Recipe>>()
                    .unwrap();
                let command = effect.command().decode::<PreparedEvmTransaction>().unwrap();
                assert_eq!(command.reserved().reservation().nonce(), u64::MAX - 1);
                let cold = runtime.read(&run).await.unwrap();
                assert_eq!(cold.head_digest(), observed.head_digest());
                let (
                    RunViewState::EffectPending {
                        effect: hot_effect, ..
                    },
                    RunViewState::EffectPending {
                        effect: cold_effect,
                        ..
                    },
                ) = (observed.state(), cold.state())
                else {
                    panic!("unchanged pending execution authority")
                };
                assert_eq!(hot_effect.call().position(), cold_effect.call().position());
                assert_eq!(hot_effect.effect_id(), cold_effect.effect_id());
                assert_eq!(attempts.load(Ordering::SeqCst), 1);
                runtime.resume(&run).await.unwrap()
            }
            other => panic!("unexpected transaction result: {}", other.is_ok()),
        };
        assert_eq!(
            hot.head_sequence(),
            if scenario == 2 {
                13
            } else if scenario == 1 {
                12
            } else {
                11
            }
        );
        assert!(matches!(
            (scenario, hot.state()),
            (1, RunViewState::Failed(_)) | (0 | 2, RunViewState::Succeeded(_))
        ));
        let loaded = store.load_run(&run, None).await.unwrap().unwrap();
        assert_eq!(loaded.head().head_sequence(), hot.head_sequence());
        assert!(loaded.head().total_bytes() <= mfm_journal::MAX_RUN_BYTES);
        let cold = runtime.read(&run).await.unwrap();
        assert_eq!(cold.head_digest(), hot.head_digest());
        match (hot.state(), cold.state()) {
            (RunViewState::Failed(hot), RunViewState::Failed(cold)) => {
                assert_eq!(hot.value_ref(), cold.value_ref());
                assert_eq!(hot.canonical_bytes(), cold.canonical_bytes());
            }
            (RunViewState::Succeeded(hot), RunViewState::Succeeded(cold)) => {
                let hot =
                    serde_json::to_value(hot.decode::<CompletedContext<Input, Recipe>>().unwrap())
                        .unwrap();
                let cold =
                    serde_json::to_value(cold.decode::<CompletedContext<Input, Recipe>>().unwrap())
                        .unwrap();
                assert_eq!(hot, cold);
            }
            _ => panic!("cold reconstruction must preserve transaction result"),
        }
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            if scenario == 2 { 2 } else { 1 }
        );
    }
}
