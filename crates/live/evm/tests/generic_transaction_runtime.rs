use mfm_evm::{
    EvmNonceReservationEffect, EvmTransactionPreparationEffect, PrepareEvmTransaction,
    ProjectEvmTransactionOutcome, ReserveEvmNonce,
};
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::sync::Arc;

use mfm_evm::{
    Eip1559TransactionCommand, EvmAddress, EvmAuthorityEpoch, EvmBlockAnchor, EvmChainInstance,
    EvmHash, EvmTransactionBinding, EvmTransactionCompletion, EvmTransactionContext,
    EvmTransactionEffect, EvmTransactionReceipt, EvmTransactionReversion, EvmTransactionRoute,
    EvmTransactionSettlement, EvmU256, ExecuteEvmTransaction,
};
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, EntryPointId, RunId, SchemaId,
};
use mfm_program::{expand_program, Operation, OperationExpansion};
use mfm_program_derive::MfmValue;
use mfm_runtime::{EffectAdapterOutcome, RunViewState, Runtime, RuntimeAssemblyBuilder};
use mfm_store::MemoryStore;
use mfm_values::MfmValue as MfmValueTrait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct FirstContext {
    first: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct SecondContext {
    second: String,
}

struct TransactionProgram<K: MfmValueTrait> {
    binding: EvmTransactionBinding,
    marker: PhantomData<fn() -> K>,
}

impl<K: MfmValueTrait> TransactionProgram<K> {
    fn new(binding: EvmTransactionBinding) -> Self {
        Self {
            binding,
            marker: PhantomData,
        }
    }
}

impl<K: MfmValueTrait> Operation for TransactionProgram<K> {
    type Input = EvmTransactionContext<K>;
    type Output = EvmTransactionCompletion<K>;
    type Failure = EvmTransactionReversion<K>;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.effect::<ExecuteEvmTransaction<K>, EvmTransactionEffect>(&self.binding)
    }
}

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

fn command(
    binding: EvmTransactionBinding,
    target: Option<EvmAddress>,
) -> Eip1559TransactionCommand {
    match target {
        None => Eip1559TransactionCommand::create(
            binding,
            vec![1, 2, 3],
            EvmU256::from_u64(0),
            nonzero(100_000),
            EvmU256::from_u64(1),
            EvmU256::from_u64(2),
        ),
        Some(target) => Eip1559TransactionCommand::call(
            binding,
            target,
            vec![4, 5, 6],
            EvmU256::from_u64(0),
            nonzero(100_000),
            EvmU256::from_u64(1),
            EvmU256::from_u64(2),
        ),
    }
    .expect("checked command")
}

fn runtime(binding: &EvmTransactionBinding, store: Arc<MemoryStore>) -> Runtime {
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_effect::<ExecuteEvmTransaction<FirstContext>, EvmTransactionEffect>()
        .expect("first exact State ABI");
    builder
        .register_effect::<ReserveEvmNonce<FirstContext>, EvmNonceReservationEffect>()
        .unwrap();
    builder
        .register_effect::<PrepareEvmTransaction<FirstContext>, EvmTransactionPreparationEffect>()
        .unwrap();
    builder
        .register_pure::<ProjectEvmTransactionOutcome<FirstContext>>()
        .unwrap();
    builder
        .register_effect::<ExecuteEvmTransaction<SecondContext>, EvmTransactionEffect>()
        .expect("second exact State ABI");
    builder
        .register_effect::<ReserveEvmNonce<SecondContext>, EvmNonceReservationEffect>()
        .unwrap();
    builder
        .register_effect::<PrepareEvmTransaction<SecondContext>, EvmTransactionPreparationEffect>()
        .unwrap();
    builder
        .register_pure::<ProjectEvmTransactionOutcome<SecondContext>>()
        .unwrap();
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
    assert_eq!(
        EvmTransactionContext::<FirstContext>::semantic_id().expect("first context semantic"),
        EvmTransactionContext::<SecondContext>::semantic_id().expect("second context semantic")
    );
    assert_ne!(
        mfm_program::nominal_contract_ref::<EvmTransactionContext<FirstContext>>()
            .expect("first context contract"),
        mfm_program::nominal_contract_ref::<EvmTransactionContext<SecondContext>>()
            .expect("second context contract")
    );
    assert_ne!(
        mfm_program::nominal_contract_ref::<EvmTransactionCompletion<FirstContext>>()
            .expect("first completion contract"),
        mfm_program::nominal_contract_ref::<EvmTransactionCompletion<SecondContext>>()
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
                &TransactionProgram::<FirstContext>::new(binding.clone()),
            )
            .expect("first Program"),
            EvmTransactionContext::new(FirstContext { first: 12 }, command(binding.clone(), None)),
        )
        .await
        .expect("first hot execution");
    let RunViewState::Succeeded(first_hot_value) = first_hot.state() else {
        panic!("first transaction did not succeed")
    };
    assert_eq!(
        first_hot_value.contract_ref(),
        &mfm_program::nominal_contract_ref::<EvmTransactionCompletion<FirstContext>>()
            .expect("first completion contract")
    );

    let second_run_id = RunId::from_digest(DigestBytes::from_array([13; 32]));
    let second_hot = hot_runtime
        .start(
            second_run_id.clone(),
            expand_program(
                EntryPointId::new("mfm.test.evm-live/second-generic-transaction@1")
                    .expect("second entry point"),
                &TransactionProgram::<SecondContext>::new(binding.clone()),
            )
            .expect("second Program"),
            EvmTransactionContext::new(
                SecondContext {
                    second: "two".to_owned(),
                },
                command(binding.clone(), Some(EvmAddress::from_bytes([14; 20]))),
            ),
        )
        .await
        .expect("second hot execution");
    let RunViewState::Succeeded(second_hot_value) = second_hot.state() else {
        panic!("second transaction did not succeed")
    };
    assert_eq!(
        second_hot_value.contract_ref(),
        &mfm_program::nominal_contract_ref::<EvmTransactionCompletion<SecondContext>>()
            .expect("second completion contract")
    );
    assert_ne!(
        first_hot_value.canonical_bytes(),
        second_hot_value.canonical_bytes()
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
