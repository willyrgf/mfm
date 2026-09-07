use mfm_evm::{
    CheckedCreatePlan, CompletedContext, CompletedTransactionFacts, CreateAt, Created, EvmAddress,
    EvmAuthorityEpoch, EvmBlockAnchor, EvmChainInstance, EvmHash, EvmNonceReservationEffect,
    EvmTransaction, EvmTransactionBinding, EvmTransactionEffect, EvmTransactionPreparationEffect,
    EvmTransactionReceipt, EvmTransactionRoute, EvmTransactionSettlement, EvmU256,
};
use mfm_evm_live::register_evm_transaction_states;
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, EntryPointId, RunId, SchemaId,
};
use mfm_program::expand_program;
use mfm_program_derive::{MfmContext, MfmValue};
use mfm_runtime::{EffectAdapterOutcome, RunViewState, Runtime, RuntimeAssemblyBuilder};
use mfm_store::MemoryStore;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.evm-live.first")]
#[serde(deny_unknown_fields)]
struct FirstContext<T> {
    first: u64,
    transaction: T,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.evm-live.second")]
#[serde(deny_unknown_fields)]
struct SecondContext<T> {
    second: String,
    transaction: T,
}
type FirstInitial = FirstContext<CheckedCreatePlan>;
type SecondInitial = SecondContext<CheckedCreatePlan>;
type FirstRecipe = CreateAt<FirstContextTransactionSlot>;
type SecondRecipe = CreateAt<SecondContextTransactionSlot>;

fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).expect("nonzero fixture")
}

fn binding() -> EvmTransactionBinding {
    let endpoint_ref = ContentRef::new(
        SchemaId::new(
            "mfm.test.endpoint",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        )
        .expect("endpoint schema"),
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([2; 32])),
    )
    .expect("endpoint ref");
    EvmTransactionBinding::new(
        EvmTransactionRoute::new(
            EvmChainInstance::new(nonzero(1), EvmHash::from_bytes([3; 32])),
            endpoint_ref,
        ),
        EvmAuthorityEpoch::new([4; 32]),
        EvmAddress::from_bytes([5; 20]),
    )
}

fn plan(binding: EvmTransactionBinding) -> CheckedCreatePlan {
    CheckedCreatePlan::new(
        binding,
        vec![1, 2, 3],
        EvmU256::from_u64(0),
        nonzero(100_000),
        EvmU256::from_u64(1),
        EvmU256::from_u64(2),
    )
    .unwrap()
}

fn runtime(binding: &EvmTransactionBinding, store: Arc<MemoryStore>) -> Runtime {
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    register_evm_transaction_states::<FirstInitial, FirstRecipe>(&mut builder).unwrap();
    register_evm_transaction_states::<SecondInitial, SecondRecipe>(&mut builder).unwrap();
    builder
        .register_effect_adapter::<EvmTransactionEffect, EvmTransactionBinding, _>(
            binding.clone(),
            |effect_id, _command_value_ref, command| {
                let effect_id = effect_id.clone();
                let created = command.reserved().command().to().is_none();
                let nonce = command.reserved().reservation().nonce();
                Box::pin(async move {
                    let receipt = EvmTransactionReceipt::new(
                        EvmBlockAnchor::new(EvmU256::from_u64(7), EvmHash::from_bytes([8; 32])),
                        EvmHash::from_bytes([9; 32]),
                    );
                    let evidence = if created {
                        EvmTransactionSettlement::created(
                            effect_id,
                            nonce,
                            receipt,
                            EvmAddress::from_bytes([10; 20]),
                        )
                    } else {
                        EvmTransactionSettlement::called(effect_id, nonce, receipt)
                    };
                    Ok(EffectAdapterOutcome::Settled(evidence))
                })
            },
        )
        .expect("shared transaction adapter");
    builder
        .register_effect_adapter::<EvmNonceReservationEffect, EvmTransactionBinding, _>(
            binding.clone(),
            |id, reference, command| {
                let reservation = mfm_evm::Reservation::new(
                    id.clone(),
                    reference.clone(),
                    mfm_evm::NonceDomain::from_binding(command.binding()),
                    0,
                )
                .unwrap();
                Box::pin(async move { Ok(EffectAdapterOutcome::Settled(reservation)) })
            },
        )
        .unwrap();
    builder
        .register_effect_adapter::<EvmTransactionPreparationEffect, EvmTransactionBinding, _>(
            binding.clone(),
            |id, _, _| {
                let evidence = mfm_evm::PreparedEvmTransactionEvidence::new(
                    id.clone(),
                    EvmHash::from_bytes([9; 32]),
                );
                Box::pin(async move { Ok(EffectAdapterOutcome::Settled(evidence)) })
            },
        )
        .unwrap();
    Runtime::new(builder.finish(), store)
}

