//! Pure checked EIP-1559 wire helpers.

use std::str::FromStr;

#[cfg(test)]
use alloy_primitives::hex;
use alloy_primitives::{keccak256, U256};
use alloy_rlp::{BufMut, Decodable, Encodable, Header};
use mfm_evm::{Eip1559TransactionCommand, EvmAddress, EvmHash, EvmU256};
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
    let expected_target = command.to().map(|address| *address.as_bytes());
    if decoded.chain_id != command.binding().route().chain_instance().chain_id().get()
        || decoded.nonce != nonce
        || decoded.max_priority_fee_per_gas != parse_u256(command.max_priority_fee_per_gas())?
        || decoded.max_fee_per_gas != parse_u256(command.max_fee_per_gas())?
        || decoded.gas_limit != command.gas_limit().get()
        || decoded.target != expected_target
        || decoded.value != parse_u256(command.value())?
        || decoded.input != command.input()
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
    let sender = sender.as_bytes();
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
    let mut address = [0_u8; 20];
    address.copy_from_slice(&digest[12..]);
    Ok(EvmAddress::from_bytes(address))
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
        .get()
        .encode(&mut payload);
    nonce.encode(&mut payload);
    encode_u256(command.max_priority_fee_per_gas(), &mut payload)?;
    encode_u256(command.max_fee_per_gas(), &mut payload)?;
    command.gas_limit().get().encode(&mut payload);
    match command.to() {
        None => [].as_slice().encode(&mut payload),
        Some(to) => to.as_bytes().as_slice().encode(&mut payload),
    }
    encode_u256(command.value(), &mut payload)?;
    command.input().encode(&mut payload);
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

fn hash(bytes: &[u8]) -> EvmHash {
    EvmHash::from_bytes(keccak256(bytes).into())
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

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_RAW: &str = "02f85382053907020a830186a08080826000c001a07ffd3c6f6e2217de62458b59faca6e9a3a829c7bcf9ebaa04e0414c1eb0d0419a06f5a761a7bb9c0dab816d83eb5472e2dfd61c8da2eca46924c500c54efe5d58b";

    fn valid_raw() -> Vec<u8> {
        hex::decode(VALID_RAW).expect("frozen raw transaction")
    }

    fn position(raw: &[u8], pattern: &[u8]) -> usize {
        raw.windows(pattern.len())
            .position(|window| window == pattern)
            .expect("frozen pattern")
    }

    #[test]
    fn signed_decoder_rejects_every_noncanonical_or_unsupported_boundary() {
        assert!(decode_signed(&valid_raw()).is_ok());

        let mut leading_zero_nonce = valid_raw();
        let nonce = position(&leading_zero_nonce, &[0x82, 0x05, 0x39, 0x07]) + 3;
        leading_zero_nonce.splice(nonce..=nonce, [0x82, 0x00, 0x07]);
        leading_zero_nonce[2] += 2;

        let mut nonempty_access_list = valid_raw();
        let access_list = position(&nonempty_access_list, &[0x82, 0x60, 0x00, 0xc0, 0x01]) + 3;
        let mut encoded_access_list = vec![0xd7, 0xd6, 0x94];
        encoded_access_list.extend_from_slice(&[0; 20]);
        encoded_access_list.push(0xc0);
        nonempty_access_list.splice(access_list..=access_list, encoded_access_list);
        nonempty_access_list[2] += 23;

        let mut invalid_parity = valid_raw();
        let parity = position(&invalid_parity, &[0xc0, 0x01, 0xa0]) + 1;
        invalid_parity[parity] = 2;

        let mut zero_scalar = valid_raw();
        let scalar = zero_scalar
            .iter()
            .rposition(|byte| *byte == 0xa0)
            .expect("scalar header");
        zero_scalar[scalar + 1..scalar + 33].fill(0);

        let mut high_s = valid_raw();
        let scalar = high_s
            .iter()
            .rposition(|byte| *byte == 0xa0)
            .expect("scalar header");
        high_s[scalar + 1..scalar + 33].copy_from_slice(&[
            0x91, 0xb4, 0xf9, 0x86, 0xaa, 0x69, 0xaa, 0xac, 0xb4, 0xe3, 0xa8, 0xa4, 0x67, 0xa8,
            0x0e, 0x0c, 0x5f, 0x60, 0x2d, 0xc8, 0xd1, 0x77, 0xb0, 0x37, 0xcc, 0x60, 0x5e, 0x63,
            0x3d, 0x74, 0x14, 0x8d,
        ]);

        let mut trailing = valid_raw();
        trailing.push(0);
        let mut malformed_list = valid_raw();
        malformed_list[2] -= 1;
        let mut noncanonical_list = valid_raw();
        noncanonical_list.splice(1..3, [0xf9, 0x00, 0x53]);

        for invalid in [
            leading_zero_nonce,
            nonempty_access_list,
            trailing,
            malformed_list,
            noncanonical_list,
            invalid_parity,
            high_s,
            zero_scalar,
        ] {
            assert!(decode_signed(&invalid).is_err());
        }
    }
}
