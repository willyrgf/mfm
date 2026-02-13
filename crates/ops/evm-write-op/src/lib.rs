//! EVM write/validation operations (Milestone 2).
//!
//! This crate provides four operations that can be composed as a pipeline:
//! - `evm_deploy`   : deploy a contract from an artifact
//! - `evm_configure`: execute post-deploy contract calls
//! - `evm_validate` : enforce chain/client/read/event assertions
//! - `evm_contract_from_nix`: adapt `nix_app` output into an EVM artifact export
//!
//! Notes:
//! - All network interaction flows through `namespace = "evm"` IO.
//! - No secrets are persisted in op_config or outputs.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use k256::ecdsa::SigningKey;
use serde::Deserialize;

use alloy_primitives::keccak256;
use mfm_collectors_evm::{EvmIoClient, JsonRpcCall};
use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, IoError, StateError};
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::StateMeta;
use mfm_machine::plan::{StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_op_common::ctx as op_ctx;
use mfm_op_common::errors as op_errors;
use mfm_op_common::idempotency as op_idempotency;
use mfm_op_common::rpc as op_rpc;
use mfm_op_common::states::meta;

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

const OP_ID_CONTRACT_FROM_NIX: &str = "evm_contract_from_nix";
const OP_ID_DEPLOY: &str = "evm_deploy";
const OP_ID_CONFIGURE: &str = "evm_configure";
const OP_ID_VALIDATE: &str = "evm_validate";
const OP_VERSION: &str = "v1";

const KEY_NIX_RESULT: &str = "result";
const KEY_CONTRACT_ARTIFACT: &str = "contract_artifact";
const KEY_CONTRACT_ADDRESS: &str = "contract_address";
const KEY_DEPLOY_TX_HASH: &str = "deploy_tx_hash";
const KEY_DEPLOY_RECEIPT: &str = "deploy_receipt";

const KEY_CONFIGURE_TX_HASHES: &str = "configure_tx_hashes";
const KEY_CONFIGURE_RECEIPTS: &str = "configure_receipts";

const KEY_VALIDATED: &str = "validated";
const KEY_CHAIN_ID: &str = "chain_id";
const KEY_CLIENT_VERSION: &str = "client_version";

fn sdk_err(code: &'static str, message: &'static str) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn state_err(code: &'static str, message: &'static str) -> StateError {
    op_errors::state_unknown(code, message)
}

fn state_err_from_io(err: IoError) -> StateError {
    op_errors::state_from_io(err)
}

fn default_poll_interval_ms() -> u64 {
    500
}

fn default_max_receipt_polls() -> u64 {
    120
}

fn default_require_client_substring() -> String {
    "reth".to_string()
}

fn default_min_count() -> u64 {
    1
}

fn default_result_pointer() -> String {
    "/artifact".to_string()
}

fn default_artifact_port() -> String {
    KEY_CONTRACT_ARTIFACT.to_string()
}

#[derive(Clone, Debug, Deserialize)]
struct ContractArtifactConfig {
    abi: serde_json::Value,
    bytecode: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmContractFromNixConfig {
    #[serde(default = "default_result_pointer")]
    result_pointer: String,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmDeployConfig {
    #[serde(default)]
    artifact: Option<ContractArtifactConfig>,

    #[serde(default = "default_artifact_port")]
    artifact_port: String,

    from: String,

    #[serde(default)]
    constructor_args: Vec<serde_json::Value>,

    #[serde(default)]
    value_wei: Option<String>,

    #[serde(default)]
    signing_key_env: Option<String>,

    #[serde(default = "default_poll_interval_ms")]
    poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    max_receipt_polls: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct ConfigureCallConfig {
    function: String,

    #[serde(default)]
    args: Vec<serde_json::Value>,

    #[serde(default)]
    value_wei: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmConfigureConfig {
    #[serde(default)]
    artifact: Option<ContractArtifactConfig>,

    #[serde(default = "default_artifact_port")]
    artifact_port: String,

    from: String,

    #[serde(default)]
    contract_address: Option<String>,

    calls: Vec<ConfigureCallConfig>,

    #[serde(default = "default_poll_interval_ms")]
    poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    max_receipt_polls: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct ReadAssertionConfig {
    function: String,

    #[serde(default)]
    args: Vec<serde_json::Value>,

    expected: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum BlockTag {
    Number(u64),
    Tag(String),
}

#[derive(Clone, Debug, Deserialize)]
struct EventAssertionConfig {
    event: String,

    #[serde(default = "default_min_count")]
    min_count: u64,

    #[serde(default)]
    from_block: Option<BlockTag>,

    #[serde(default)]
    to_block: Option<BlockTag>,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmValidateConfig {
    #[serde(default)]
    artifact: Option<ContractArtifactConfig>,

    #[serde(default = "default_artifact_port")]
    artifact_port: String,

    #[serde(default)]
    contract_address: Option<String>,

    expected_chain_id: u64,

    #[serde(default = "default_require_client_substring")]
    require_client_substring: String,

    #[serde(default)]
    read_assertions: Vec<ReadAssertionConfig>,

    #[serde(default)]
    event_assertions: Vec<EventAssertionConfig>,
}

#[derive(Clone, Debug)]
struct AbiFunction {
    name: String,
    inputs: Vec<String>,
    outputs: Vec<String>,
}

#[derive(Clone, Debug)]
struct AbiEvent {
    name: String,
    inputs: Vec<String>,
    anonymous: bool,
}

#[derive(Clone, Debug, Default)]
struct ParsedAbi {
    constructor_inputs: Vec<String>,
    functions: Vec<AbiFunction>,
    events: Vec<AbiEvent>,
}

#[derive(Clone, Debug)]
struct DeployRuntimeConfig {
    artifact: Option<ContractArtifactConfig>,
    artifact_port: String,
    from: String,
    constructor_args: Vec<serde_json::Value>,
    value_hex: Option<String>,
    signing_key_env: Option<String>,
    poll_interval_ms: u64,
    max_receipt_polls: u64,
}

#[derive(Clone, Debug)]
struct ConfigureRuntimeCall {
    function: String,
    args: Vec<serde_json::Value>,
    value_hex: Option<String>,
}

#[derive(Clone, Debug)]
struct ConfigureRuntimeConfig {
    artifact: Option<ContractArtifactConfig>,
    artifact_port: String,
    from: String,
    contract_address: Option<String>,
    calls: Vec<ConfigureRuntimeCall>,
    poll_interval_ms: u64,
    max_receipt_polls: u64,
}

#[derive(Clone, Debug)]
struct PreparedReadAssertion {
    data_hex: String,
    expected: serde_json::Value,
    outputs: Vec<String>,
}

#[derive(Clone, Debug)]
struct PreparedEventAssertion {
    event: String,
    topic0_hex: String,
    min_count: u64,
    from_block: serde_json::Value,
    to_block: serde_json::Value,
}

#[derive(Clone, Debug)]
struct ValidateRuntimeConfig {
    artifact: Option<ContractArtifactConfig>,
    artifact_port: String,
    contract_address: Option<String>,
    expected_chain_id: u64,
    require_client_substring: String,
    read_assertions: Vec<ReadAssertionConfig>,
    event_assertions: Vec<EventAssertionConfig>,
}

#[derive(Clone, Debug, Deserialize)]
struct AbiParamJson {
    #[serde(rename = "type")]
    typ: String,
}

#[derive(Clone, Debug, Deserialize)]
struct AbiItemJson {
    #[serde(rename = "type")]
    kind: String,

    #[serde(default)]
    name: Option<String>,

    #[serde(default)]
    inputs: Vec<AbiParamJson>,

    #[serde(default)]
    outputs: Vec<AbiParamJson>,

    #[serde(default)]
    anonymous: Option<bool>,
}

fn parse_abi(abi: &serde_json::Value) -> Result<ParsedAbi, String> {
    let items: Vec<AbiItemJson> = serde_json::from_value(abi.clone())
        .map_err(|_| "artifact.abi must be a valid JSON ABI array".to_string())?;

    let mut out = ParsedAbi::default();

    for it in items {
        match it.kind.as_str() {
            "constructor" => {
                out.constructor_inputs = it.inputs.into_iter().map(|p| p.typ).collect();
            }
            "function" => {
                let Some(name) = it.name else {
                    return Err("function ABI item missing name".to_string());
                };
                out.functions.push(AbiFunction {
                    name,
                    inputs: it.inputs.into_iter().map(|p| p.typ).collect(),
                    outputs: it.outputs.into_iter().map(|p| p.typ).collect(),
                });
            }
            "event" => {
                let Some(name) = it.name else {
                    return Err("event ABI item missing name".to_string());
                };
                out.events.push(AbiEvent {
                    name,
                    inputs: it.inputs.into_iter().map(|p| p.typ).collect(),
                    anonymous: it.anonymous.unwrap_or(false),
                });
            }
            _ => {}
        }
    }

    Ok(out)
}

fn normalize_hex_str(s: &str) -> Result<String, String> {
    let Some(rest) = s.strip_prefix("0x") else {
        return Err("hex string must start with 0x".to_string());
    };
    if rest.is_empty() {
        return Ok("0x".to_string());
    }
    if !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("hex string contains non-hex characters".to_string());
    }
    let even = if rest.len() % 2 == 0 {
        rest.to_ascii_lowercase()
    } else {
        format!("0{}", rest.to_ascii_lowercase())
    };
    Ok(format!("0x{even}"))
}

fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    let n = normalize_hex_str(s)?;
    let rest = n.strip_prefix("0x").expect("prefix");
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    hex::decode(rest).map_err(|_| "invalid hex".to_string())
}

fn bytes_to_hex_prefixed(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn parse_bytecode(v: &serde_json::Value) -> Result<Vec<u8>, String> {
    match v {
        serde_json::Value::String(s) => hex_to_bytes(s),
        serde_json::Value::Object(m) => {
            let Some(obj) = m.get("object") else {
                return Err("artifact.bytecode object missing field `object`".to_string());
            };
            let Some(s) = obj.as_str() else {
                return Err("artifact.bytecode.object must be a string".to_string());
            };
            hex_to_bytes(s)
        }
        _ => Err("artifact.bytecode must be a string or object with `object`".to_string()),
    }
}

fn parse_address_hex(s: &str) -> Result<[u8; 20], String> {
    let b = hex_to_bytes(s)?;
    if b.len() != 20 {
        return Err("address must be 20 bytes".to_string());
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&b);
    Ok(out)
}

fn normalize_address(s: &str) -> Result<String, String> {
    let b = parse_address_hex(s)?;
    Ok(bytes_to_hex_prefixed(&b))
}

fn encode_u64_word(n: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&n.to_be_bytes());
    out
}

fn encode_len_word(n: usize) -> Result<[u8; 32], String> {
    let n64 = u64::try_from(n).map_err(|_| "length overflow".to_string())?;
    Ok(encode_u64_word(n64))
}

fn parse_uint_word(v: &serde_json::Value) -> Result<[u8; 32], String> {
    let mut out = [0u8; 32];

    match v {
        serde_json::Value::Number(n) => {
            let Some(u) = n.as_u64() else {
                return Err("numeric value must fit into u64".to_string());
            };
            out[24..32].copy_from_slice(&u.to_be_bytes());
            Ok(out)
        }
        serde_json::Value::String(s) => {
            if s.starts_with("0x") {
                let b = hex_to_bytes(s)?;
                if b.len() > 32 {
                    return Err("hex integer must fit into 32 bytes".to_string());
                }
                out[32 - b.len()..].copy_from_slice(&b);
                return Ok(out);
            }

            let parsed = s
                .parse::<u128>()
                .map_err(|_| "decimal integer must fit into u128".to_string())?;
            out[16..32].copy_from_slice(&parsed.to_be_bytes());
            Ok(out)
        }
        _ => Err("integer arg must be a number or string".to_string()),
    }
}

fn parse_bool_word(v: &serde_json::Value) -> Result<[u8; 32], String> {
    let b = match v {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => match n.as_u64() {
            Some(0) => false,
            Some(1) => true,
            _ => return Err("bool numeric arg must be 0 or 1".to_string()),
        },
        serde_json::Value::String(s) => match s.as_str() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => return Err("bool arg must be true/false/0/1".to_string()),
        },
        _ => return Err("bool arg must be bool/number/string".to_string()),
    };

    let mut out = [0u8; 32];
    out[31] = if b { 1 } else { 0 };
    Ok(out)
}

fn parse_bytes_m_word(m: usize, v: &serde_json::Value) -> Result<[u8; 32], String> {
    let Some(s) = v.as_str() else {
        return Err("bytesM arg must be a hex string".to_string());
    };
    let b = hex_to_bytes(s)?;
    if b.len() != m {
        return Err("bytesM arg length mismatch".to_string());
    }
    let mut out = [0u8; 32];
    out[..m].copy_from_slice(&b);
    Ok(out)
}

fn is_dynamic_type(t: &str) -> bool {
    t == "bytes" || t == "string"
}

fn pad_to_32(mut b: Vec<u8>) -> Vec<u8> {
    let r = b.len() % 32;
    if r != 0 {
        b.extend(std::iter::repeat_n(0u8, 32 - r));
    }
    b
}

fn encode_dynamic_value(t: &str, v: &serde_json::Value) -> Result<Vec<u8>, String> {
    let data = match t {
        "string" => {
            let Some(s) = v.as_str() else {
                return Err("string arg must be a string".to_string());
            };
            s.as_bytes().to_vec()
        }
        "bytes" => {
            let Some(s) = v.as_str() else {
                return Err("bytes arg must be a hex string".to_string());
            };
            hex_to_bytes(s)?
        }
        _ => return Err("unsupported dynamic type".to_string()),
    };

    let mut out = Vec::new();
    out.extend_from_slice(&encode_len_word(data.len())?);
    out.extend_from_slice(&pad_to_32(data));
    Ok(out)
}

fn encode_static_value(t: &str, v: &serde_json::Value) -> Result<[u8; 32], String> {
    if t == "address" {
        let Some(s) = v.as_str() else {
            return Err("address arg must be a string".to_string());
        };
        let addr = parse_address_hex(s)?;
        let mut out = [0u8; 32];
        out[12..32].copy_from_slice(&addr);
        return Ok(out);
    }

    if t == "bool" {
        return parse_bool_word(v);
    }

    if t.starts_with("uint") || t.starts_with("int") {
        return parse_uint_word(v);
    }

    if t == "bytes32" {
        return parse_bytes_m_word(32, v);
    }

    if let Some(size_str) = t.strip_prefix("bytes") {
        let m = size_str
            .parse::<usize>()
            .map_err(|_| "invalid bytesM type".to_string())?;
        if m == 0 || m > 32 {
            return Err("bytesM size must be in [1,32]".to_string());
        }
        return parse_bytes_m_word(m, v);
    }

    Err("unsupported static ABI type".to_string())
}

fn encode_params(types: &[String], args: &[serde_json::Value]) -> Result<Vec<u8>, String> {
    if types.len() != args.len() {
        return Err("argument count mismatch".to_string());
    }

    let head_size = types
        .len()
        .checked_mul(32)
        .ok_or_else(|| "head size overflow".to_string())?;

    let mut head = Vec::<[u8; 32]>::with_capacity(types.len());
    let mut tail = Vec::<u8>::new();

    for (t, a) in types.iter().zip(args.iter()) {
        if is_dynamic_type(t) {
            let off = head_size
                .checked_add(tail.len())
                .ok_or_else(|| "offset overflow".to_string())?;
            head.push(encode_len_word(off)?);
            let dyn_enc = encode_dynamic_value(t, a)?;
            tail.extend_from_slice(&dyn_enc);
        } else {
            head.push(encode_static_value(t, a)?);
        }
    }

    let mut out = Vec::with_capacity(head_size + tail.len());
    for w in head {
        out.extend_from_slice(&w);
    }
    out.extend_from_slice(&tail);
    Ok(out)
}

fn selector(name: &str, input_types: &[String]) -> [u8; 4] {
    let sig = format!("{}({})", name, input_types.join(","));
    let h = keccak256(sig.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

fn resolve_function_call(
    abi: &ParsedAbi,
    function: &str,
    args: &[serde_json::Value],
) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut winners: Vec<(Vec<u8>, Vec<String>)> = Vec::new();

    for f in abi
        .functions
        .iter()
        .filter(|f| f.name == function && f.inputs.len() == args.len())
    {
        let Ok(mut enc) = encode_params(&f.inputs, args) else {
            continue;
        };
        let mut out = Vec::with_capacity(4 + enc.len());
        out.extend_from_slice(&selector(&f.name, &f.inputs));
        out.append(&mut enc);
        winners.push((out, f.outputs.clone()));
    }

    match winners.len() {
        0 => Err("function resolution failed for provided args".to_string()),
        1 => Ok(winners.remove(0)),
        _ => Err("ambiguous overloaded function for provided args".to_string()),
    }
}

fn constructor_data(
    abi: &ParsedAbi,
    bytecode: &[u8],
    args: &[serde_json::Value],
) -> Result<Vec<u8>, String> {
    let enc = encode_params(&abi.constructor_inputs, args)?;
    let mut out = Vec::with_capacity(bytecode.len() + enc.len());
    out.extend_from_slice(bytecode);
    out.extend_from_slice(&enc);
    Ok(out)
}

fn parse_artifact(cfg: &ContractArtifactConfig) -> Result<(ParsedAbi, Vec<u8>), String> {
    let abi = parse_abi(&cfg.abi)?;
    let bytecode = parse_bytecode(&cfg.bytecode)?;
    Ok((abi, bytecode))
}

fn prepare_validate_assertions(
    abi: &ParsedAbi,
    read_assertions: &[ReadAssertionConfig],
    event_assertions: &[EventAssertionConfig],
) -> Result<(Vec<PreparedReadAssertion>, Vec<PreparedEventAssertion>), String> {
    let mut prepared_reads = Vec::with_capacity(read_assertions.len());
    for ra in read_assertions {
        let (data, outputs) = resolve_function_call(abi, &ra.function, &ra.args)
            .map_err(|_| "read assertion did not match ABI".to_string())?;
        prepared_reads.push(PreparedReadAssertion {
            data_hex: bytes_to_hex_prefixed(&data),
            expected: ra.expected.clone(),
            outputs,
        });
    }

    let mut prepared_events = Vec::with_capacity(event_assertions.len());
    for ea in event_assertions {
        let Some(event) = abi.events.iter().find(|e| e.name == ea.event) else {
            return Err("event assertion referenced unknown event".to_string());
        };

        if event.anonymous {
            return Err("anonymous events are not supported for validation".to_string());
        }

        let sig = format!("{}({})", event.name, event.inputs.join(","));
        let topic0 = keccak256(sig.as_bytes());

        let from_block = to_block_param(&ea.from_block, "earliest")
            .map_err(|_| "invalid from_block".to_string())?;
        let to_block =
            to_block_param(&ea.to_block, "latest").map_err(|_| "invalid to_block".to_string())?;

        prepared_events.push(PreparedEventAssertion {
            event: ea.event.clone(),
            topic0_hex: bytes_to_hex_prefixed(topic0.as_slice()),
            min_count: ea.min_count,
            from_block,
            to_block,
        });
    }

    Ok((prepared_reads, prepared_events))
}

fn parse_value_wei_to_hex(value_wei: &Option<String>) -> Result<Option<String>, String> {
    let Some(v) = value_wei else {
        return Ok(None);
    };
    if v.starts_with("0x") {
        return Ok(Some(normalize_hex_str(v)?));
    }
    let parsed = v
        .parse::<u128>()
        .map_err(|_| "value_wei must be decimal or 0x-prefixed hex".to_string())?;
    Ok(Some(format!("0x{:x}", parsed)))
}

fn ensure_nonzero_polls(max_receipt_polls: u64) -> Result<(), String> {
    if max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
}

fn ensure_nonempty_artifact_port(artifact_port: &str) -> Result<(), String> {
    if artifact_port.trim().is_empty() {
        return Err("artifact_port must be non-empty".to_string());
    }
    Ok(())
}

fn ensure_nonempty_env_name(env_name: &str) -> Result<(), String> {
    if env_name.trim().is_empty() {
        return Err("signing_key_env must be non-empty".to_string());
    }
    Ok(())
}

fn to_block_param(tag: &Option<BlockTag>, default_tag: &str) -> Result<serde_json::Value, String> {
    match tag {
        None => Ok(serde_json::json!(default_tag)),
        Some(BlockTag::Tag(t)) => Ok(serde_json::json!(t)),
        Some(BlockTag::Number(n)) => Ok(serde_json::json!(format!("0x{n:x}"))),
    }
}

fn decode_single_output_to_json(
    outputs: &[String],
    raw_hex: &str,
) -> Result<serde_json::Value, String> {
    let normalized = normalize_hex_str(raw_hex)?;

    if outputs.is_empty() {
        return Ok(serde_json::json!(normalized));
    }

    if outputs.len() != 1 {
        return Ok(serde_json::json!(normalized));
    }

    let out_t = outputs[0].as_str();
    let b = hex_to_bytes(&normalized)?;
    if b.len() < 32 {
        return Err("eth_call return data too short".to_string());
    }

    match out_t {
        "bool" => Ok(serde_json::json!(b[31] == 1u8)),
        "address" => {
            let addr = &b[12..32];
            Ok(serde_json::json!(bytes_to_hex_prefixed(addr)))
        }
        t if t.starts_with("uint") || t.starts_with("int") => {
            let mut v: u64 = 0;
            for byte in &b[24..32] {
                v = (v << 8) | u64::from(*byte);
            }
            Ok(serde_json::json!(v))
        }
        _ => Ok(serde_json::json!(normalized)),
    }
}

fn expected_matches(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    if let (Some(a), Some(e)) = (actual.as_str(), expected.as_str()) {
        let an = normalize_hex_str(a).ok();
        let en = normalize_hex_str(e).ok();
        if let (Some(an), Some(en)) = (an, en) {
            return an == en;
        }
    }
    actual == expected
}

fn context_read_string(ctx: &dyn DynContext, key: &str) -> Result<String, StateError> {
    op_ctx::read_string_required(
        ctx,
        &ContextKey(key.to_string()),
        "ctx_missing_key",
        "required context key was missing",
        "ctx_type_mismatch",
        "context value was not a string",
    )
}

fn context_read_json(ctx: &dyn DynContext, key: &str) -> Result<serde_json::Value, StateError> {
    op_ctx::read_json_required(
        ctx,
        &ContextKey(key.to_string()),
        "ctx_missing_key",
        "required context key was missing",
    )
}

fn resolve_artifact_config(
    ctx: &dyn DynContext,
    configured: &Option<ContractArtifactConfig>,
    artifact_port: &str,
) -> Result<ContractArtifactConfig, StateError> {
    if let Some(artifact) = configured {
        return Ok(artifact.clone());
    }

    let v = context_read_json(ctx, artifact_port)?;
    serde_json::from_value::<ContractArtifactConfig>(v)
        .map_err(|_| state_err("ctx_type_mismatch", "context artifact value was invalid"))
}

fn context_write_json(
    ctx: &mut dyn DynContext,
    key: &str,
    value: serde_json::Value,
) -> Result<(), StateError> {
    op_ctx::write_json(ctx, ContextKey(key.to_string()), value)
}

async fn send_transaction(
    client: &mut EvmIoClient<'_>,
    mut tx_obj: serde_json::Value,
) -> Result<String, StateError> {
    if tx_obj.get("gas").is_none() {
        let gas = estimate_gas_hex(client, &tx_obj).await?;
        tx_obj["gas"] = serde_json::json!(gas);
    }

    if tx_obj.get("gasPrice").is_none() && tx_obj.get("maxFeePerGas").is_none() {
        let gas_price = gas_price_hex(client).await?;
        tx_obj["gasPrice"] = serde_json::json!(gas_price);
    }

    let res = client
        .call(JsonRpcCall::new(
            "eth_sendTransaction",
            serde_json::json!([tx_obj]),
        ))
        .await
        .map_err(state_err_from_io)?;

    let Some(tx_hash) = res.response.as_str() else {
        return Err(state_err(
            "evm_response_invalid",
            "eth_sendTransaction returned non-string tx hash",
        ));
    };

    normalize_hex_str(tx_hash).map_err(|_| {
        state_err(
            "evm_response_invalid",
            "eth_sendTransaction returned invalid hex tx hash",
        )
    })
}

async fn send_raw_transaction(
    client: &mut EvmIoClient<'_>,
    raw_tx_hex: &str,
) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new(
            "eth_sendRawTransaction",
            serde_json::json!([raw_tx_hex]),
        ))
        .await
        .map_err(state_err_from_io)?;

    let Some(tx_hash) = res.response.as_str() else {
        return Err(state_err(
            "evm_response_invalid",
            "eth_sendRawTransaction returned non-string tx hash",
        ));
    };

    normalize_hex_str(tx_hash).map_err(|_| {
        state_err(
            "evm_response_invalid",
            "eth_sendRawTransaction returned invalid hex tx hash",
        )
    })
}

async fn send_signed_create_transaction(
    client: &mut EvmIoClient<'_>,
    signing_key_env: &str,
    from: &str,
    constructor_payload: &[u8],
    value_hex: Option<&str>,
) -> Result<String, StateError> {
    let signing_key = signing_key_from_env(signing_key_env)?;
    let signer_addr = signer_address_hex(&signing_key);
    let configured_from = normalize_address(from)
        .map_err(|_| state_err("invalid_op_config", "invalid from address"))?;
    if signer_addr != configured_from {
        return Err(state_err(
            "signing_key_address_mismatch",
            "signing key did not match configured from address",
        ));
    }

    let tx_obj = {
        let mut tx = serde_json::json!({
            "from": configured_from,
            "data": bytes_to_hex_prefixed(constructor_payload),
        });
        if let Some(v) = value_hex {
            tx["value"] = serde_json::json!(v);
        }
        tx
    };

    let nonce_hex = transaction_count_hex(client, &configured_from).await?;
    let gas_hex = estimate_gas_hex(client, &tx_obj).await?;
    let gas_price_hex = gas_price_hex(client).await?;
    let chain_id = client.chain_id_u64().await.map_err(state_err_from_io)?;

    let raw_tx_hex = sign_legacy_create_raw_tx(
        &signing_key,
        chain_id,
        &nonce_hex,
        &gas_price_hex,
        &gas_hex,
        value_hex.unwrap_or("0x0"),
        constructor_payload,
    )?;

    send_raw_transaction(client, &raw_tx_hex).await
}

async fn transaction_count_hex(
    client: &mut EvmIoClient<'_>,
    from: &str,
) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new(
            "eth_getTransactionCount",
            serde_json::json!([from, "pending"]),
        ))
        .await
        .map_err(state_err_from_io)?;

    let Some(nonce) = res.response.as_str() else {
        return Err(state_err(
            "evm_response_invalid",
            "eth_getTransactionCount returned non-string nonce",
        ));
    };

    normalize_hex_str(nonce).map_err(|_| {
        state_err(
            "evm_response_invalid",
            "eth_getTransactionCount returned invalid hex nonce",
        )
    })
}

fn signing_key_from_env(signing_key_env: &str) -> Result<SigningKey, StateError> {
    let raw = std::env::var(signing_key_env).map_err(|_| {
        state_err(
            "missing_signing_key_env",
            "signing_key_env did not exist in process environment",
        )
    })?;

    let normalized = normalize_hex_str(&raw)
        .map_err(|_| state_err("invalid_signing_key_env", "signing key hex was invalid"))?;
    let bytes = hex_to_bytes(&normalized)
        .map_err(|_| state_err("invalid_signing_key_env", "signing key hex was invalid"))?;
    let key: [u8; 32] = bytes.try_into().map_err(|_| {
        state_err(
            "invalid_signing_key_env",
            "signing key must be exactly 32 bytes",
        )
    })?;

    SigningKey::from_bytes((&key).into()).map_err(|_| {
        state_err(
            "invalid_signing_key_env",
            "signing key did not form a valid secp256k1 key",
        )
    })
}

fn signer_address_hex(signing_key: &SigningKey) -> String {
    let public_key = signing_key.verifying_key().to_encoded_point(false);
    let hash = keccak256(&public_key.as_bytes()[1..]);
    bytes_to_hex_prefixed(&hash.as_slice()[12..])
}

fn sign_legacy_create_raw_tx(
    signing_key: &SigningKey,
    chain_id: u64,
    nonce_hex: &str,
    gas_price_hex: &str,
    gas_limit_hex: &str,
    value_hex: &str,
    data: &[u8],
) -> Result<String, StateError> {
    let nonce = hex_quantity_to_rlp_bytes(nonce_hex)?;
    let gas_price = hex_quantity_to_rlp_bytes(gas_price_hex)?;
    let gas_limit = hex_quantity_to_rlp_bytes(gas_limit_hex)?;
    let value = hex_quantity_to_rlp_bytes(value_hex)?;
    let chain_id_bytes = u128_to_min_be(u128::from(chain_id));

    let unsigned = rlp_encode_list(&[
        nonce.clone(),
        gas_price.clone(),
        gas_limit.clone(),
        Vec::new(),
        value.clone(),
        data.to_vec(),
        chain_id_bytes.clone(),
        Vec::new(),
        Vec::new(),
    ]);

    let sighash = keccak256(&unsigned);
    let (sig, recid) = signing_key
        .sign_prehash_recoverable(sighash.as_slice())
        .map_err(|_| state_err("signing_failed", "failed to sign deployment transaction"))?;

    let sig_bytes = sig.to_bytes();
    let r = trim_leading_zero_bytes(&sig_bytes[..32]);
    let s = trim_leading_zero_bytes(&sig_bytes[32..]);
    let v = u128::from(chain_id) * 2 + 35 + u128::from(u8::from(recid));
    let v_bytes = u128_to_min_be(v);

    let signed = rlp_encode_list(&[
        nonce,
        gas_price,
        gas_limit,
        Vec::new(),
        value,
        data.to_vec(),
        v_bytes,
        r,
        s,
    ]);

    Ok(bytes_to_hex_prefixed(&signed))
}

fn hex_quantity_to_rlp_bytes(value: &str) -> Result<Vec<u8>, StateError> {
    let normalized = normalize_hex_str(value)
        .map_err(|_| state_err("invalid_op_config", "invalid transaction quantity hex"))?;
    let bytes = hex_to_bytes(&normalized)
        .map_err(|_| state_err("invalid_op_config", "invalid transaction quantity hex"))?;
    Ok(trim_leading_zero_bytes(&bytes))
}

fn trim_leading_zero_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx] == 0 {
        idx += 1;
    }
    bytes[idx..].to_vec()
}