#[tokio::test]
async fn one_transaction_state_selects_multiple_exact_generic_codecs_hot_and_cold() {
    assert_ne!(
        mfm_program::nominal_contract_ref::<FirstInitial>().expect("first context contract"),
        mfm_program::nominal_contract_ref::<SecondInitial>().expect("second context contract")
    );
    assert_ne!(
        mfm_program::nominal_contract_ref::<CompletedContext<FirstInitial, FirstRecipe>>()
            .expect("first completion contract"),
        mfm_program::nominal_contract_ref::<CompletedContext<SecondInitial, SecondRecipe>>()
            .expect("second completion contract")
    );

    let binding = binding();
    let store = Arc::new(MemoryStore::new());
    let hot_runtime = runtime(&binding, Arc::clone(&store));
    let first_run_id = RunId::from_digest(DigestBytes::from_array([11; 32]));
    let first_hot = hot_runtime
        .start(
            first_run_id.clone(),
            expand_program(
                EntryPointId::new("mfm.test.evm-live/first-generic-transaction@1")
                    .expect("first entry point"),
                &EvmTransaction::<FirstInitial, FirstRecipe>::new(binding.clone()),
            )
            .expect("first Program"),
            FirstContext {
                first: 12,
                transaction: plan(binding.clone()),
            },
        )
        .await
        .expect("first hot execution");
    let RunViewState::Succeeded(first_hot_value) = first_hot.state() else {
        panic!("first transaction did not succeed")
    };
    assert_eq!(
        first_hot_value.contract_ref(),
        &mfm_program::nominal_contract_ref::<CompletedContext<FirstInitial, FirstRecipe>>()
            .expect("first completion contract")
    );

    let first_report: FirstContext<CompletedTransactionFacts<Created>> =
        serde_json::from_slice(first_hot_value.canonical_bytes()).unwrap();
    assert_eq!(first_report.first, 12);
    assert_eq!(
        first_report.transaction.command(),
        &plan(binding.clone()).command()
    );
    assert_eq!(first_report.transaction.reservation().nonce(), 0);
    assert_eq!(
        first_report.transaction.preparation().transaction_hash(),
        &EvmHash::from_bytes([9; 32])
    );
    assert_eq!(
        first_report.transaction.settlement().transaction_hash(),
        first_report.transaction.preparation().transaction_hash()
    );
    assert_eq!(
        first_report.transaction.outcome().created_address(),
        &EvmAddress::from_bytes([10; 20])
    );

    let second_run_id = RunId::from_digest(DigestBytes::from_array([13; 32]));
    let second_hot = hot_runtime
        .start(
            second_run_id.clone(),
            expand_program(
                EntryPointId::new("mfm.test.evm-live/second-generic-transaction@1")
                    .expect("second entry point"),
                &EvmTransaction::<SecondInitial, SecondRecipe>::new(binding.clone()),
            )
            .expect("second Program"),
            SecondContext {
                second: "two".to_owned(),
                transaction: plan(binding.clone()),
            },
        )
        .await
        .expect("second hot execution");
    let RunViewState::Succeeded(second_hot_value) = second_hot.state() else {
        panic!("second transaction did not succeed")
    };
    assert_eq!(
        second_hot_value.contract_ref(),
        &mfm_program::nominal_contract_ref::<CompletedContext<SecondInitial, SecondRecipe>>()
            .expect("second completion contract")
    );
    assert_ne!(
        first_hot_value.canonical_bytes(),
        second_hot_value.canonical_bytes()
    );

    let second_report: SecondContext<CompletedTransactionFacts<Created>> =
        serde_json::from_slice(second_hot_value.canonical_bytes()).unwrap();
    assert_eq!(second_report.second, "two");
    assert_eq!(
        second_report.transaction.command(),
        &plan(binding.clone()).command()
    );

    let cold_runtime = runtime(&binding, store);
    let first_cold = cold_runtime
        .read(&first_run_id)
        .await
        .expect("first cold read");
    let RunViewState::Succeeded(first_cold_value) = first_cold.state() else {
        panic!("first cold transaction did not succeed")
    };
    assert_eq!(first_cold.head_digest(), first_hot.head_digest());
    assert_eq!(
        first_cold_value.canonical_bytes(),
        first_hot_value.canonical_bytes()
    );

    let second_cold = cold_runtime
        .read(&second_run_id)
        .await
        .expect("second cold read");
    let RunViewState::Succeeded(second_cold_value) = second_cold.state() else {
        panic!("second cold transaction did not succeed")
    };
    assert_eq!(second_cold.head_digest(), second_hot.head_digest());
    assert_eq!(
        second_cold_value.canonical_bytes(),
        second_hot_value.canonical_bytes()
    );
}

