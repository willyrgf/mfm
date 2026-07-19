use alloy_primitives::B256;
use mfm_evm_signing::TransientSignedEip1559Envelope;

fn main() {
    let _payload = TransientSignedEip1559Envelope {
        bytes: vec![0x01],
        transaction_hash: B256::ZERO,
    };
}