fn u128_to_min_be(mut value: u128) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }

    let mut out = Vec::new();
    if bytes.len() <= 55 {
        out.push(0x80 + bytes.len() as u8);
        out.extend_from_slice(bytes);
        return out;
    }

    let len_bytes = usize_to_min_be(bytes.len());
    out.push(0xb7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(bytes);
    out
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for item in items {
        payload.extend_from_slice(&rlp_encode_bytes(item));
    }

    let mut out = Vec::new();
    if payload.len() <= 55 {
        out.push(0xc0 + payload.len() as u8);
        out.extend_from_slice(&payload);
        return out;
    }

    let len_bytes = usize_to_min_be(payload.len());
    out.push(0xf7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(&payload);
    out
}

fn usize_to_min_be(mut value: usize) -> Vec<u8> {
    if value == 0 {
        return vec![0];
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

async fn estimate_gas_hex(
    client: &mut EvmIoClient<'_>,
    tx_obj: &serde_json::Value,
) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new(
            "eth_estimateGas",
            serde_json::json!([tx_obj]),
        ))
        .await
        .map_err(state_err_from_io)?;

    let Some(gas) = res.response.as_str() else {
        return Err(state_err(
            "evm_response_invalid",
            "eth_estimateGas returned non-string gas value",
        ));
    };

    normalize_hex_str(gas).map_err(|_| {
        state_err(
            "evm_response_invalid",
            "eth_estimateGas returned invalid hex gas value",
        )
    })
}

async fn gas_price_hex(client: &mut EvmIoClient<'_>) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new("eth_gasPrice", serde_json::json!([])))
        .await
        .map_err(state_err_from_io)?;

    let Some(gas_price) = res.response.as_str() else {
        return Err(state_err(
            "evm_response_invalid",
            "eth_gasPrice returned non-string gas price",
        ));
    };

    normalize_hex_str(gas_price).map_err(|_| {
        state_err(
            "evm_response_invalid",
            "eth_gasPrice returned invalid hex gas price",
        )
    })
}

async fn wait_for_receipt(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    tx_hash: &str,
    poll_interval_ms: u64,
    max_receipt_polls: u64,
) -> Result<serde_json::Value, StateError> {
    for poll_index in 0..max_receipt_polls {
        let request = serde_json::to_value(JsonRpcCall::new(
            "eth_getTransactionReceipt",
            serde_json::json!([tx_hash]),
        ))
        .expect("JsonRpcCall must serialize");
        let res = io
            .call(IoCall {
                namespace: "evm".to_string(),
                request,
                fact_key: Some(FactKey(format!(
                    "mfm:evm|state:{}|receipt_poll:{}|tx:{}",
                    state_id.0, poll_index, tx_hash
                ))),
            })
            .await
            .map_err(state_err_from_io)?;

        if !res.response.is_null() {
            return Ok(res.response);
        }

        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
    }

    Err(state_err(
        "evm_receipt_timeout",
        "timed out waiting for transaction receipt",
    ))
}

fn ensure_receipt_success(receipt: &serde_json::Value) -> Result<(), StateError> {
    let Some(obj) = receipt.as_object() else {
        return Err(state_err(
            "evm_response_invalid",
            "transaction receipt was not a JSON object",
        ));
    };

    if let Some(status) = obj.get("status").and_then(|v| v.as_str()) {
        let s = normalize_hex_str(status)
            .map_err(|_| state_err("evm_response_invalid", "receipt status was not valid hex"))?;
        if s != "0x01" && s != "0x1" {
            return Err(state_err(
                "evm_receipt_failed_status",
                "transaction receipt reported failed status",
            ));
        }
    }

    Ok(())
}

fn receipt_contract_address(receipt: &serde_json::Value) -> Result<String, StateError> {
    let Some(obj) = receipt.as_object() else {
        return Err(state_err(
            "evm_response_invalid",
            "transaction receipt was not a JSON object",
        ));
    };
    let Some(addr) = obj.get("contractAddress").and_then(|v| v.as_str()) else {
        return Err(state_err(
            "evm_receipt_missing_contract_address",
            "receipt did not include contractAddress",
        ));
    };

    normalize_address(addr).map_err(|_| {
        state_err(
            "evm_response_invalid",
            "receipt contractAddress was invalid",
        )
    })
}

fn resolve_contract_address(
    ctx: &dyn DynContext,
    configured: &Option<String>,
) -> Result<String, StateError> {
    match configured {
        Some(a) => normalize_address(a)
            .map_err(|_| state_err("invalid_op_config", "contract_address was invalid")),
        None => {
            let a = context_read_string(ctx, KEY_CONTRACT_ADDRESS)?;
            normalize_address(&a)
                .map_err(|_| state_err("ctx_type_mismatch", "context contract address invalid"))
        }
    }
}

#[derive(Clone, Default)]
pub struct EvmContractFromNixOp;

impl Operation for EvmContractFromNixOp {
    fn op_id(&self) -> OpId {
        OpId(OP_ID_CONTRACT_FROM_NIX.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmContractFromNixConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                sdk_err(
                    "invalid_op_config",
                    "invalid evm_contract_from_nix op_config",
                )
            })?;
        if !cfg.result_pointer.is_empty() && !cfg.result_pointer.starts_with('/') {
            return Err(sdk_err(
                "invalid_op_config",
                "result_pointer must be empty or start with '/'",
            ));
        }

        Ok(OpIo {
            imports: vec![PortKey(KEY_NIX_RESULT.to_string())],
            exports: vec![PortKey(KEY_CONTRACT_ARTIFACT.to_string())],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: EvmContractFromNixConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                sdk_err(
                    "invalid_op_config",
                    "invalid evm_contract_from_nix op_config",
                )
            })?;
        if !cfg.result_pointer.is_empty() && !cfg.result_pointer.starts_with('/') {
            return Err(sdk_err(
                "invalid_op_config",
                "result_pointer must be empty or start with '/'",
            ));
        }

        let state_id = StateId(format!("{}.adapt", op_path.0));
        let state = Arc::new(ContractFromNixState {
            result_pointer: cfg.result_pointer,
        });

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state,
            }],
            edges: Vec::new(),
        })
    }
}

