//! Pure checked EIP-1559 wire helpers.

use std::str::FromStr;

use alloy_consensus::{SignableTransaction, Signed, TxEip1559};
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_primitives::{keccak256, Address, Bytes, Signature, TxKind, B256, U256};
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
    let digest = keccak256(&key.as_bytes()[1..]);
    let mut address = [0_u8; 20];
    address.copy_from_slice(&digest[12..]);
    EvmAddress::from_bytes(address)
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
    if raw.len() > MAX_EXACT_RAW_TRANSACTION_BYTES {
        return Err(EvmCodecError::Invalid);
    }
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
    let mut r = [0_u8; 32];
    let mut s = [0_u8; 32];
    r.copy_from_slice(&signature.as_bytes()[..32]);
    s.copy_from_slice(&signature.as_bytes()[32..]);
    Signature::from_scalars_and_parity(B256::from(r), B256::from(s), signature.recovery_id() == 1)
}

fn compact_signature(signature: &Signature) -> Result<CompactRecoverableSignature, EvmCodecError> {
    let mut bytes = [0_u8; 64];
    bytes[..32].copy_from_slice(&signature.r().to_be_bytes::<32>());
    bytes[32..].copy_from_slice(&signature.s().to_be_bytes::<32>());
    CompactRecoverableSignature::new(bytes, u8::from(signature.v()))
        .map_err(|_| EvmCodecError::Invalid)
}