struct WrongMode;
impl mfm_evm::TransactionRecipe<FirstInitial> for WrongMode {
    type Slot = FirstContextTransactionSlot;
    type Success = mfm_evm::Called;
    fn recipe_id() -> mfm_values::Result<mfm_ids::StableId> {
        Ok(mfm_ids::StableId::new("mfm.test.wrong-mode@1").unwrap())
    }
    fn source_ids() -> mfm_values::Result<Vec<mfm_ids::StableId>> {
        <FirstRecipe as mfm_evm::TransactionRecipe<FirstInitial>>::source_ids()
    }
    fn command(context: &FirstInitial) -> mfm_evm::Eip1559TransactionCommand {
        context.transaction.command()
    }
}

#[tokio::test]
async fn an_incompatible_recipe_mode_is_internal_before_prepare_append_or_adapter_entry() {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    register_evm_transaction_states::<FirstInitial, WrongMode>(&mut builder).unwrap();
    builder
        .register_effect_adapter::<EvmNonceReservationEffect, _, _>(binding(), |_, _, _| {
            panic!("invalid recipe entered reservation IO")
        })
        .unwrap();
    builder
        .register_effect_adapter::<EvmTransactionPreparationEffect, _, _>(binding(), |_, _, _| {
            panic!("invalid recipe entered preparation IO")
        })
        .unwrap();
    builder
        .register_effect_adapter::<EvmTransactionEffect, _, _>(binding(), |_, _, _| {
            panic!("invalid recipe entered execution IO")
        })
        .unwrap();
    let runtime = Runtime::new(builder.finish(), Arc::new(MemoryStore::new()));
    let id = RunId::from_digest(DigestBytes::from_array([50; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test/wrong-mode@1").unwrap(),
        &EvmTransaction::<FirstInitial, WrongMode>::new(binding()),
    )
    .unwrap();
    assert!(matches!(
        runtime
            .start(
                id.clone(),
                program,
                FirstContext {
                    first: 1,
                    transaction: plan(binding())
                }
            )
            .await,
        Err(mfm_runtime::RuntimeError::Internal)
    ));
    let retained = runtime.read(&id).await.unwrap();
    assert_eq!(retained.head_sequence(), 1);
    assert!(matches!(retained.state(), RunViewState::Runnable));
}

#[derive(Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.selected-source")]
struct SourceContext<A, B, T> {
    a: A,
    b: B,
    transaction: T,
}
type Sources = SourceContext<
    CompletedTransactionFacts<Created>,
    CompletedTransactionFacts<Created>,
    mfm_evm::CheckedCallPlan,
>;
type FromA = mfm_evm::CallCreatedAt<SourceContextTransactionSlot, SourceContextASlot>;
type FromB = mfm_evm::CallCreatedAt<SourceContextTransactionSlot, SourceContextBSlot>;

#[test]
fn same_typed_source_selection_changes_executable_identity_without_changing_value_abis() {
    use mfm_evm::ReserveEvmNonce;
    use mfm_program::{nominal_contract_ref, State};
    assert_ne!(
        ReserveEvmNonce::<Sources, FromA>::state_id().unwrap(),
        ReserveEvmNonce::<Sources, FromB>::state_id().unwrap()
    );
    assert_eq!(
        nominal_contract_ref::<<ReserveEvmNonce<Sources, FromA> as State>::Input>().unwrap(),
        nominal_contract_ref::<<ReserveEvmNonce<Sources, FromB> as State>::Input>().unwrap()
    );
    assert_eq!(
        nominal_contract_ref::<<ReserveEvmNonce<Sources, FromA> as State>::Output>().unwrap(),
        nominal_contract_ref::<<ReserveEvmNonce<Sources, FromB> as State>::Output>().unwrap()
    );
    assert_ne!(
        ReserveEvmNonce::<FirstInitial, FirstRecipe>::state_id().unwrap(),
        ReserveEvmNonce::<FirstInitial, WrongMode>::state_id().unwrap()
    );
}

#[tokio::test]
async fn selecting_another_same_typed_source_requires_its_own_assembly_before_io() {
    use mfm_evm::{
        ExecutedTransactionFacts, NonceDomain, PreparedEvmTransactionEvidence,
        PreparedTransactionFacts, Reservation, ReservedEvmTransaction, TransactionRecipe,
    };
    let command = plan(binding()).command();
    let reservation = Reservation::new(
        mfm_ids::EffectId::from_digest(DigestBytes::from_array([1; 32])),
        mfm_values::canonicalize_mfm_value(&command).unwrap().1,
        NonceDomain::from_binding(command.binding()),
        0,
    )
    .unwrap();
    let prepared = PreparedTransactionFacts::new(
        ReservedEvmTransaction::new(command, reservation).unwrap(),
        PreparedEvmTransactionEvidence::new(
            mfm_ids::EffectId::from_digest(DigestBytes::from_array([2; 32])),
            EvmHash::from_bytes([9; 32]),
        ),
    );
    let [a, b] = [3, 4].map(|byte| {
        CompletedTransactionFacts::<Created>::new(
            ExecutedTransactionFacts::new(
                prepared.clone(),
                EvmTransactionSettlement::created(
                    mfm_ids::EffectId::from_digest(DigestBytes::from_array([byte; 32])),
                    0,
                    EvmTransactionReceipt::new(
                        EvmBlockAnchor::new(EvmU256::from_u64(7), EvmHash::from_bytes([8; 32])),
                        EvmHash::from_bytes([9; 32]),
                    ),
                    EvmAddress::from_bytes([byte; 20]),
                ),
            )
            .unwrap(),
        )
        .unwrap()
    });
    let input = SourceContext {
        a,
        b,
        transaction: mfm_evm::CheckedCallPlan::new(
            binding(),
            vec![],
            EvmU256::from_u64(0),
            nonzero(1),
            EvmU256::from_u64(1),
            EvmU256::from_u64(2),
        )
        .unwrap(),
    };
    assert_eq!(
        FromA::command(&input).to(),
        Some(&EvmAddress::from_bytes([3; 20]))
    );
    assert_eq!(
        FromB::command(&input).to(),
        Some(&EvmAddress::from_bytes([4; 20]))
    );
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    register_evm_transaction_states::<Sources, FromA>(&mut builder).unwrap();
    builder
        .register_effect_adapter::<EvmNonceReservationEffect, _, _>(binding(), |_, _, _| {
            panic!("incompatible source entered reservation IO")
        })
        .unwrap();
    builder
        .register_effect_adapter::<EvmTransactionPreparationEffect, _, _>(binding(), |_, _, _| {
            panic!("incompatible source entered preparation IO")
        })
        .unwrap();
    builder
        .register_effect_adapter::<EvmTransactionEffect, _, _>(binding(), |_, _, _| {
            panic!("incompatible source entered execution IO")
        })
        .unwrap();
    let runtime = Runtime::new(builder.finish(), Arc::new(MemoryStore::new()));
    let id = RunId::from_digest(DigestBytes::from_array([51; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test/selected-source@1").unwrap(),
        &EvmTransaction::<Sources, FromB>::new(binding()),
    )
    .unwrap();
    assert!(matches!(
        runtime.start(id.clone(), program, input).await,
        Err(mfm_runtime::RuntimeError::IncompatibleAssembly)
    ));
    assert!(matches!(
        runtime.read(&id).await,
        Err(mfm_runtime::RuntimeError::Absent)
    ));
}

struct CustomCreate;
impl mfm_evm::TransactionRecipe<FirstInitial> for CustomCreate {
    type Slot = FirstContextTransactionSlot;
    type Success = Created;
    fn recipe_id() -> mfm_values::Result<mfm_ids::StableId> {
        Ok(mfm_ids::StableId::new("mfm.test.custom-create@1").unwrap())
    }
    fn source_ids() -> mfm_values::Result<Vec<mfm_ids::StableId>> {
        <FirstRecipe as mfm_evm::TransactionRecipe<FirstInitial>>::source_ids()
    }
    fn command(context: &FirstInitial) -> mfm_evm::Eip1559TransactionCommand {
        context.transaction.command()
    }
}

#[test]
fn custom_recipe_identity_is_committed_even_when_slots_mode_and_command_are_equal() {
    use mfm_evm::{ReserveEvmNonce, TransactionRecipe};
    use mfm_program::State;
    let input = FirstContext {
        first: 1,
        transaction: plan(binding()),
    };
    assert_eq!(FirstRecipe::command(&input), CustomCreate::command(&input));
    assert_eq!(
        <FirstRecipe as TransactionRecipe<FirstInitial>>::source_ids().unwrap(),
        CustomCreate::source_ids().unwrap()
    );
    assert_ne!(
        ReserveEvmNonce::<FirstInitial, FirstRecipe>::state_id().unwrap(),
        ReserveEvmNonce::<FirstInitial, CustomCreate>::state_id().unwrap()
    );
}
