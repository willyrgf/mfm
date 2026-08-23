//! Pure checked EIP-1559 wire helpers.

use std::str::FromStr;

use alloy_primitives::{hex, keccak256, U256};
use alloy_rlp::{BufMut, Decodable, Encodable, Header};
use mfm_evm::{Eip1559TransactionCommand, EvmAddress, EvmHash, EvmTransactionAction, EvmU256};
use mfm_evm_transaction_authority::{ExactRawTransaction, MAX_EXACT_RAW_TRANSACTION_BYTES};
use mfm_signing::{
    recover_public_key, CompactRecoverableSignature, Secp256k1PublicKey, SigningDigest,
};

const TYPE_2: u8 = 0x02;

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
    let value = format!("0x{}", hex::encode(&digest[12..]));
    EvmAddress::new(value).expect("derived Ethereum address has a fixed valid encoding")
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
    let mut encoded = Vec::new();
    encoded.put_u8(TYPE_2);
    encoded.extend_from_slice(&encode_unsigned(command, nonce)?);
    Ok(SigningDigest::from_bytes(keccak256(&encoded).into()))
}

pub(crate) fn signed_transaction(
    command: &Eip1559TransactionCommand,
    nonce: u64,
    signature: CompactRecoverableSignature,
) -> Result<(ExactRawTransaction, EvmHash), EvmCodecError> {
    if signature.recovery_id() > 1 {
        return Err(EvmCodecError::Invalid);
    }
    let mut payload = transaction_payload(command, nonce)?;
    signature.recovery_id().encode(&mut payload);
    encode_minimal_integer(&signature.as_bytes()[..32], &mut payload);
    encode_minimal_integer(&signature.as_bytes()[32..], &mut payload);

    let mut raw = Vec::with_capacity(2 + payload.len());
    raw.put_u8(TYPE_2);
    Header {
        list: true,
        payload_length: payload.len(),
    }
    .encode(&mut raw);
    raw.extend_from_slice(&payload);
    if raw.len() > MAX_EXACT_RAW_TRANSACTION_BYTES {
        return Err(EvmCodecError::Invalid);
    }
    let transaction_hash = hash(&raw);
    let raw = ExactRawTransaction::new(raw).map_err(|_| EvmCodecError::Invalid)?;
    Ok((raw, transaction_hash))
}

pub(crate) fn validate_signed_transaction(
    command: &Eip1559TransactionCommand,
    nonce: u64,
    expected_hash: &EvmHash,
    raw: &ExactRawTransaction,
    expected_key: &Secp256k1PublicKey,
    expected_sender: &EvmAddress,
) -> Result<(), EvmCodecError> {
    if &hash(raw.as_bytes()) != expected_hash {
        return Err(EvmCodecError::Invalid);
    }
    let decoded = decode_signed(raw.as_bytes())?;
    let input = command
        .action()
        .input_bytes()
        .map_err(|_| EvmCodecError::Invalid)?;
    let expected_target = command
        .action()
        .call_target()
        .map(address_bytes)
        .transpose()?;
    if decoded.chain_id != command.binding().route().chain_instance().chain_id()
        || decoded.nonce != nonce
        || decoded.max_priority_fee_per_gas != parse_u256(command.max_priority_fee_per_gas())?
        || decoded.max_fee_per_gas != parse_u256(command.max_fee_per_gas())?
        || decoded.gas_limit != command.gas_limit()
        || decoded.target != expected_target
        || decoded.value != parse_u256(command.value())?
        || decoded.input != input
    {
        return Err(EvmCodecError::Invalid);
    }

    let digest = transaction_signing_digest(command, nonce)?;
    let recovered =
        recover_public_key(digest, decoded.signature).map_err(|_| EvmCodecError::Invalid)?;
    if &recovered != expected_key
        || &ethereum_address(&recovered) != expected_sender
        || &ethereum_address(expected_key) != expected_sender
    {
        return Err(EvmCodecError::Invalid);
    }
    Ok(())
}

pub(crate) fn create_address(sender: &EvmAddress, nonce: u64) -> Result<EvmAddress, EvmCodecError> {
    let sender = address_bytes(sender)?;
    let mut payload = Vec::new();
    sender.as_slice().encode(&mut payload);
    nonce.encode(&mut payload);
    let mut encoded = Vec::new();
    Header {
        list: true,
        payload_length: payload.len(),
    }
    .encode(&mut encoded);
    encoded.extend_from_slice(&payload);
    let digest = keccak256(&encoded);
    EvmAddress::new(format!("0x{}", hex::encode(&digest[12..]))).map_err(|_| EvmCodecError::Invalid)
}

fn encode_unsigned(
    command: &Eip1559TransactionCommand,
    nonce: u64,
) -> Result<Vec<u8>, EvmCodecError> {
    let payload = transaction_payload(command, nonce)?;
    let mut encoded = Vec::with_capacity(payload.len() + 4);
    Header {
        list: true,
        payload_length: payload.len(),
    }
    .encode(&mut encoded);
    encoded.extend_from_slice(&payload);
    Ok(encoded)
}

