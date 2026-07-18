//! Canonical EVM primitives shared by transaction and validation states.

use std::str::FromStr;

use alloy_primitives::{Address, B256, U256};
use mfm_evm_capabilities::{
    EvmBlockAnchor, EvmSessionEvidence, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};

use crate::EvmStateError;

pub(crate) fn parse_address(value: &str) -> Result<Address, EvmStateError> {
    let address = Address::from_str(value).map_err(|_| invalid("EVM address was invalid"))?;
    if canonical_address(address) != value {
        return Err(invalid("EVM address was not canonical"));
    }
    Ok(address)
}

pub(crate) fn parse_hash(value: &str) -> Result<B256, EvmStateError> {
    let hash = B256::from_str(value).map_err(|_| invalid("EVM hash was invalid"))?;
    if canonical_hash(hash) != value {
        return Err(invalid("EVM hash was not canonical"));
    }
    Ok(hash)
}

pub(crate) fn parse_quantity(value: &str) -> Result<U256, EvmStateError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("EVM quantity was not canonical decimal"));
    }
    let quantity = U256::from_str(value).map_err(|_| invalid("EVM quantity exceeded U256"))?;
    if quantity.to_string() != value {
        return Err(invalid("EVM quantity was not canonical decimal"));
    }
    Ok(quantity)
}

pub(crate) fn parse_bytes(value: &str, maximum: usize) -> Result<Vec<u8>, EvmStateError> {
    let body = value
        .strip_prefix("0x")
        .ok_or_else(|| invalid("EVM bytes lacked canonical prefix"))?;
    if body.len() % 2 != 0 || body.len() / 2 > maximum {
        return Err(invalid("EVM bytes had invalid or excessive length"));
    }
    let bytes = hex::decode(body).map_err(|_| invalid("EVM bytes were invalid hex"))?;
    if canonical_bytes(&bytes) != value {
        return Err(invalid("EVM bytes were not canonical"));
    }
    Ok(bytes)
}

pub(crate) fn canonical_address(value: Address) -> String {
    format!("{value:#x}")
}

pub(crate) fn canonical_hash(value: B256) -> String {
    format!("{value:#x}")
}

pub(crate) fn canonical_bytes(value: &[u8]) -> String {
    format!("0x{}", hex::encode(value))
}

pub(crate) fn validate_session(
    session: &EvmSessionEvidence,
    network_id: &str,
    chain_id: u64,
) -> Result<(), EvmStateError> {
    if session.network_id() != network_id
        || session.chain_id() != chain_id
        || session.implementation_id() != EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    {
        return Err(invalid(
            "EVM session did not match certified semantic authority",
        ));
    }
    Ok(())
}

pub(crate) fn validate_block_anchor(anchor: &EvmBlockAnchor) -> Result<(), EvmStateError> {
    anchor
        .to_block()
        .map(|_| ())
        .map_err(|_| invalid("EVM block anchor was invalid"))
}

pub(crate) fn block_anchor_number(anchor: &EvmBlockAnchor) -> Result<U256, EvmStateError> {
    anchor
        .number_quantity()
        .map_err(|_| invalid("EVM block anchor number was invalid"))
}

pub(crate) fn block_anchor_hash(anchor: &EvmBlockAnchor) -> Result<B256, EvmStateError> {
    anchor
        .hash_value()
        .map_err(|_| invalid("EVM block anchor hash was invalid"))
}

pub(crate) fn invalid(reason: impl Into<String>) -> EvmStateError {
    EvmStateError::InvalidInput {
        reason: reason.into(),
    }
}
