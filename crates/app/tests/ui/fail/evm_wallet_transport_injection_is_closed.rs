use std::sync::Arc;

use mfm_evm_live::transport::EvmJsonRpcTransport;
use mfm_evm_live::{EvmWalletExecutor, EvmWalletRequestQualification};
use mfm_executor::{ExecutorLedgerStore, KeyedExecutorLedger};
use mfm_signing::GenerationGuardedDeterministicSigningProviderBinder;

fn old_two_parameter_shape<Store: ExecutorLedgerStore>() {
    let _: Option<EvmWalletExecutor<Store, EvmJsonRpcTransport>> = None;
}

fn old_injectable_constructor<Store: ExecutorLedgerStore>(
    ledger: KeyedExecutorLedger<Store>,
    transport: EvmJsonRpcTransport,
    signer: GenerationGuardedDeterministicSigningProviderBinder,
    qualification: Arc<EvmWalletRequestQualification>,
) {
    let _ = EvmWalletExecutor::new(ledger, transport, signer, qualification);
}

fn main() {}
