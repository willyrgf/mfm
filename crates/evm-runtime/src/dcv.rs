use alloy_primitives::keccak256;
use serde::Deserialize;

use mfm_evm_core::abi as common_abi;
use mfm_evm_core::encoding;
use mfm_evm_core::hex as common_hex;

pub use common_abi::{AbiEvent, AbiFunction, ParsedAbi};

#[derive(Clone, Debug, Deserialize)]
pub struct ContractArtifactConfig {
    pub abi: serde_json::Value,
    pub bytecode: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ConfigureCallConfig {
    pub function: String,

    #[serde(default)]
    pub args: Vec<serde_json::Value>,

    #[serde(default)]
    pub value_wei: Option<String>,
}

fn default_min_count() -> u64 {
    1
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum BlockTag {
    Number(u64),
    Tag(String),
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReadAssertionConfig {
    pub function: String,

    #[serde(default)]
    pub args: Vec<serde_json::Value>,

    pub expected: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EventAssertionConfig {
    pub event: String,

    #[serde(default = "default_min_count")]
    pub min_count: u64,

    #[serde(default)]
    pub from_block: Option<BlockTag>,

    #[serde(default)]
    pub to_block: Option<BlockTag>,
}

#[derive(Clone, Debug)]
pub struct PreparedReadAssertion {
    pub data_hex: String,
    pub expected: serde_json::Value,
    pub outputs: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PreparedEventAssertion {
    pub event: String,
    pub topic0_hex: String,
    pub min_count: u64,
    pub from_block: serde_json::Value,
    pub to_block: serde_json::Value,
}

pub fn parse_abi(abi: &serde_json::Value) -> Result<ParsedAbi, String> {
    common_abi::parse_abi(abi).map_err(|e| e.message)
}

pub fn normalize_hex_str(s: &str) -> Result<String, String> {
    common_hex::normalize_hex_str(s).map_err(|e| e.message)
}

pub fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    common_hex::hex_to_bytes(s).map_err(|e| e.message)
}

pub fn bytes_to_hex_prefixed(bytes: &[u8]) -> String {
    common_hex::bytes_to_hex_prefixed(bytes)
}

pub fn resolve_function_call(
    abi: &ParsedAbi,
    function_name: &str,
    args: &[serde_json::Value],
) -> Result<(Vec<u8>, Vec<String>), String> {
    common_abi::resolve_function_call(abi, function_name, args).map_err(|e| e.message)
}

pub fn constructor_data(
    abi: &ParsedAbi,
    bytecode: &[u8],
    constructor_args: &[serde_json::Value],
) -> Result<Vec<u8>, String> {
    common_abi::constructor_data(abi, bytecode, constructor_args).map_err(|e| e.message)
}

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

pub fn parse_value_wei_to_hex(value_wei: &Option<String>) -> Result<Option<String>, String> {
    common_abi::parse_value_wei_to_hex(value_wei).map_err(|e| e.message)
}

pub fn ensure_nonzero_polls(max_receipt_polls: u64) -> Result<(), String> {
    if max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
}

pub fn ensure_nonempty_artifact_port(artifact_port: &str) -> Result<(), String> {
    if artifact_port.trim().is_empty() {
        return Err("artifact_port must be non-empty".to_string());
    }
    Ok(())
}

pub fn normalize_address(s: &str) -> Result<String, String> {
    encoding::normalize_address(s).map_err(|e| e.message)
}

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
