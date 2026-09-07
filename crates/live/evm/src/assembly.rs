use mfm_evm::{
    CompletedContext, CompletedTransactionFacts, EvmNonceReservationEffect, EvmTransactionEffect,
    EvmTransactionPreparationEffect, ExecuteEvmTransaction, ExecutedContext,
    ExecutedTransactionFacts, PrepareEvmTransaction, PreparedContext, PreparedTransactionFacts,
    ProjectEvmTransactionOutcome, ReserveEvmNonce, ReservedContext, ReservedEvmTransaction,
    TransactionRecipe,
};
use mfm_runtime::RuntimeAssemblyBuilder;
use mfm_values::{ContextSlot, MfmValue};

/// Registers the four exact State ABIs of one accumulated-context transaction.
///
/// This pure helper installs executable types only. Explicit IO capabilities remain
/// bound separately by [`crate::register_evm_transaction_adapters`].
pub fn register_evm_transaction_states<C: MfmValue, R: TransactionRecipe<C>>(
    builder: &mut RuntimeAssemblyBuilder,
) -> mfm_runtime::Result<()>
where
    R::Slot: ContextSlot<
            ReservedContext<C, R>,
            Value = ReservedEvmTransaction,
            With<PreparedTransactionFacts> = PreparedContext<C, R>,
        > + ContextSlot<
            PreparedContext<C, R>,
            Value = PreparedTransactionFacts,
            With<ExecutedTransactionFacts> = ExecutedContext<C, R>,
        > + ContextSlot<
            ExecutedContext<C, R>,
            Value = ExecutedTransactionFacts,
            With<CompletedTransactionFacts<R::Success>> = CompletedContext<C, R>,
        >,
{
    builder.register_effect::<ReserveEvmNonce<C, R>, EvmNonceReservationEffect>()?;
    builder.register_effect::<PrepareEvmTransaction<C, R>, EvmTransactionPreparationEffect>()?;
    builder.register_effect::<ExecuteEvmTransaction<C, R>, EvmTransactionEffect>()?;
    builder.register_pure::<ProjectEvmTransactionOutcome<C, R>>()
}
