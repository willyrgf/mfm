use mfm_state_evm_contracts::ContractTransactionIntent;

fn main() {
    let intent = intent();
    let _ = intent.raw_transaction;
}

fn intent() -> ContractTransactionIntent {
    unimplemented!()
}
