//! Pure checked EIP-1559 wire helpers.

use std::str::FromStr;

use alloy_consensus::{SignableTransaction, Signed, TxEip1559};
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_primitives::{keccak256, Address, Bytes, Signature, TxKind, U256};
use mfm_evm::custody::{ExactRawTransaction, MAX_EXACT_RAW_TRANSACTION_BYTES};
use mfm_evm::{Eip1559TransactionCommand, EvmAddress, EvmHash, EvmU256};
use mfm_signing::{
    recover_public_key, CompactRecoverableSignature, Secp256k1PublicKey, SigningDigest,
};

/// Redaction-safe pure EVM codec failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmCodecError {
    /// Input or wire bytes violate the fixed EVM codec contract.
    #[error("EVM codec input is invalid")]
    Invalid,
}

/// Derives the Ethereum address of one checked uncompressed secp256k1 public key.
pub fn ethereum_address(key: &Secp256k1PublicKey) -> EvmAddress {
    // The checked uncompressed key contains exactly 64 bytes after its SEC1 prefix.
    EvmAddress::from_bytes(Address::from_raw_public_key(&key.as_bytes()[1..]).into_array())
}

/// Computes bounded public Keccak-256 without copying the input.
pub fn evm_keccak256(public_bytes: &[u8]) -> Result<EvmHash, EvmCodecError> {
    if public_bytes.len() > MAX_EXACT_RAW_TRANSACTION_BYTES {
        return Err(EvmCodecError::Invalid);
    }
    Ok(hash(public_bytes))
}

pub(crate) fn transaction_signing_digest(
    command: &Eip1559TransactionCommand,
    nonce: u64,
) -> Result<SigningDigest, EvmCodecError> {
    let transaction = unsigned_transaction(command, nonce)?;
    Ok(SigningDigest::from_bytes(
        transaction.signature_hash().into(),
    ))
}

pub(crate) fn signed_transaction(
    command: &Eip1559TransactionCommand,
    nonce: u64,
    signature: CompactRecoverableSignature,
) -> Result<(ExactRawTransaction, EvmHash), EvmCodecError> {
    if signature.recovery_id() > 1 {
        return Err(EvmCodecError::Invalid);
    }
    let transaction = unsigned_transaction(command, nonce)?;
    let signed = transaction.into_signed(alloy_signature(signature));
    let transaction_hash = EvmHash::from_bytes((*signed.hash()).into());
    let raw = signed.encoded_2718();
    let raw = ExactRawTransaction::new(raw).map_err(|_| EvmCodecError::Invalid)?;
    Ok((raw, transaction_hash))
}

pub(crate) fn validate_signed_transaction(
    command: &Eip1559TransactionCommand,
    nonce: u64,
    expected_hash: &EvmHash,
    raw: &ExactRawTransaction,
    expected_sender: &EvmAddress,
) -> Result<(), EvmCodecError> {
    if &hash(raw.as_bytes()) != expected_hash {
        return Err(EvmCodecError::Invalid);
    }
    if raw.as_bytes().first().copied() != Some(TxEip1559::tx_type() as u8) {
        return Err(EvmCodecError::Invalid);
    }
    let signed = <Signed<TxEip1559> as Decodable2718>::decode_2718_exact(raw.as_bytes())
        .map_err(|_| EvmCodecError::Invalid)?;
    if signed.tx() != &unsigned_transaction(command, nonce)?
        || &EvmHash::from_bytes((*signed.hash()).into()) != expected_hash
    {
        return Err(EvmCodecError::Invalid);
    }

    let digest = SigningDigest::from_bytes(signed.signature_hash().into());
    let signature = compact_signature(signed.signature())?;
    let recovered = recover_public_key(digest, &signature).map_err(|_| EvmCodecError::Invalid)?;
    if &ethereum_address(&recovered) != expected_sender {
        return Err(EvmCodecError::Invalid);
    }
    Ok(())
}

pub(crate) fn create_address(sender: &EvmAddress, nonce: u64) -> EvmAddress {
    let sender = Address::from(*sender.as_bytes());
    EvmAddress::from_bytes(sender.create(nonce).into_array())
}

fn unsigned_transaction(
    command: &Eip1559TransactionCommand,
    nonce: u64,
) -> Result<TxEip1559, EvmCodecError> {
    if nonce == u64::MAX {
        return Err(EvmCodecError::Invalid);
    }
    Ok(TxEip1559 {
        chain_id: command.binding().route.chain_instance.chain_id.get(),
        nonce,
        gas_limit: command.gas_limit().get(),
        max_fee_per_gas: command.max_fee_per_gas(),
        max_priority_fee_per_gas: command.max_priority_fee_per_gas(),
        to: command.to().map_or(TxKind::Create, |address| {
            TxKind::Call(Address::from(*address.as_bytes()))
        }),
        value: parse_u256(command.value())?,
        access_list: Default::default(),
        input: Bytes::copy_from_slice(command.input()),
    })
}

fn parse_u256(value: &EvmU256) -> Result<U256, EvmCodecError> {
    U256::from_str(value.as_str()).map_err(|_| EvmCodecError::Invalid)
}

fn hash(bytes: &[u8]) -> EvmHash {
    EvmHash::from_bytes(keccak256(bytes).into())
}

fn alloy_signature(signature: CompactRecoverableSignature) -> Signature {
    Signature::from_bytes_and_parity(signature.as_bytes(), signature.recovery_id() == 1)
}

fn compact_signature(signature: &Signature) -> Result<CompactRecoverableSignature, EvmCodecError> {
    let [bytes @ .., parity] = signature.as_rsy();
    CompactRecoverableSignature::new(bytes, parity).map_err(|_| EvmCodecError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consensus_signature_conversion_preserves_raw_parity_and_checks_scalars() {
        let mut expected = [0; 64];
        expected[31] = 1;
        expected[63] = 2;
        for parity in [0, 1] {
            let external = Signature::new(U256::from(1), U256::from(2), parity == 1);
            let checked = compact_signature(&external).unwrap();
            assert_eq!(checked.as_bytes(), &expected);
            assert_eq!(checked.recovery_id(), parity);
            assert_eq!(alloy_signature(checked), external);
        }
        assert!(compact_signature(&Signature::new(U256::ZERO, U256::from(2), false)).is_err());
        assert!(compact_signature(&Signature::new(U256::from(1), U256::MAX, false)).is_err());
    }
}