struct ContractFromNixState {
    result_pointer: String,
}

#[async_trait]
impl State for ContractFromNixState {
    fn meta(&self) -> StateMeta {
        meta::config()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let result = context_read_json(ctx, KEY_NIX_RESULT)?;

        let artifact_value = if self.result_pointer.is_empty() {
            result
        } else {
            result
                .pointer(&self.result_pointer)
                .cloned()
                .ok_or_else(|| {
                    state_err(
                        "nix_result_pointer_missing",
                        "nix result did not contain the configured pointer",
                    )
                })?
        };

        let artifact = serde_json::from_value::<ContractArtifactConfig>(artifact_value.clone())
            .map_err(|_| {
                state_err(
                    "invalid_contract_artifact",
                    "nix result artifact was invalid",
                )
            })?;
        parse_artifact(&artifact).map_err(|_| {
            state_err(
                "invalid_contract_artifact",
                "nix result artifact was invalid",
            )
        })?;

        context_write_json(ctx, KEY_CONTRACT_ARTIFACT, artifact_value)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone, Default)]
pub struct EvmDeployOp;

impl Operation for EvmDeployOp {
    fn op_id(&self) -> OpId {
        OpId(OP_ID_DEPLOY.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmDeployConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid evm_deploy op_config"))?;
        ensure_nonempty_artifact_port(&cfg.artifact_port)
            .map_err(|_| sdk_err("invalid_op_config", "artifact_port must be non-empty"))?;
        if let Some(env_name) = cfg.signing_key_env.as_deref() {
            ensure_nonempty_env_name(env_name)
                .map_err(|_| sdk_err("invalid_op_config", "signing_key_env must be non-empty"))?;
        }

        let imports = if cfg.artifact.is_none() {
            vec![PortKey(cfg.artifact_port)]
        } else {
            Vec::new()
        };

        Ok(OpIo {
            imports,
            exports: vec![
                PortKey(KEY_CONTRACT_ADDRESS.to_string()),
                PortKey(KEY_DEPLOY_TX_HASH.to_string()),
                PortKey(KEY_DEPLOY_RECEIPT.to_string()),
            ],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: EvmDeployConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid evm_deploy op_config"))?;

        ensure_nonempty_artifact_port(&cfg.artifact_port)
            .map_err(|_| sdk_err("invalid_op_config", "artifact_port must be non-empty"))?;
        if let Some(env_name) = cfg.signing_key_env.as_deref() {
            ensure_nonempty_env_name(env_name)
                .map_err(|_| sdk_err("invalid_op_config", "signing_key_env must be non-empty"))?;
        }
        if let Some(artifact) = &cfg.artifact {
            parse_artifact(artifact)
                .map_err(|_| sdk_err("invalid_op_config", "invalid contract artifact"))?;
        }

        let from = normalize_address(&cfg.from)
            .map_err(|_| sdk_err("invalid_op_config", "invalid from address"))?;

        ensure_nonzero_polls(cfg.max_receipt_polls)
            .map_err(|_| sdk_err("invalid_op_config", "max_receipt_polls must be > 0"))?;

        let value_hex = parse_value_wei_to_hex(&cfg.value_wei)
            .map_err(|_| sdk_err("invalid_op_config", "invalid value_wei"))?;

        let state_id = StateId(format!("{}.deploy", op_path.0));
        let state = Arc::new(DeployState {
            state_id: state_id.clone(),
            cfg: DeployRuntimeConfig {
                artifact: cfg.artifact,
                artifact_port: cfg.artifact_port,
                from,
                constructor_args: cfg.constructor_args,
                value_hex,
                signing_key_env: cfg.signing_key_env,
                poll_interval_ms: cfg.poll_interval_ms,
                max_receipt_polls: cfg.max_receipt_polls,
            },
        });

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state,
            }],
            edges: Vec::new(),
        })
    }
}