fn transaction_payload(
    command: &Eip1559TransactionCommand,
    nonce: u64,
) -> Result<Vec<u8>, EvmCodecError> {
    let mut payload = Vec::new();
    command
        .binding()
        .route()
        .chain_instance()
        .chain_id()
        .encode(&mut payload);
    nonce.encode(&mut payload);
    encode_u256(command.max_priority_fee_per_gas(), &mut payload)?;
    encode_u256(command.max_fee_per_gas(), &mut payload)?;
    command.gas_limit().encode(&mut payload);
    match command.action() {
        EvmTransactionAction::Create { .. } => [].as_slice().encode(&mut payload),
        EvmTransactionAction::Call { to, .. } => {
            address_bytes(to)?.as_slice().encode(&mut payload);
        }
    }
    encode_u256(command.value(), &mut payload)?;
    command
        .action()
        .input_bytes()
        .map_err(|_| EvmCodecError::Invalid)?
        .as_slice()
        .encode(&mut payload);
    Header {
        list: true,
        payload_length: 0,
    }
    .encode(&mut payload);
    Ok(payload)
}

fn encode_u256(value: &EvmU256, output: &mut Vec<u8>) -> Result<(), EvmCodecError> {
    let bytes = parse_u256(value)?.to_be_bytes::<32>();
    encode_minimal_integer(&bytes, output);
    Ok(())
}

fn encode_minimal_integer(bytes: &[u8], output: &mut Vec<u8>) {
    let first = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    bytes[first..].encode(output);
}

fn parse_u256(value: &EvmU256) -> Result<U256, EvmCodecError> {
    U256::from_str(value.as_str()).map_err(|_| EvmCodecError::Invalid)
}

fn address_bytes(address: &EvmAddress) -> Result<[u8; 20], EvmCodecError> {
    let mut bytes = [0u8; 20];
    hex::decode_to_slice(&address.as_str()[2..], &mut bytes).map_err(|_| EvmCodecError::Invalid)?;
    Ok(bytes)
}

fn hash(bytes: &[u8]) -> EvmHash {
    EvmHash::new(format!("{:#x}", keccak256(bytes)))
        .expect("Keccak-256 always renders as one exact EVM hash")
}

struct DecodedTransaction {
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: U256,
    max_fee_per_gas: U256,
    gas_limit: u64,
    target: Option<[u8; 20]>,
    value: U256,
    input: Vec<u8>,
    signature: CompactRecoverableSignature,
}

fn decode_signed(raw: &[u8]) -> Result<DecodedTransaction, EvmCodecError> {
    let (&transaction_type, encoded) = raw.split_first().ok_or(EvmCodecError::Invalid)?;
    if transaction_type != TYPE_2 {
        return Err(EvmCodecError::Invalid);
    }
    let mut outer = encoded;
    let mut payload = Header::decode_bytes(&mut outer, true).map_err(|_| EvmCodecError::Invalid)?;
    if !outer.is_empty() {
        return Err(EvmCodecError::Invalid);
    }

    let chain_id = u64::decode(&mut payload).map_err(|_| EvmCodecError::Invalid)?;
    if chain_id == 0 {
        return Err(EvmCodecError::Invalid);
    }
    let nonce = u64::decode(&mut payload).map_err(|_| EvmCodecError::Invalid)?;
    let max_priority_fee_per_gas = decode_u256(&mut payload)?;
    let max_fee_per_gas = decode_u256(&mut payload)?;
    let gas_limit = u64::decode(&mut payload).map_err(|_| EvmCodecError::Invalid)?;
    let target_bytes =
        Header::decode_bytes(&mut payload, false).map_err(|_| EvmCodecError::Invalid)?;
    let target = match target_bytes.len() {
        0 => None,
        20 => Some(
            target_bytes
                .try_into()
                .map_err(|_| EvmCodecError::Invalid)?,
        ),
        _ => return Err(EvmCodecError::Invalid),
    };
    let value = decode_u256(&mut payload)?;
    let input = Header::decode_bytes(&mut payload, false)
        .map_err(|_| EvmCodecError::Invalid)?
        .to_vec();
    let access_list =
        Header::decode_bytes(&mut payload, true).map_err(|_| EvmCodecError::Invalid)?;
    if !access_list.is_empty() {
        return Err(EvmCodecError::Invalid);
    }
    let recovery_id = u8::decode(&mut payload).map_err(|_| EvmCodecError::Invalid)?;
    if recovery_id > 1 {
        return Err(EvmCodecError::Invalid);
    }
    let r = decode_scalar(&mut payload)?;
    let s = decode_scalar(&mut payload)?;
    if !payload.is_empty() {
        return Err(EvmCodecError::Invalid);
    }
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(&r);
    signature[32..].copy_from_slice(&s);
    let signature = CompactRecoverableSignature::new(signature, recovery_id)
        .map_err(|_| EvmCodecError::Invalid)?;
    Ok(DecodedTransaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit,
        target,
        value,
        input,
        signature,
    })
}

fn decode_u256(payload: &mut &[u8]) -> Result<U256, EvmCodecError> {
    let bytes = Header::decode_bytes(payload, false).map_err(|_| EvmCodecError::Invalid)?;
    if bytes.len() > 32 || bytes.first() == Some(&0) {
        return Err(EvmCodecError::Invalid);
    }
    Ok(U256::from_be_slice(bytes))
}

fn decode_scalar(payload: &mut &[u8]) -> Result<[u8; 32], EvmCodecError> {
    let bytes = Header::decode_bytes(payload, false).map_err(|_| EvmCodecError::Invalid)?;
    if bytes.is_empty() || bytes.len() > 32 || bytes.first() == Some(&0) {
        return Err(EvmCodecError::Invalid);
    }
    let mut scalar = [0u8; 32];
    scalar[32 - bytes.len()..].copy_from_slice(bytes);
    Ok(scalar)
}
