use mfm_evm::{
    EvmNonceReservationEffect, EvmTransactionEffect, EvmTransactionPreparationEffect,
    ExecuteEvmTransaction, PrepareEvmTransaction, ProjectEvmTransactionOutcome, ReserveEvmNonce,
    TransactionRecipe,
};
use mfm_runtime::RuntimeAssemblyBuilder;
use mfm_values::MfmValue;

/// Registers the four exact State ABIs of one accumulated-context transaction.
///
/// This pure helper installs executable types only. Explicit IO capabilities remain
/// bound separately by [`crate::register_evm_transaction_adapters`].
pub fn register_evm_transaction_states<C: MfmValue, R: TransactionRecipe<C>>(
    builder: &mut RuntimeAssemblyBuilder,
) -> mfm_runtime::Result<()>
where
    PrepareEvmTransaction<C, R>: mfm_program::EffectState<EvmTransactionPreparationEffect>,
    ExecuteEvmTransaction<C, R>: mfm_program::EffectState<EvmTransactionEffect>,
    ProjectEvmTransactionOutcome<C, R>: mfm_program::PureState,
{
    builder.register_effect::<ReserveEvmNonce<C, R>, EvmNonceReservationEffect>()?;
    builder.register_effect::<PrepareEvmTransaction<C, R>, EvmTransactionPreparationEffect>()?;
    builder.register_effect::<ExecuteEvmTransaction<C, R>, EvmTransactionEffect>()?;
    builder.register_pure::<ProjectEvmTransactionOutcome<C, R>>()
}