struct DeployState {
    state_id: StateId,
    cfg: DeployRuntimeConfig,
}

#[async_trait]
impl State for DeployState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_scope(
            "mfm:evm_deploy",
            &self.state_id,
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let artifact = resolve_artifact_config(ctx, &self.cfg.artifact, &self.cfg.artifact_port)?;
        let (abi, bytecode) = parse_artifact(&artifact)
            .map_err(|_| state_err("invalid_contract_artifact", "contract artifact was invalid"))?;
        let constructor_payload = constructor_data(&abi, &bytecode, &self.cfg.constructor_args)
            .map_err(|_| state_err("invalid_op_config", "constructor args did not match ABI"))?;

        let mut client = EvmIoClient::new(self.state_id.clone(), io);

        let mut tx = serde_json::json!({
            "from": self.cfg.from,
            "data": bytes_to_hex_prefixed(&constructor_payload),
        });
        if let Some(v) = &self.cfg.value_hex {
            tx["value"] = serde_json::json!(v);
        }

        let tx_hash = if let Some(env_name) = self.cfg.signing_key_env.as_deref() {
            send_signed_create_transaction(
                &mut client,
                env_name,
                &self.cfg.from,
                &constructor_payload,
                self.cfg.value_hex.as_deref(),
            )
            .await?
        } else {
            send_transaction(&mut client, tx).await?
        };
        drop(client);

        let receipt = wait_for_receipt(
            &self.state_id,
            io,
            &tx_hash,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        ensure_receipt_success(&receipt)?;

        let contract_address = receipt_contract_address(&receipt)?;

        context_write_json(
            ctx,
            KEY_CONTRACT_ADDRESS,
            serde_json::json!(contract_address),
        )?;
        context_write_json(ctx, KEY_DEPLOY_TX_HASH, serde_json::json!(tx_hash))?;
        context_write_json(ctx, KEY_DEPLOY_RECEIPT, receipt)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone, Default)]
