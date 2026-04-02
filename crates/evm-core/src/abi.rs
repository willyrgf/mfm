//! ABI parsing and calldata construction helpers.
//!
//! This module intentionally keeps the supported surface small: enough to parse compiler ABI JSON,
//! derive selectors, and build common constructor/function call payloads used by higher-level
//! runtime crates.
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm_core::abi::{function_selector, parse_abi};
//!
//! let abi = serde_json::json!([
//!   {
//!     "type": "function",
//!     "name": "balanceOf",
//!     "inputs": [{"type": "address"}],
//!     "outputs": [{"type": "uint256"}]
//!   }
//! ]);
//!
//! let parsed = parse_abi(&abi)?;
//! assert_eq!(parsed.functions[0].name, "balanceOf");
//! assert_eq!(function_selector("balanceOf", &["address".to_string()]), [0x70, 0xa0, 0x82, 0x31]);
//! # Ok::<(), mfm_evm_core::util_error::UtilError>(())
//! ```

use std::borrow::Borrow;

use alloy_primitives::keccak256;
use serde::Deserialize;

use crate::encoding::{encode_len_word, parse_address_hex};
use crate::hex::{bytes_to_hex_prefixed, hex_nibble, hex_to_bytes, normalize_hex_str};
use crate::util_error::UtilError;

/// Simplified function entry extracted from a JSON ABI.
#[derive(Clone, Debug)]
pub struct AbiFunction {
    /// Solidity function name.
    pub name: String,
    /// Canonical Solidity input types.
    pub inputs: Vec<String>,
    /// Canonical Solidity output types.
    pub outputs: Vec<String>,
}

/// Simplified event entry extracted from a JSON ABI.
#[derive(Clone, Debug)]
pub struct AbiEvent {
    /// Solidity event name.
    pub name: String,
    /// Canonical Solidity input types.
    pub inputs: Vec<String>,
    /// Whether the event omits topic 0.
    pub anonymous: bool,
}

