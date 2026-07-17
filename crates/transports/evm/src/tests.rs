use super::*;
use alloy_primitives::{address, B256, U256};
use mfm_evm_capabilities::{EvmTransactionSubmitRequest, SignedEvmPayload};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const HASH_HEX: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const OCCUPYING_HASH_HEX: &str =
    "0x2222222222222222222222222222222222222222222222222222222222222222";

#[path = "tests/behavior.rs"]
mod behavior;
#[path = "tests/rpc_server.rs"]
mod rpc_server;
use self::rpc_server::*;

#[test]
fn private_rpc_decoders_reject_noncanonical_quantities_and_data() {
    assert_eq!(parse_u256("0x0").expect("zero"), U256::ZERO);
    assert_eq!(parse_u256("0x2a").expect("quantity"), U256::from(42));
    for invalid in ["", "0X2a", "2a", "0x", "0x00", "0x01", "0xgg"] {
        assert_eq!(parse_u256(invalid), Err(EvmTransportError::InvalidResponse));
    }

    assert_eq!(
        decode_hex_bytes("0x").expect("empty bytes"),
        Vec::<u8>::new()
    );
    assert_eq!(decode_hex_bytes("0x00ff").expect("bytes"), vec![0, 255]);
    for invalid in ["", "0X00", "00", "0x0", "0xzz"] {
        assert_eq!(
            decode_hex_bytes(invalid),
            Err(EvmTransportError::InvalidResponse)
        );
    }
}