pub struct EvmConfigureOp;

impl Operation for EvmConfigureOp {
    fn op_id(&self) -> OpId {
        OpId(OP_ID_CONFIGURE.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmConfigureConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid evm_configure op_config"))?;
        ensure_nonempty_artifact_port(&cfg.artifact_port)
            .map_err(|_| sdk_err("invalid_op_config", "artifact_port must be non-empty"))?;

        let mut imports = Vec::new();
        if cfg.artifact.is_none() {
            imports.push(PortKey(cfg.artifact_port.clone()));
        }
        if cfg.contract_address.is_none() {
            imports.push(PortKey(KEY_CONTRACT_ADDRESS.to_string()));
        }

        Ok(OpIo {
            imports,
            exports: vec![
                PortKey(KEY_CONFIGURE_TX_HASHES.to_string()),
                PortKey(KEY_CONFIGURE_RECEIPTS.to_string()),
            ],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: EvmConfigureConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid evm_configure op_config"))?;

        ensure_nonempty_artifact_port(&cfg.artifact_port)
            .map_err(|_| sdk_err("invalid_op_config", "artifact_port must be non-empty"))?;

        if cfg.calls.is_empty() {
            return Err(sdk_err(
                "invalid_op_config",
                "evm_configure requires at least one call",
            ));
        }

        let parsed_abi = if let Some(artifact) = &cfg.artifact {
            let (abi, _bytecode) = parse_artifact(artifact)
                .map_err(|_| sdk_err("invalid_op_config", "invalid contract artifact"))?;
            Some(abi)
        } else {
            None
        };

        let from = normalize_address(&cfg.from)
            .map_err(|_| sdk_err("invalid_op_config", "invalid from address"))?;

        let contract_address = cfg
            .contract_address
            .as_ref()
            .map(|a| normalize_address(a))
            .transpose()
            .map_err(|_| sdk_err("invalid_op_config", "invalid contract_address"))?;

        ensure_nonzero_polls(cfg.max_receipt_polls)
            .map_err(|_| sdk_err("invalid_op_config", "max_receipt_polls must be > 0"))?;

        let mut calls = Vec::with_capacity(cfg.calls.len());
        for c in &cfg.calls {
            if let Some(abi) = &parsed_abi {
                let _ = resolve_function_call(abi, &c.function, &c.args).map_err(|_| {
                    sdk_err("invalid_op_config", "configure call did not match ABI")
                })?;
            }
            let value_hex = parse_value_wei_to_hex(&c.value_wei)
                .map_err(|_| sdk_err("invalid_op_config", "invalid value_wei in configure call"))?;

            calls.push(ConfigureRuntimeCall {
                function: c.function.clone(),
                args: c.args.clone(),
                value_hex,
            });
        }

        let state_id = StateId(format!("{}.configure", op_path.0));
        let state = Arc::new(ConfigureState {
            state_id: state_id.clone(),
            cfg: ConfigureRuntimeConfig {
                artifact: cfg.artifact,
                artifact_port: cfg.artifact_port,
                from,
                contract_address,
                calls,
                poll_interval_ms: cfg.poll_interval_ms,
                max_receipt_polls: cfg.max_receipt_polls,
            },
        });

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state,
            }],
            edges: Vec::new(),
        })
    }
}