/// Parsed ABI summary used by higher-level planning code.
#[derive(Clone, Debug, Default)]
pub struct ParsedAbi {
    /// Canonical constructor input types.
    pub constructor_inputs: Vec<String>,
    /// Parsed function entries.
    pub functions: Vec<AbiFunction>,
    /// Parsed event entries.
    pub events: Vec<AbiEvent>,
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

/// Parses a JSON ABI array into a simplified [`ParsedAbi`] representation.
pub fn parse_abi(abi: &serde_json::Value) -> Result<ParsedAbi, UtilError> {
    let items: Vec<AbiItemJson> = serde_json::from_value(abi.clone()).map_err(|_| {
        UtilError::new("invalid_abi", "artifact.abi must be a valid JSON ABI array")
    })?;

    let mut out = ParsedAbi::default();
    for it in items {
        match it.kind.as_str() {
            "constructor" => {
                out.constructor_inputs = it.inputs.into_iter().map(|p| p.typ).collect();
            }
            "function" => {
                let Some(name) = it.name else {
                    return Err(UtilError::new(
                        "invalid_abi",
                        "function ABI item missing name",
                    ));
                };
                out.functions.push(AbiFunction {
                    name,
                    inputs: it.inputs.into_iter().map(|p| p.typ).collect(),
                    outputs: it.outputs.into_iter().map(|p| p.typ).collect(),
                });
            }
            "event" => {
                let Some(name) = it.name else {
                    return Err(UtilError::new("invalid_abi", "event ABI item missing name"));
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

/// Parses compiler bytecode output from either a raw string or `{ object: ... }` form.
pub fn parse_bytecode(v: &serde_json::Value) -> Result<Vec<u8>, UtilError> {
    match v {
        serde_json::Value::String(s) => hex_to_bytes(s),
        serde_json::Value::Object(m) => {
            let Some(obj) = m.get("object") else {
                return Err(UtilError::new(
                    "invalid_bytecode",
                    "artifact.bytecode object missing field `object`",
                ));
            };
            let Some(s) = obj.as_str() else {
                return Err(UtilError::new(
                    "invalid_bytecode",
                    "artifact.bytecode.object must be a string",
                ));
            };
            hex_to_bytes(s)
        }
        _ => Err(UtilError::new(
            "invalid_bytecode",
            "artifact.bytecode must be a string or object with `object`",
        )),
    }
}

/// Computes the first four bytes of the Keccak-256 function signature hash.
///
/// This is the standard Ethereum ABI selector derivation used for function calldata prefixes.
pub fn function_selector(name: &str, input_types: &[String]) -> [u8; 4] {
    let sig = format!("{}({})", name, input_types.join(","));
    let h = keccak256(sig.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

fn parse_uint_word(v: &serde_json::Value) -> Result<[u8; 32], UtilError> {
    let mut out = [0u8; 32];

    match v {
        serde_json::Value::Number(n) => {
            let Some(u) = n.as_u64() else {
                return Err(UtilError::new(
                    "invalid_uint",
                    "numeric value must fit into u64",
                ));
            };
            out[24..32].copy_from_slice(&u.to_be_bytes());
            Ok(out)
        }
        serde_json::Value::String(s) => {
            if s.starts_with("0x") || s.starts_with("0X") {
                let b = hex_to_bytes(s)?;
                if b.len() > 32 {
                    return Err(UtilError::new(
                        "invalid_uint",
                        "hex integer must fit into 32 bytes",
                    ));
                }
                out[32 - b.len()..].copy_from_slice(&b);
                return Ok(out);
            }

            let parsed = s.parse::<u128>().map_err(|_| {
                UtilError::new("invalid_uint", "decimal integer must fit into u128")
            })?;
            out[16..32].copy_from_slice(&parsed.to_be_bytes());
            Ok(out)
        }
        _ => Err(UtilError::new(
            "invalid_uint",
            "integer arg must be a number or string",
        )),
    }
}

fn parse_bool_word(v: &serde_json::Value) -> Result<[u8; 32], UtilError> {
    let b = match v {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => match n.as_u64() {
            Some(0) => false,
            Some(1) => true,
            _ => {
                return Err(UtilError::new(
                    "invalid_bool",
                    "bool numeric arg must be 0 or 1",
                ));
            }
        },
        serde_json::Value::String(s) => match s.as_str() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => {
                return Err(UtilError::new(
                    "invalid_bool",
                    "bool arg must be true/false/0/1",
                ));
            }
        },
        _ => {
            return Err(UtilError::new(
                "invalid_bool",
                "bool arg must be bool/number/string",
            ));
        }
    };

    let mut out = [0u8; 32];
    out[31] = if b { 1 } else { 0 };
    Ok(out)
}

fn parse_bytes_m_word(m: usize, v: &serde_json::Value) -> Result<[u8; 32], UtilError> {
    let Some(s) = v.as_str() else {
        return Err(UtilError::new(
            "invalid_bytes_m",
            "bytesM arg must be a hex string",
        ));
    };
    let b = hex_to_bytes(s)?;
    if b.len() != m {
        return Err(UtilError::new(
            "invalid_bytes_m",
            "bytesM arg length mismatch",
        ));
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

fn encode_dynamic_value(t: &str, v: &serde_json::Value) -> Result<Vec<u8>, UtilError> {
    let data = match t {
        "string" => {
            let Some(s) = v.as_str() else {
                return Err(UtilError::new(
                    "invalid_string",
                    "string arg must be a string",
                ));
            };
            s.as_bytes().to_vec()
        }
        "bytes" => {
            let Some(s) = v.as_str() else {
                return Err(UtilError::new(
                    "invalid_bytes",
                    "bytes arg must be a hex string",
                ));
            };
            hex_to_bytes(s)?
        }
        _ => {
            return Err(UtilError::new(
                "unsupported_type",
                "unsupported dynamic type",
            ))
        }
    };

    let mut out = Vec::new();
    out.extend_from_slice(&encode_len_word(data.len())?);
    out.extend_from_slice(&pad_to_32(data));
    Ok(out)
}

fn encode_address_word(addr: &str) -> Result<Vec<u8>, UtilError> {
    let n = normalize_hex_str(addr)?;
    let rest = n.strip_prefix("0x").unwrap_or_default();
    if rest.len() != 40 {
        return Err(UtilError::new(
            "invalid_address",
            "address must be 20 bytes",
        ));
    }

    let mut out = vec![0u8; 32];
    for (i, chunk) in rest.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0])
            .ok_or_else(|| UtilError::new("invalid_address", "invalid address hex"))?;
        let lo = hex_nibble(chunk[1])
            .ok_or_else(|| UtilError::new("invalid_address", "invalid address hex"))?;
        out[12 + i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn encode_static_value(t: &str, v: &serde_json::Value) -> Result<[u8; 32], UtilError> {
    if t == "address" {
        let Some(s) = v.as_str() else {
            return Err(UtilError::new(
                "invalid_address",
                "address arg must be a string",
            ));
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
            .map_err(|_| UtilError::new("invalid_bytes_m", "invalid bytesM type"))?;
        if m == 0 || m > 32 {
            return Err(UtilError::new(
                "invalid_bytes_m",
                "bytesM size must be in [1,32]",
            ));
        }
        return parse_bytes_m_word(m, v);
    }

    Err(UtilError::new(
        "unsupported_type",
        "unsupported static ABI type",
    ))
}

/// ABI-encodes function or constructor arguments.
///
/// Supported dynamic types are `bytes` and `string`. Supported static types include `address`,
/// integer types, `bool`, and `bytesM`.
pub fn encode_params<T>(types: &[String], args: &[T]) -> Result<Vec<u8>, UtilError>
where
    T: Borrow<serde_json::Value>,
{
    if types.len() != args.len() {
        return Err(UtilError::new(
            "argument_count_mismatch",
            "argument count mismatch",
        ));
    }

    let head_size = types
        .len()
        .checked_mul(32)
        .ok_or_else(|| UtilError::new("overflow", "head size overflow"))?;

    let mut head = Vec::<[u8; 32]>::with_capacity(types.len());
    let mut tail = Vec::<u8>::new();

    for (t, a) in types.iter().zip(args.iter()) {
        let a = a.borrow();
        if is_dynamic_type(t) {
            let off = head_size
                .checked_add(tail.len())
                .ok_or_else(|| UtilError::new("overflow", "offset overflow"))?;
            head.push(encode_len_word(off)?);
            let dyn_enc = encode_dynamic_value(t, a)?;
            tail.extend_from_slice(&dyn_enc);
        } else if t == "address" {
            let Some(s) = a.as_str() else {
                return Err(UtilError::new(
                    "invalid_address",
                    "address arg must be a string",
                ));
            };
            let word = encode_address_word(s)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&word);
            head.push(arr);
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

/// Resolves an overloaded function name against the provided arguments and returns calldata.
///
/// The returned tuple contains the complete calldata and the resolved output type list.
pub fn resolve_function_call<T>(
    abi: &ParsedAbi,
    function: &str,
    args: &[T],
) -> Result<(Vec<u8>, Vec<String>), UtilError>
where
    T: Borrow<serde_json::Value>,
{
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
        out.extend_from_slice(&function_selector(&f.name, &f.inputs));
        out.append(&mut enc);
        winners.push((out, f.outputs.clone()));
    }

    match winners.len() {
        0 => Err(UtilError::new(
            "function_resolution_failed",
            "function resolution failed for provided args",
        )),
        1 => Ok(winners.remove(0)),
        _ => Err(UtilError::new(
            "function_resolution_ambiguous",
            "ambiguous overloaded function for provided args",
        )),
    }
}

/// Appends ABI-encoded constructor arguments to contract bytecode.
pub fn constructor_data<T>(
    abi: &ParsedAbi,
    bytecode: &[u8],
    args: &[T],
) -> Result<Vec<u8>, UtilError>
where
    T: Borrow<serde_json::Value>,
{
    let enc = encode_params(&abi.constructor_inputs, args)?;
    let mut out = Vec::with_capacity(bytecode.len() + enc.len());
    out.extend_from_slice(bytecode);
    out.extend_from_slice(&enc);
    Ok(out)
}

/// Normalizes optional transaction value input from decimal or hex into lowercase hex.
pub fn parse_value_wei_to_hex(value_wei: &Option<String>) -> Result<Option<String>, UtilError> {
    let Some(raw) = value_wei else {
        return Ok(None);
    };
    if raw.starts_with("0x") || raw.starts_with("0X") {
        return Ok(Some(normalize_hex_str(raw)?));
    }
    let v = raw
        .parse::<u128>()
        .map_err(|_| UtilError::new("invalid_value_wei", "value_wei must be decimal or 0x hex"))?;
    Ok(Some(format!("0x{:x}", v)))
}

/// Decodes a single-output `eth_call` response into a JSON value when the type is supported.
///
/// Unsupported output types fall back to the normalized hex string.
pub fn decode_single_output_to_json(
    outputs: &[String],
    raw_hex: &str,
) -> Result<serde_json::Value, UtilError> {
    let normalized = normalize_hex_str(raw_hex)?;
    if outputs.is_empty() || outputs.len() != 1 {
        return Ok(serde_json::json!(normalized));
    }

    let out_t = outputs[0].as_str();
    let b = hex_to_bytes(&normalized)?;
    if b.len() < 32 {
        return Err(UtilError::new(
            "invalid_eth_call_output",
            "eth_call return data too short",
        ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_dynamic_string() {
        let types = vec!["string".to_string()];
        let args = vec![serde_json::json!("hello")];

        let out = encode_params(&types, &args).expect("encode");
        assert_eq!(out.len(), 96);
        assert_eq!(out[31], 32u8);
        assert_eq!(out[63], 5u8);
    }

    #[test]
    fn selector_for_transfer() {
        let selector =
            function_selector("transfer", &["address".to_string(), "uint256".to_string()]);
        assert_eq!(selector, [0xa9, 0x05, 0x9c, 0xbb]);
    }
}
