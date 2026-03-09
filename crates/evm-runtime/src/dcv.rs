//! Deploy/configure/validate helpers shared by reusable EVM runtime states.
//!
//! This module owns the pure data-shaping layer behind the EVM write-path states:
//!
//! - contract artifact parsing
//! - ABI-based calldata encoding
//! - validation assertion preparation
//! - small normalization helpers for addresses, blocks, and values
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm_runtime::dcv::{
//!     prepare_validate_assertions, BlockTag, EventAssertionConfig, ReadAssertionConfig,
//! };
//!
//! let abi = mfm_evm_runtime::dcv::parse_abi(&serde_json::json!([
//!     {
//!         "type": "function",
//!         "name": "owner",
//!         "inputs": [],
//!         "outputs": [{ "name": "", "type": "address" }],
//!         "stateMutability": "view"
//!     },
//!     {
//!         "type": "event",
//!         "name": "Configured",
//!         "inputs": [],
//!         "anonymous": false
//!     }
//! ]))
//! .unwrap();
//!
//! let (reads, events) = prepare_validate_assertions(
//!     &abi,
//!     &[ReadAssertionConfig {
//!         function: "owner".to_string(),
//!         args: vec![],
//!         expected: serde_json::json!("0x0000000000000000000000000000000000000000"),
//!     }],
//!     &[EventAssertionConfig {
//!         event: "Configured".to_string(),
//!         min_count: 1,
//!         from_block: Some(BlockTag::Number(0)),
//!         to_block: Some(BlockTag::Tag("latest".to_string())),
//!     }],
//! )
//! .unwrap();
//!
//! assert_eq!(reads.len(), 1);
//! assert_eq!(events[0].event, "Configured");
//! assert!(events[0].topic0_hex.starts_with("0x"));
//! ```

use alloy_primitives::keccak256;
use serde::Deserialize;

use mfm_evm_core::abi as common_abi;
use mfm_evm_core::encoding;
use mfm_evm_core::hex as common_hex;

pub use common_abi::{AbiEvent, AbiFunction, ParsedAbi};

/// JSON contract artifact used by deploy/configure/validate flows.
#[derive(Clone, Debug, Deserialize)]
pub struct ContractArtifactConfig {
    /// Contract ABI JSON payload.
    pub abi: serde_json::Value,
    /// Contract bytecode JSON payload.
    pub bytecode: serde_json::Value,
}

/// Runtime configuration for a single on-chain function call.
#[derive(Clone, Debug, Deserialize)]
pub struct ConfigureCallConfig {
    /// Function name to invoke.
    pub function: String,

    #[serde(default)]
    /// Positional arguments passed to the function call.
    pub args: Vec<serde_json::Value>,

    #[serde(default)]
    /// Optional call value expressed in wei.
    pub value_wei: Option<String>,
}

fn default_min_count() -> u64 {
    1
}

/// Block selector used by validation reads and event queries.
///
/// Numeric blocks are rendered as hex quantities; string tags are passed through as-is.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum BlockTag {
    /// Explicit block number.
    Number(u64),
    /// Symbolic tag such as `latest` or `earliest`.
    Tag(String),
}

/// Read assertion to evaluate through `eth_call`.
#[derive(Clone, Debug, Deserialize)]
pub struct ReadAssertionConfig {
    /// Function name to call.
    pub function: String,

    #[serde(default)]
    /// Positional arguments passed to the function call.
    pub args: Vec<serde_json::Value>,

    /// Expected decoded value.
    pub expected: serde_json::Value,
}

/// Event assertion to evaluate with `eth_getLogs`.
#[derive(Clone, Debug, Deserialize)]
pub struct EventAssertionConfig {
    /// Event name expected in the ABI.
    pub event: String,

    #[serde(default = "default_min_count")]
    /// Minimum matching log count required for success.
    pub min_count: u64,

    #[serde(default)]
    /// Optional lower bound for the log query.
    pub from_block: Option<BlockTag>,

    #[serde(default)]
    /// Optional upper bound for the log query.
    pub to_block: Option<BlockTag>,
}

/// Prepared read assertion ready for runtime execution.
#[derive(Clone, Debug)]
pub struct PreparedReadAssertion {
    /// Encoded calldata for the asserted function call.
    pub data_hex: String,
    /// Expected decoded value.
    pub expected: serde_json::Value,
    /// ABI output types used during decoding.
    pub outputs: Vec<String>,
}

/// Prepared event assertion ready for runtime execution.
#[derive(Clone, Debug)]
pub struct PreparedEventAssertion {
    /// Event name referenced by the assertion.
    pub event: String,
    /// Keccak event signature topic.
    pub topic0_hex: String,
    /// Minimum matching log count required for success.
    pub min_count: u64,
    /// Lower bound for the log query.
    pub from_block: serde_json::Value,
    /// Upper bound for the log query.
    pub to_block: serde_json::Value,
}

/// Parses an ABI JSON value into the shared ABI representation.
pub fn parse_abi(abi: &serde_json::Value) -> Result<ParsedAbi, String> {
    common_abi::parse_abi(abi).map_err(|e| e.message)
}

/// Normalizes a hex string to lowercase `0x`-prefixed form.
pub fn normalize_hex_str(s: &str) -> Result<String, String> {
    common_hex::normalize_hex_str(s).map_err(|e| e.message)
}

/// Decodes a hex string into bytes.
pub fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    common_hex::hex_to_bytes(s).map_err(|e| e.message)
}

