use mfm_capabilities::AdapterError;
use mfm_evm::*;
use mfm_ids::{DigestBytes, EntryPointId, RunId};
use mfm_program::{ConclusionBound, EffectBounds, FromNever, ProgramLimits};
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
type Failure = EvmTransactionFailure<ExecutedContext<Input, Recipe>>;

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
    let input_bytes = mfm_values::canonicalize_mfm_value(&input)
        .unwrap()
        .0
        .as_bytes()
        .len() as u64;
    let command_bytes = mfm_values::canonicalize_mfm_value(&input.transaction.command())
        .unwrap()
        .0
        .as_bytes()
        .len() as u64;
    // Retained reservation, preparation and settlement add only fixed-width identifiers,
    // quantities and closed outcome fields. 16 KiB covers these complete facts and keys;
    // adding the command again conservatively avoids assuming a serialized slot subtraction.
    let context_bytes = input_bytes + command_bytes + 16_384;
    // The frame envelope allowance includes repeated content references and recovery wire.
    let envelope = 16_384;
    let bounds = EvmTransactionBounds {
        reservation: EffectBounds::new(command_bytes + envelope, context_bytes + 4096 + envelope)
            .unwrap(),
        preparation: EffectBounds::new(
            command_bytes + 4096 + envelope,
            context_bytes + 4096 + envelope,
        )
        .unwrap(),
        execution: EffectBounds::new(
            command_bytes + 4096 + envelope,
            context_bytes + 4096 + envelope,
        )
        .unwrap(),
        // Account for both original and mapped root failure even though Identity may deduplicate.
        projection: ConclusionBound::new(2 * context_bytes + envelope).unwrap(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/recovery-transaction@1").unwrap(),
        &EvmTransaction::<Input, Recipe>::new(binding.clone(), bounds),
        &input,
        ProgramLimits::new(1),
    )
    .unwrap();
    for scenario in 0..3 {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder
            .register_effect::<ReserveEvmNonce<Input, Recipe>, EvmNonceReservationEffect>()
            .unwrap();
        builder.register_effect::<PrepareEvmTransaction<Input, Recipe>, EvmTransactionPreparationEffect>().unwrap();
        builder
            .register_effect::<ExecuteEvmTransaction<Input, Recipe>, EvmTransactionEffect>()
            .unwrap();
        builder
            .register_pure::<ProjectEvmTransactionOutcome<Input, Recipe>>()
            .unwrap();
        builder.register_map::<FromNever<Failure>>().unwrap();
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
                                    cause: EvmOperationalError::Timeout,
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
        if scenario == 0 {
            let oversized = mfm_program::expand_program(
                EntryPointId::new("mfm.test/recovery-transaction@1").unwrap(),
                &EvmTransaction::<Input, Recipe>::new(binding.clone(), bounds),
                &input,
                ProgramLimits::new(u32::MAX),
            )
            .unwrap();
            assert!(matches!(
                runtime.start(run.clone(), oversized, input.clone()).await,
                Err(InvocationFailure::Execution {
                    error: mfm_runtime::RuntimeError::Capacity,
                    last_observed: None,
                    ..
                })
            ));
            assert!(store.load_run(&run).await.unwrap().is_none());
            assert_eq!(attempts.load(Ordering::SeqCst), 0);
        }
        let hot = match runtime
            .start(run.clone(), program.clone(), input.clone())
            .await
        {
            Ok(view) if scenario != 2 => view,
            Err(InvocationFailure::RecoveryStopped {
                observed, incident, ..
            }) if scenario == 2 => {
                assert_eq!(observed.head_sequence(), 6);
                assert!(matches!(
                    incident
                        .state_context
                        .decode::<EvmTransactionAdapterContext>()
                        .unwrap(),
                    EvmTransactionAdapterContext::Execution { .. }
                ));
                let cold = runtime.read(&run).await.unwrap();
                assert_eq!(cold.head_digest(), observed.head_digest());
                let (
                    RunViewState::EffectPending {
                        position: hot_position,
                        effect_id: hot_id,
                    },
                    RunViewState::EffectPending {
                        position: cold_position,
                        effect_id: cold_id,
                    },
                ) = (observed.state(), cold.state())
                else {
                    panic!("unchanged pending execution authority")
                };
                assert_eq!(hot_position, cold_position);
                assert_eq!(hot_id, cold_id);
                assert_eq!(attempts.load(Ordering::SeqCst), 1);
                runtime.resume(&run).await.unwrap()
            }
            other => panic!("unexpected transaction result: {}", other.is_ok()),
        };
        assert_eq!(hot.head_sequence(), 8);
        assert!(matches!(
            (scenario, hot.state()),
            (1, RunViewState::Failed(_)) | (0 | 2, RunViewState::Succeeded(_))
        ));
        let history = mfm_journal::JournalHistory::qualify(
            &run,
            store.load_run(&run).await.unwrap().unwrap(),
        )
        .unwrap();
        let frame_lengths = history.frame_lengths().collect::<Vec<_>>();
        let cost = program
            .history_bound(ConclusionBound::new(frame_lengths[0] as u64).unwrap())
            .unwrap();
        assert_eq!(cost.frames(), 15);
        assert!(cost.bytes() <= mfm_journal::MAX_RUN_BYTES);
        assert!(history.total_bytes() <= cost.bytes());
        eprintln!(
            "scenario {scenario}: frames {frame_lengths:?}; actual {} bytes, admitted {} bytes",
            history.total_bytes(),
            cost.bytes()
        );
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
