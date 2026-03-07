#![warn(missing_docs)]
//! EVM write/validation operations.
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
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_evm_write::EvmDeployOp;
//! use mfm_sdk::op::Operation;
//!
//! let op = EvmDeployOp;
//! assert_eq!(op.op_id().as_str(), "evm_deploy");
//! ```

use std::sync::Arc;

#[cfg(test)]
use k256::ecdsa::SigningKey;
use serde::Deserialize;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use zeroize::Zeroizing;

use alloy_primitives::keccak256;
use mfm_evm_runtime::dcv as shared_dcv;
use mfm_evm_runtime::states::write::{
    EvmConfigureRuntimeCall as SharedConfigureRuntimeCall,
    EvmConfigureState as SharedConfigureState,
    EvmConfigureStateConfig as SharedConfigureStateConfig, EvmDeployState as SharedDeployState,
    EvmDeployStateConfig as SharedDeployStateConfig, EvmValidateState as SharedValidateState,
    EvmValidateStateConfig as SharedValidateStateConfig, NixArtifactToEvmContractState,
};
use mfm_machine::config::RunConfig;
#[cfg(test)]
use mfm_machine::errors::StateError;
use mfm_machine::ids::{OpId, OpPath, StateId};
use mfm_machine::plan::{StateGraph, StateNode};
use mfm_state_common::errors as op_errors;
use mfm_state_common::rpc as op_rpc;

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
struct ConfigureRuntimeCall {
    function: String,
    args: Vec<serde_json::Value>,
    value_hex: Option<String>,
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

#[cfg(test)]
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

fn parse_artifact(cfg: &ContractArtifactConfig) -> Result<(ParsedAbi, Vec<u8>), String> {
    let abi = parse_abi(&cfg.abi)?;
    let bytecode = parse_bytecode(&cfg.bytecode)?;
    Ok((abi, bytecode))
}

fn validate_assertions_match_abi(
    abi: &ParsedAbi,
    read_assertions: &[ReadAssertionConfig],
    event_assertions: &[EventAssertionConfig],
) -> Result<(), String> {
    for ra in read_assertions {
        let _ = resolve_function_call(abi, &ra.function, &ra.args)
            .map_err(|_| "read assertion did not match ABI".to_string())?;
    }

    for ea in event_assertions {
        let Some(event) = abi.events.iter().find(|e| e.name == ea.event) else {
            return Err("event assertion referenced unknown event".to_string());
        };

        if event.anonymous {
            return Err("anonymous events are not supported for validation".to_string());
        }

        let _topic0 = keccak256(format!("{}({})", event.name, event.inputs.join(",")).as_bytes());
    }

    Ok(())
}

fn ensure_nonempty_env_name(env_name: &str) -> Result<(), String> {
    if env_name.trim().is_empty() {
        return Err("signing_key_env must be non-empty".to_string());
    }
    Ok(())
}

#[cfg(test)]
fn signing_key_from_env(signing_key_env: &str) -> Result<SigningKey, StateError> {
    let raw = Zeroizing::new(std::env::var(signing_key_env).map_err(|_| {
        op_errors::state_unknown(
            "missing_signing_key_env",
            "signing_key_env did not exist in process environment",
        )
    })?);

    let normalized = Zeroizing::new(normalize_hex_str(raw.as_str()).map_err(|_| {
        op_errors::state_unknown("invalid_signing_key_env", "signing key hex was invalid")
    })?);
    let bytes = Zeroizing::new(hex_to_bytes(normalized.as_str()).map_err(|_| {
        op_errors::state_unknown("invalid_signing_key_env", "signing key hex was invalid")
    })?);
    if bytes.len() != 32 {
        return Err(op_errors::state_unknown(
            "invalid_signing_key_env",
            "signing key must be exactly 32 bytes",
        ));
    }

    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(bytes.as_slice());

    SigningKey::from_bytes((&*key).into()).map_err(|_| {
        op_errors::state_unknown(
            "invalid_signing_key_env",
            "signing key did not form a valid secp256k1 key",
        )
    })
}

#[cfg(test)]
fn signer_address_hex(signing_key: &SigningKey) -> String {
    let public_key = signing_key.verifying_key().to_encoded_point(false);
    let hash = keccak256(&public_key.as_bytes()[1..]);
    bytes_to_hex_prefixed(&hash.as_slice()[12..])
}

/// Planner that adapts a `nix_app` result into a contract artifact export.
#[derive(Clone, Default)]
pub struct EvmContractFromNixOp;

impl Operation for EvmContractFromNixOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_CONTRACT_FROM_NIX.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmContractFromNixConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid evm_contract_from_nix op_config",
                )
            })?;
        if !cfg.result_pointer.is_empty() && !cfg.result_pointer.starts_with('/') {
            return Err(op_errors::sdk_parse_error(
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
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid evm_contract_from_nix op_config",
                )
            })?;
        if !cfg.result_pointer.is_empty() && !cfg.result_pointer.starts_with('/') {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "result_pointer must be empty or start with '/'",
            ));
        }

        let state_id = StateId::must_new(format!("{}.adapt", op_path.0));
        let state = Arc::new(NixArtifactToEvmContractState {
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

/// Planner that deploys an EVM contract artifact.
#[derive(Clone, Default)]
pub struct EvmDeployOp;

impl Operation for EvmDeployOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_DEPLOY.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmDeployConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_deploy op_config")
        })?;
        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;
        if let Some(env_name) = cfg.signing_key_env.as_deref() {
            ensure_nonempty_env_name(env_name).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "signing_key_env must be non-empty")
            })?;
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
        let cfg: EvmDeployConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_deploy op_config")
        })?;

        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;
        if let Some(env_name) = cfg.signing_key_env.as_deref() {
            ensure_nonempty_env_name(env_name).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "signing_key_env must be non-empty")
            })?;
        }
        if let Some(artifact) = &cfg.artifact {
            parse_artifact(artifact).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid contract artifact")
            })?;
        }

        let from = shared_dcv::normalize_address(&cfg.from)
            .map_err(|_| op_errors::sdk_parse_error("invalid_op_config", "invalid from address"))?;

        shared_dcv::ensure_nonzero_polls(cfg.max_receipt_polls).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "max_receipt_polls must be > 0")
        })?;

        let value_hex = shared_dcv::parse_value_wei_to_hex(&cfg.value_wei)
            .map_err(|_| op_errors::sdk_parse_error("invalid_op_config", "invalid value_wei"))?;

        let state_id = StateId::must_new(format!("{}.deploy", op_path.0));
        let state = Arc::new(SharedDeployState {
            state_id: state_id.clone(),
            cfg: SharedDeployStateConfig {
                artifact: cfg.artifact.map(|a| shared_dcv::ContractArtifactConfig {
                    abi: a.abi,
                    bytecode: a.bytecode,
                }),
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

/// Planner that executes post-deploy contract calls.
#[derive(Clone, Default)]
pub struct EvmConfigureOp;

impl Operation for EvmConfigureOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_CONFIGURE.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmConfigureConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_configure op_config")
        })?;
        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;

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
        let cfg: EvmConfigureConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_configure op_config")
        })?;

        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;

        if cfg.calls.is_empty() {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "evm_configure requires at least one call",
            ));
        }

        let parsed_abi = if let Some(artifact) = &cfg.artifact {
            let (abi, _bytecode) = parse_artifact(artifact).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid contract artifact")
            })?;
            Some(abi)
        } else {
            None
        };

        let from = shared_dcv::normalize_address(&cfg.from)
            .map_err(|_| op_errors::sdk_parse_error("invalid_op_config", "invalid from address"))?;

        let contract_address = cfg
            .contract_address
            .as_ref()
            .map(|a| shared_dcv::normalize_address(a))
            .transpose()
            .map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid contract_address")
            })?;

        shared_dcv::ensure_nonzero_polls(cfg.max_receipt_polls).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "max_receipt_polls must be > 0")
        })?;

        let mut calls = Vec::with_capacity(cfg.calls.len());
        for c in &cfg.calls {
            if let Some(abi) = &parsed_abi {
                let _ = resolve_function_call(abi, &c.function, &c.args).map_err(|_| {
                    op_errors::sdk_parse_error(
                        "invalid_op_config",
                        "configure call did not match ABI",
                    )
                })?;
            }
            let value_hex = shared_dcv::parse_value_wei_to_hex(&c.value_wei).map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid value_wei in configure call",
                )
            })?;

            calls.push(ConfigureRuntimeCall {
                function: c.function.clone(),
                args: c.args.clone(),
                value_hex,
            });
        }

        let state_id = StateId::must_new(format!("{}.configure", op_path.0));
        let state = Arc::new(SharedConfigureState {
            state_id: state_id.clone(),
            cfg: SharedConfigureStateConfig {
                artifact: cfg.artifact.map(|a| shared_dcv::ContractArtifactConfig {
                    abi: a.abi,
                    bytecode: a.bytecode,
                }),
                artifact_port: cfg.artifact_port,
                from,
                contract_address,
                calls: calls
                    .into_iter()
                    .map(|c| SharedConfigureRuntimeCall {
                        function: c.function,
                        args: c.args,
                        value_hex: c.value_hex,
                    })
                    .collect(),
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

/// Planner that validates deployed contracts and chain assumptions.
#[derive(Clone, Default)]
pub struct EvmValidateOp;

impl Operation for EvmValidateOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_VALIDATE.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmValidateConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_validate op_config")
        })?;
        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;

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
        let cfg: EvmValidateConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_validate op_config")
        })?;

        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;

        let parsed_abi = if let Some(artifact) = &cfg.artifact {
            let (abi, _bytecode) = parse_artifact(artifact).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid contract artifact")
            })?;
            Some(abi)
        } else {
            None
        };

        let contract_address = cfg
            .contract_address
            .as_ref()
            .map(|a| shared_dcv::normalize_address(a))
            .transpose()
            .map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid contract_address")
            })?;

        if cfg.require_client_substring.trim().is_empty() {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "require_client_substring must be non-empty",
            ));
        }

        if let Some(abi) = &parsed_abi {
            validate_assertions_match_abi(abi, &cfg.read_assertions, &cfg.event_assertions)
                .map_err(|err| {
                    op_errors::sdk_parse_error(
                        "invalid_op_config",
                        op_rpc::validation_assertion_error_message(&err),
                    )
                })?;
        }

        let state_id = StateId::must_new(format!("{}.validate", op_path.0));
        let state = Arc::new(SharedValidateState {
            state_id: state_id.clone(),
            cfg: SharedValidateStateConfig {
                artifact: cfg.artifact.map(|a| shared_dcv::ContractArtifactConfig {
                    abi: a.abi,
                    bytecode: a.bytecode,
                }),
                artifact_port: cfg.artifact_port,
                contract_address,
                expected_chain_id: cfg.expected_chain_id,
                require_client_substring: cfg.require_client_substring,
                read_assertions: cfg
                    .read_assertions
                    .into_iter()
                    .map(|a| shared_dcv::ReadAssertionConfig {
                        function: a.function,
                        args: a.args,
                        expected: a.expected,
                    })
                    .collect(),
                event_assertions: cfg
                    .event_assertions
                    .into_iter()
                    .map(|a| shared_dcv::EventAssertionConfig {
                        event: a.event,
                        min_count: a.min_count,
                        from_block: a.from_block.map(|b| match b {
                            BlockTag::Number(n) => shared_dcv::BlockTag::Number(n),
                            BlockTag::Tag(s) => shared_dcv::BlockTag::Tag(s),
                        }),
                        to_block: a.to_block.map(|b| match b {
                            BlockTag::Number(n) => shared_dcv::BlockTag::Number(n),
                            BlockTag::Tag(s) => shared_dcv::BlockTag::Tag(s),
                        }),
                    })
                    .collect(),
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

    #[test]
    fn signing_key_from_env_parses_valid_key() {
        let env_name = "MFM_TEST_SIGNING_KEY_VALID";
        std::env::set_var(
            env_name,
            "0x0000000000000000000000000000000000000000000000000000000000000001",
        );

        let key = signing_key_from_env(env_name).expect("signing key should parse");
        assert_eq!(
            signer_address_hex(&key),
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );

        std::env::remove_var(env_name);
    }

    #[test]
    fn signing_key_from_env_rejects_short_key() {
        let env_name = "MFM_TEST_SIGNING_KEY_INVALID";
        std::env::set_var(env_name, "0x1234");

        let err = signing_key_from_env(env_name).expect_err("short key should fail");
        assert!(err.info.message.contains("exactly 32 bytes"));

        std::env::remove_var(env_name);
    }

    #[test]
    fn signing_key_from_env_rejects_missing_env() {
        let env_name = "MFM_TEST_SIGNING_KEY_MISSING";
        std::env::remove_var(env_name);

        let err = signing_key_from_env(env_name).expect_err("missing env should fail");
        assert!(err
            .info
            .message
            .contains("did not exist in process environment"));
    }

    #[test]
    fn signing_key_from_env_rejects_invalid_hex() {
        let env_name = "MFM_TEST_SIGNING_KEY_BAD_HEX";
        std::env::set_var(
            env_name,
            "0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        );

        let err = signing_key_from_env(env_name).expect_err("invalid hex should fail");
        assert!(err.info.message.contains("hex was invalid"));

        std::env::remove_var(env_name);
    }

    #[test]
    fn signing_key_from_env_rejects_invalid_curve_key() {
        let env_name = "MFM_TEST_SIGNING_KEY_ZERO";
        std::env::set_var(
            env_name,
            "0x0000000000000000000000000000000000000000000000000000000000000000",
        );

        let err = signing_key_from_env(env_name).expect_err("zero key should fail");
        assert!(err.info.message.contains("valid secp256k1 key"));

        std::env::remove_var(env_name);
    }
}