struct ConfigureState {
    state_id: StateId,
    cfg: ConfigureRuntimeConfig,
}

#[async_trait]
impl State for ConfigureState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_scope(
            "mfm:evm_configure",
            &self.state_id,
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let artifact = resolve_artifact_config(ctx, &self.cfg.artifact, &self.cfg.artifact_port)?;
        let (abi, _bytecode) = parse_artifact(&artifact)
            .map_err(|_| state_err("invalid_contract_artifact", "contract artifact was invalid"))?;

        let to = resolve_contract_address(ctx, &self.cfg.contract_address)?;

        let mut tx_hashes: Vec<serde_json::Value> = Vec::new();
        let mut receipts: Vec<serde_json::Value> = Vec::new();

        for call in &self.cfg.calls {
            let (calldata, _outputs) = resolve_function_call(&abi, &call.function, &call.args)
                .map_err(|_| state_err("invalid_op_config", "configure call did not match ABI"))?;
            let mut tx = serde_json::json!({
                "from": self.cfg.from,
                "to": to,
                "data": bytes_to_hex_prefixed(&calldata),
            });
            if let Some(v) = &call.value_hex {
                tx["value"] = serde_json::json!(v);
            }

            let mut client = EvmIoClient::new(self.state_id.clone(), io);
            let tx_hash = send_transaction(&mut client, tx).await?;
            drop(client);
            let receipt = wait_for_receipt(
                &self.state_id,
                io,
                &tx_hash,
                self.cfg.poll_interval_ms,
                self.cfg.max_receipt_polls,
            )
            .await?;
            ensure_receipt_success(&receipt)?;

            tx_hashes.push(serde_json::json!({
                "function": call.function,
                "tx_hash": tx_hash,
            }));
            receipts.push(receipt);
        }

        context_write_json(
            ctx,
            KEY_CONFIGURE_TX_HASHES,
            serde_json::Value::Array(tx_hashes),
        )?;
        context_write_json(
            ctx,
            KEY_CONFIGURE_RECEIPTS,
            serde_json::Value::Array(receipts),
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone, Default)]
pub struct EvmValidateOp;

