use std::sync::Arc;

use mfm_evm_live::{EvmWalletExecutor, EvmWalletRequestQualification};
use mfm_executor::{ExecutorLedgerStore, KeyedExecutorLedger};
use mfm_signing::GenerationGuardedDeterministicSigningProviderBinder;

fn construct<Store: ExecutorLedgerStore>(
    ledger: KeyedExecutorLedger<Store>,
    signer: GenerationGuardedDeterministicSigningProviderBinder,
    qualification: Arc<EvmWalletRequestQualification>,
) -> mfm_executor::Result<EvmWalletExecutor<Store>> {
    EvmWalletExecutor::new(ledger, signer, qualification)
}

fn main() {}
