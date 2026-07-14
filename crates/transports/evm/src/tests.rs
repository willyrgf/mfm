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