impl Operation for EvmValidateOp {
    fn op_id(&self) -> OpId {
        OpId(OP_ID_VALIDATE.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmValidateConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid evm_validate op_config"))?;
        ensure_nonempty_artifact_port(&cfg.artifact_port)
            .map_err(|_| sdk_err("invalid_op_config", "artifact_port must be non-empty"))?;

        let mut imports = Vec::new();
        if cfg.artifact.is_none() {
            imports.push(PortKey(cfg.artifact_port.clone()));
        }
        if cfg.contract_address.is_none() {
            imports.push(PortKey(KEY_CONTRACT_ADDRESS.to_string()));
        }

        Ok(OpIo {
            imports,
            exports: vec![
                PortKey(KEY_VALIDATED.to_string()),
                PortKey(KEY_CHAIN_ID.to_string()),
                PortKey(KEY_CLIENT_VERSION.to_string()),
            ],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: EvmValidateConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid evm_validate op_config"))?;

        ensure_nonempty_artifact_port(&cfg.artifact_port)
            .map_err(|_| sdk_err("invalid_op_config", "artifact_port must be non-empty"))?;

        let parsed_abi = if let Some(artifact) = &cfg.artifact {
            let (abi, _bytecode) = parse_artifact(artifact)
                .map_err(|_| sdk_err("invalid_op_config", "invalid contract artifact"))?;
            Some(abi)
        } else {
            None
        };

        let contract_address = cfg
            .contract_address
            .as_ref()
            .map(|a| normalize_address(a))
            .transpose()
            .map_err(|_| sdk_err("invalid_op_config", "invalid contract_address"))?;

        if cfg.require_client_substring.trim().is_empty() {
            return Err(sdk_err(
                "invalid_op_config",
                "require_client_substring must be non-empty",
            ));
        }

        if let Some(abi) = &parsed_abi {
            let _ = prepare_validate_assertions(abi, &cfg.read_assertions, &cfg.event_assertions)
                .map_err(|err| match err.as_str() {
                "read assertion did not match ABI" => {
                    sdk_err("invalid_op_config", "read assertion did not match ABI")
                }
                "event assertion referenced unknown event" => sdk_err(
                    "invalid_op_config",
                    "event assertion referenced unknown event",
                ),
                "anonymous events are not supported for validation" => sdk_err(
                    "invalid_op_config",
                    "anonymous events are not supported for validation",
                ),
                "invalid from_block" => sdk_err("invalid_op_config", "invalid from_block"),
                "invalid to_block" => sdk_err("invalid_op_config", "invalid to_block"),
                _ => sdk_err("invalid_op_config", "invalid evm_validate op_config"),
            })?;
        }

        let state_id = StateId(format!("{}.validate", op_path.0));
        let state = Arc::new(ValidateState {
            state_id: state_id.clone(),
            cfg: ValidateRuntimeConfig {
                artifact: cfg.artifact,
                artifact_port: cfg.artifact_port,
                contract_address,
                expected_chain_id: cfg.expected_chain_id,
                require_client_substring: cfg.require_client_substring,
                read_assertions: cfg.read_assertions,
                event_assertions: cfg.event_assertions,
            },
        });

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state,
            }],
            edges: Vec::new(),
        })
    }
}

struct ValidateState {
    state_id: StateId,
    cfg: ValidateRuntimeConfig,
}