/// Encodes bytes as a lowercase `0x`-prefixed hex string.
pub fn bytes_to_hex_prefixed(bytes: &[u8]) -> String {
    common_hex::bytes_to_hex_prefixed(bytes)
}

/// Resolves and encodes a function call from ABI name plus JSON arguments.
pub fn resolve_function_call(
    abi: &ParsedAbi,
    function_name: &str,
    args: &[serde_json::Value],
) -> Result<(Vec<u8>, Vec<String>), String> {
    common_abi::resolve_function_call(abi, function_name, args).map_err(|e| e.message)
}

/// Encodes constructor bytecode plus constructor arguments.
pub fn constructor_data(
    abi: &ParsedAbi,
    bytecode: &[u8],
    constructor_args: &[serde_json::Value],
) -> Result<Vec<u8>, String> {
    common_abi::constructor_data(abi, bytecode, constructor_args).map_err(|e| e.message)
}

/// Parses a contract artifact into validated ABI and bytecode components.
pub fn parse_artifact(cfg: &ContractArtifactConfig) -> Result<(ParsedAbi, Vec<u8>), String> {
    let abi = parse_abi(&cfg.abi)?;
    let bytecode = common_abi::parse_bytecode(&cfg.bytecode).map_err(|e| e.message)?;
    Ok((abi, bytecode))
}

fn block_tag_to_rpc_value(block: &Option<BlockTag>, default_latest: bool) -> serde_json::Value {
    match block {
        Some(BlockTag::Number(n)) => serde_json::json!(format!("0x{:x}", n)),
        Some(BlockTag::Tag(s)) => serde_json::json!(s),
        None if default_latest => serde_json::json!("latest"),
        None => serde_json::json!("earliest"),
    }
}

/// Prepares read and event assertions for runtime validation.
pub fn prepare_validate_assertions(
    abi: &ParsedAbi,
    read_assertions: &[ReadAssertionConfig],
    event_assertions: &[EventAssertionConfig],
) -> Result<(Vec<PreparedReadAssertion>, Vec<PreparedEventAssertion>), String> {
    let mut reads = Vec::with_capacity(read_assertions.len());
    for ra in read_assertions {
        let (call_data, outputs) = resolve_function_call(abi, &ra.function, &ra.args)
            .map_err(|_| "read assertion did not match ABI".to_string())?;
        reads.push(PreparedReadAssertion {
            data_hex: bytes_to_hex_prefixed(&call_data),
            expected: ra.expected.clone(),
            outputs,
        });
    }

    let mut events = Vec::with_capacity(event_assertions.len());
    for ea in event_assertions {
        let ev = abi
            .events
            .iter()
            .find(|e| e.name == ea.event)
            .ok_or_else(|| "event assertion referenced unknown event".to_string())?;
        if ev.anonymous {
            return Err("anonymous events are not supported for validation".to_string());
        }
        let sig = format!("{}({})", ev.name, ev.inputs.join(","));
        let topic0 = keccak256(sig.as_bytes());
        let topic0_hex = bytes_to_hex_prefixed(topic0.as_slice());
        let from_block = block_tag_to_rpc_value(&ea.from_block, false);
        let to_block = block_tag_to_rpc_value(&ea.to_block, true);
        events.push(PreparedEventAssertion {
            event: ea.event.clone(),
            topic0_hex,
            min_count: ea.min_count,
            from_block,
            to_block,
        });
    }

    Ok((reads, events))
}

/// Normalizes an optional wei value into RPC hex quantity form.
pub fn parse_value_wei_to_hex(value_wei: &Option<String>) -> Result<Option<String>, String> {
    common_abi::parse_value_wei_to_hex(value_wei).map_err(|e| e.message)
}

/// Ensures the receipt-poll budget is non-zero.
pub fn ensure_nonzero_polls(max_receipt_polls: u64) -> Result<(), String> {
    if max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
}

/// Ensures the configured artifact context port is non-empty.
pub fn ensure_nonempty_artifact_port(artifact_port: &str) -> Result<(), String> {
    if artifact_port.trim().is_empty() {
        return Err("artifact_port must be non-empty".to_string());
    }
    Ok(())
}

/// Normalizes an EVM address to canonical lowercase `0x`-prefixed form.
pub fn normalize_address(s: &str) -> Result<String, String> {
    encoding::normalize_address(s).map_err(|e| e.message)
}

/// Decodes a single-output `eth_call` response into JSON.
pub fn decode_single_output_to_json(
    outputs: &[String],
    raw_hex: &str,
) -> Result<serde_json::Value, String> {
    if outputs.len() != 1 {
        return Err("only single-output assertions are supported".to_string());
    }
    let decoded =
        common_abi::decode_single_output_to_json(outputs, raw_hex).map_err(|e| e.message)?;
    if outputs[0] != "bool"
        && outputs[0] != "address"
        && outputs[0] != "uint256"
        && outputs[0] != "uint"
    {
        return Err(format!(
            "unsupported output type for assertion: {}",
            outputs[0]
        ));
    }
    Ok(decoded)
}

/// Returns whether `actual` satisfies the configured expected value.
pub fn expected_matches(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    if expected == actual {
        return true;
    }

    match (actual, expected) {
        (serde_json::Value::String(a), serde_json::Value::String(e)) => {
            if a.starts_with("0x") && e.starts_with("0x") {
                normalize_hex_str(a).ok() == normalize_hex_str(e).ok()
            } else {
                false
            }
        }
        _ => false,
    }
}