#[async_trait]
impl State for ValidateState {
    fn meta(&self) -> StateMeta {
        meta::validate()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let artifact = resolve_artifact_config(ctx, &self.cfg.artifact, &self.cfg.artifact_port)?;
        let (abi, _bytecode) = parse_artifact(&artifact)
            .map_err(|_| state_err("invalid_contract_artifact", "contract artifact was invalid"))?;
        let (read_assertions, event_assertions) = prepare_validate_assertions(
            &abi,
            &self.cfg.read_assertions,
            &self.cfg.event_assertions,
        )
        .map_err(|err| match err.as_str() {
            "read assertion did not match ABI" => {
                state_err("invalid_op_config", "read assertion did not match ABI")
            }
            "event assertion referenced unknown event" => state_err(
                "invalid_op_config",
                "event assertion referenced unknown event",
            ),
            "anonymous events are not supported for validation" => state_err(
                "invalid_op_config",
                "anonymous events are not supported for validation",
            ),
            "invalid from_block" => state_err("invalid_op_config", "invalid from_block"),
            "invalid to_block" => state_err("invalid_op_config", "invalid to_block"),
            _ => state_err("invalid_op_config", "invalid evm_validate op_config"),
        })?;

        let mut client = EvmIoClient::new(self.state_id.clone(), io);

        let client_version_res = client
            .call(JsonRpcCall::new(
                "web3_clientVersion",
                serde_json::json!([]),
            ))
            .await
            .map_err(state_err_from_io)?;
        let client_version = op_rpc::expect_string(
            &client_version_res.response,
            "evm_response_invalid",
            "web3_clientVersion was not a string",
        )?;
        op_rpc::assert_condition(
            client_version
                .to_ascii_lowercase()
                .contains(&self.cfg.require_client_substring.to_ascii_lowercase()),
            "reth_client_mismatch",
            "rpc clientVersion did not match required reth substring",
        )?;

        let chain_id = client.chain_id_u64().await.map_err(state_err_from_io)?;
        op_rpc::assert_condition(
            chain_id == self.cfg.expected_chain_id,
            "chain_id_mismatch",
            "rpc chain id did not match expected_chain_id",
        )?;

        let to = resolve_contract_address(ctx, &self.cfg.contract_address)?;

        for ra in &read_assertions {
            let res = client
                .call(JsonRpcCall::new(
                    "eth_call",
                    serde_json::json!([
                        {"to": to, "data": ra.data_hex},
                        "latest"
                    ]),
                ))
                .await
                .map_err(state_err_from_io)?;

            let raw = op_rpc::expect_string(
                &res.response,
                "evm_response_invalid",
                "eth_call returned non-string",
            )?;

            let actual = decode_single_output_to_json(&ra.outputs, &raw).map_err(|_| {
                state_err("evm_response_invalid", "failed to decode eth_call output")
            })?;

            op_rpc::assert_condition(
                expected_matches(&actual, &ra.expected),
                "validation_failed",
                "read assertion failed during evm_validate",
            )?;
        }

        for ea in &event_assertions {
            let logs_res = client
                .call(JsonRpcCall::new(
                    "eth_getLogs",
                    serde_json::json!([
                        {
                            "address": to,
                            "fromBlock": ea.from_block,
                            "toBlock": ea.to_block,
                            "topics": [ea.topic0_hex],
                        }
                    ]),
                ))
                .await
                .map_err(state_err_from_io)?;

            let logs = op_rpc::expect_array(
                &logs_res.response,
                "evm_response_invalid",
                "eth_getLogs returned non-array",
            )?;
            op_rpc::assert_condition(
                u64::try_from(logs.len()).unwrap_or(0) >= ea.min_count,
                "validation_failed",
                "event assertion failed during evm_validate",
            )?;

            let _ = &ea.event;
        }

        context_write_json(ctx, KEY_CHAIN_ID, serde_json::json!(chain_id))?;
        context_write_json(ctx, KEY_CLIENT_VERSION, serde_json::json!(client_version))?;
        context_write_json(ctx, KEY_VALIDATED, serde_json::json!(true))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_collectors_evm::parse_u64_hex_value;

    fn sample_artifact() -> serde_json::Value {
        serde_json::json!({
            "abi": [
                {
                    "type": "constructor",
                    "inputs": [{"name": "x", "type": "uint256"}]
                },
                {
                    "type": "function",
                    "name": "setValue",
                    "inputs": [{"name": "x", "type": "uint256"}],
                    "outputs": []
                },
                {
                    "type": "function",
                    "name": "getValue",
                    "inputs": [],
                    "outputs": [{"name": "", "type": "uint256"}]
                },
                {
                    "type": "event",
                    "name": "ValueSet",
                    "inputs": [{"name": "x", "type": "uint256", "indexed": false}],
                    "anonymous": false
                }
            ],
            "bytecode": {
                "object": "0x60006000"
            }
        })
    }

    #[test]
    fn encode_params_uint_address() {
        let types = vec!["uint256".to_string(), "address".to_string()];
        let args = vec![
            serde_json::json!(7),
            serde_json::json!("0x1111111111111111111111111111111111111111"),
        ];

        let out = encode_params(&types, &args).expect("encode");
        assert_eq!(out.len(), 64);
        assert_eq!(out[31], 7u8);
        assert_eq!(
            &out[44..64],
            &hex_to_bytes("0x1111111111111111111111111111111111111111").unwrap()
        );
    }

    #[test]
    fn encode_dynamic_string() {
        let types = vec!["string".to_string()];
        let args = vec![serde_json::json!("hello")];

        let out = encode_params(&types, &args).expect("encode");
        assert_eq!(out.len(), 96);
        // offset = 0x20
        assert_eq!(out[31], 32u8);
        // len = 5
        assert_eq!(out[63], 5u8);
    }

    #[test]
    fn resolve_function_call_works() {
        let artifact: ContractArtifactConfig =
            serde_json::from_value(sample_artifact()).expect("artifact config");
        let (abi, _bytecode) = parse_artifact(&artifact).expect("parse artifact");

        let (calldata, _outputs) =
            resolve_function_call(&abi, "setValue", &[serde_json::json!(42)]).expect("call");

        assert!(calldata.len() >= 4 + 32);
    }

    #[test]
    fn deploy_io_exports_contract_address() {
        let op = EvmDeployOp;
        let io = op
            .io(&serde_json::json!({
                "artifact": sample_artifact(),
                "from": "0x1111111111111111111111111111111111111111"
            }))
            .expect("io");
        assert!(io
            .exports
            .iter()
            .any(|k| k.0.as_str() == KEY_CONTRACT_ADDRESS));
    }

    #[test]
    fn deploy_io_imports_artifact_port_when_artifact_is_unset() {
        let op = EvmDeployOp;
        let io = op
            .io(&serde_json::json!({
                "artifact_port": "contract_artifact",
                "from": "0x1111111111111111111111111111111111111111"
            }))
            .expect("io");
        assert!(io
            .imports
            .iter()
            .any(|k| k.0.as_str() == KEY_CONTRACT_ARTIFACT));
    }

    #[test]
    fn configure_io_imports_contract_address_when_unset() {
        let op = EvmConfigureOp;
        let io = op
            .io(&serde_json::json!({
                "artifact": sample_artifact(),
                "from": "0x1111111111111111111111111111111111111111",
                "calls": [{"function":"setValue","args":[1]}]
            }))
            .expect("io");

        assert!(io
            .imports
            .iter()
            .any(|k| k.0.as_str() == KEY_CONTRACT_ADDRESS));
    }

    #[test]
    fn contract_from_nix_io_imports_result_and_exports_artifact() {
        let op = EvmContractFromNixOp;
        let io = op
            .io(&serde_json::json!({
                "result_pointer": "/artifact"
            }))
            .expect("io");

        assert!(io.imports.iter().any(|k| k.0.as_str() == KEY_NIX_RESULT));
        assert!(io
            .exports
            .iter()
            .any(|k| k.0.as_str() == KEY_CONTRACT_ARTIFACT));
    }

    #[test]
    fn validate_expand_builds_read_and_event_assertions() {
        let op = EvmValidateOp;
        let plan = op
            .expand(
                OpPath("m.validate".to_string()),
                &serde_json::json!({
                    "artifact": sample_artifact(),
                    "contract_address": "0x1111111111111111111111111111111111111111",
                    "expected_chain_id": 1337,
                    "read_assertions": [
                        {"function":"getValue","args":[],"expected": 1}
                    ],
                    "event_assertions": [
                        {"event":"ValueSet","min_count":1}
                    ]
                }),
                &RunConfig {
                    nix_flake_allowlist: Vec::new(),
                    io_mode: mfm_machine::config::IoMode::Live,
                    retry_policy: mfm_machine::config::RetryPolicy {
                        max_attempts: 1,
                        backoff: mfm_machine::config::BackoffPolicy::Fixed {
                            delay: Duration::from_millis(0),
                        },
                    },
                    event_profile: mfm_machine::config::EventProfile::Normal,
                    execution_mode: mfm_machine::config::ExecutionMode::Sequential,
                    context_checkpointing:
                        mfm_machine::config::ContextCheckpointing::AfterEveryState,
                    replay_missing_fact_retryable: false,
                    skip_tags: Vec::new(),
                },
            )
            .expect("expand");

        assert_eq!(plan.states.len(), 1);
        assert!(plan.edges.is_empty());
    }

    #[test]
    fn parse_u64_hex_value_still_works_for_receipt_numbers() {
        let v = serde_json::json!("0x7b");
        assert_eq!(parse_u64_hex_value(&v).unwrap(), 123);
    }
}
