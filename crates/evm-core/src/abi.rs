//! ABI parsing and calldata construction helpers.
//!
//! This module intentionally keeps the supported surface small: enough to parse compiler ABI JSON,
//! derive selectors, and build common constructor/function call payloads used by higher-level
//! runtime crates.
//!
//! Supported ABI argument types are `address`, `bool`, `bytes1` through `bytes32`, dynamic
//! `bytes`, `string`, and signed/unsigned integer widths from 8 through 256 bits in 8-bit
//! increments. The aliases `int` and `uint` are treated as 256-bit integers. Unsupported ABI
//! types fail explicitly during encoding or function resolution.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AbiIntegerType {
    Signed(usize),
    Unsigned(usize),
}

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
    let sig = function_signature(name, input_types);
    let h = keccak256(sig.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

fn function_signature(name: &str, input_types: &[String]) -> String {
    format!("{}({})", name, input_types.join(","))
}

fn parse_integer_type(t: &str) -> Option<Result<AbiIntegerType, UtilError>> {
    if let Some(width) = t.strip_prefix("uint") {
        return Some(parse_integer_width(width, "uint").map(AbiIntegerType::Unsigned));
    }
    if let Some(width) = t.strip_prefix("int") {
        return Some(parse_integer_width(width, "int").map(AbiIntegerType::Signed));
    }
    None
}

fn parse_integer_width(width: &str, prefix: &'static str) -> Result<usize, UtilError> {
    let bits = if width.is_empty() {
        256
    } else {
        width.parse::<usize>().map_err(|_| {
            UtilError::new(
                "invalid_integer_type",
                format!("{prefix} ABI width must be a decimal integer"),
            )
        })?
    };

    if !(8..=256).contains(&bits) || bits % 8 != 0 {
        return Err(UtilError::new(
            "invalid_integer_type",
            format!("{prefix} ABI width must be a multiple of 8 in [8,256]"),
        ));
    }

    Ok(bits)
}

fn trim_leading_zero_bytes(bytes: &[u8]) -> &[u8] {
    let first_nonzero = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    &bytes[first_nonzero..]
}

fn bit_len_be(bytes: &[u8]) -> usize {
    let bytes = trim_leading_zero_bytes(bytes);
    let Some(first) = bytes.first() else {
        return 0;
    };
    (bytes.len() - 1) * 8 + (8 - first.leading_zeros() as usize)
}

fn u128_fits_bits(value: u128, bits: usize) -> bool {
    if bits >= 128 {
        true
    } else {
        value < (1u128 << bits)
    }
}

fn signed_i128_bounds(bits: usize) -> (i128, i128) {
    if bits >= 128 {
        return (i128::MIN, i128::MAX);
    }
    let max = (1i128 << (bits - 1)) - 1;
    let min = -(1i128 << (bits - 1));
    (min, max)
}

fn encode_u128_word(value: u128) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[16..32].copy_from_slice(&value.to_be_bytes());
    out
}

fn encode_unsigned_bytes_word(bytes: &[u8]) -> [u8; 32] {
    let bytes = trim_leading_zero_bytes(bytes);
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(bytes);
    out
}

fn encode_i128_word(value: i128) -> [u8; 32] {
    let mut out = if value.is_negative() {
        [0xffu8; 32]
    } else {
        [0u8; 32]
    };
    out[16..32].copy_from_slice(&value.to_be_bytes());
    out
}

fn parse_uint_word(bits: usize, v: &serde_json::Value) -> Result<[u8; 32], UtilError> {
    match v {
        serde_json::Value::Number(n) => {
            let Some(u) = n.as_u64() else {
                return Err(UtilError::new(
                    "invalid_uint",
                    "numeric value must fit into u64",
                ));
            };
            let value = u128::from(u);
            if !u128_fits_bits(value, bits) {
                return Err(UtilError::new(
                    "invalid_uint",
                    format!("uint{bits} argument out of bounds"),
                ));
            }
            Ok(encode_u128_word(value))
        }
        serde_json::Value::String(s) => {
            if s.starts_with("0x") || s.starts_with("0X") {
                let b = hex_to_bytes(s)?;
                let trimmed = trim_leading_zero_bytes(&b);
                if trimmed.len() > bits / 8 {
                    return Err(UtilError::new(
                        "invalid_uint",
                        format!("uint{bits} hex integer out of bounds"),
                    ));
                }
                return Ok(encode_unsigned_bytes_word(trimmed));
            }

            let parsed = s.parse::<u128>().map_err(|_| {
                UtilError::new("invalid_uint", "decimal integer must fit into u128")
            })?;
            if !u128_fits_bits(parsed, bits) {
                return Err(UtilError::new(
                    "invalid_uint",
                    format!("uint{bits} argument out of bounds"),
                ));
            }
            Ok(encode_u128_word(parsed))
        }
        _ => Err(UtilError::new(
            "invalid_uint",
            "integer arg must be a number or string",
        )),
    }
}

fn parse_int_word(bits: usize, v: &serde_json::Value) -> Result<[u8; 32], UtilError> {
    match v {
        serde_json::Value::Number(n) => {
            let Some(i) = n.as_i64() else {
                return Err(UtilError::new(
                    "invalid_int",
                    "numeric value must fit into i64",
                ));
            };
            encode_i128_with_bounds(bits, i128::from(i))
        }
        serde_json::Value::String(s) => {
            if s.starts_with("0x") || s.starts_with("0X") {
                let b = hex_to_bytes(s)?;
                if bit_len_be(&b) > bits.saturating_sub(1) {
                    return Err(UtilError::new(
                        "invalid_int",
                        format!("int{bits} positive hex integer out of bounds"),
                    ));
                }
                return Ok(encode_unsigned_bytes_word(&b));
            }

            let parsed = s.parse::<i128>().map_err(|_| {
                UtilError::new("invalid_int", "decimal signed integer must fit into i128")
            })?;
            encode_i128_with_bounds(bits, parsed)
        }
        _ => Err(UtilError::new(
            "invalid_int",
            "integer arg must be a number or string",
        )),
    }
}

fn encode_i128_with_bounds(bits: usize, value: i128) -> Result<[u8; 32], UtilError> {
    let (min, max) = signed_i128_bounds(bits);
    if value < min || value > max {
        return Err(UtilError::new(
            "invalid_int",
            format!("int{bits} argument out of bounds"),
        ));
    }
    Ok(encode_i128_word(value))
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

    if let Some(integer_type) = parse_integer_type(t) {
        return match integer_type? {
            AbiIntegerType::Unsigned(bits) => parse_uint_word(bits, v),
            AbiIntegerType::Signed(bits) => parse_int_word(bits, v),
        };
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
/// signed/unsigned integer widths from 8 through 256 bits, `bool`, and `bytesM`.
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
    let mut failures: Vec<String> = Vec::new();
    let mut candidate_count = 0usize;

    for f in abi
        .functions
        .iter()
        .filter(|f| f.name == function && f.inputs.len() == args.len())
    {
        candidate_count += 1;
        match encode_params(&f.inputs, args) {
            Ok(mut enc) => {
                let mut out = Vec::with_capacity(4 + enc.len());
                out.extend_from_slice(&function_selector(&f.name, &f.inputs));
                out.append(&mut enc);
                winners.push((out, f.outputs.clone()));
            }
            Err(err) => failures.push(format!(
                "{}: {}",
                function_signature(&f.name, &f.inputs),
                err.message
            )),
        }
    }

    match winners.len() {
        0 if candidate_count == 0 => Err(UtilError::new(
            "function_resolution_failed",
            "function resolution failed for provided args",
        )),
        0 => Err(UtilError::new(
            "function_resolution_failed",
            format!(
                "function resolution failed for provided args; candidate errors: {}",
                failures.join("; ")
            ),
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

    fn encode_one(typ: &str, value: serde_json::Value) -> Result<Vec<u8>, UtilError> {
        encode_params(&[typ.to_string()], &[value])
    }

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
    fn encode_dynamic_bytes() {
        let out = encode_one("bytes", serde_json::json!("0xdeadbeef")).expect("encode");
        assert_eq!(out.len(), 96);
        assert_eq!(out[31], 32u8);
        assert_eq!(out[63], 4u8);
        assert_eq!(&out[64..68], &[0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn encode_uint_width_bounds() {
        let max = encode_one("uint8", serde_json::json!(255)).expect("max uint8");
        assert_eq!(max[31], 0xff);

        let with_leading_zero =
            encode_one("uint8", serde_json::json!("0x00ff")).expect("trim leading zero");
        assert_eq!(with_leading_zero[31], 0xff);

        let decimal_err = encode_one("uint8", serde_json::json!(256)).expect_err("overflow");
        assert_eq!(decimal_err.code, "invalid_uint");
        assert!(decimal_err.message.contains("uint8 argument out of bounds"));

        let hex_err = encode_one("uint8", serde_json::json!("0x0100")).expect_err("overflow");
        assert_eq!(hex_err.code, "invalid_uint");
        assert!(hex_err.message.contains("uint8 hex integer out of bounds"));
    }

    #[test]
    fn encode_signed_int_width_bounds() {
        let negative_one = encode_one("int8", serde_json::json!(-1)).expect("negative one");
        assert!(negative_one.iter().all(|byte| *byte == 0xff));

        let min = encode_one("int8", serde_json::json!(-128)).expect("min int8");
        assert_eq!(&min[..31], &[0xff; 31]);
        assert_eq!(min[31], 0x80);

        let max = encode_one("int8", serde_json::json!(127)).expect("max int8");
        assert_eq!(&max[..31], &[0u8; 31]);
        assert_eq!(max[31], 0x7f);

        let low_err = encode_one("int8", serde_json::json!(-129)).expect_err("underflow");
        assert_eq!(low_err.code, "invalid_int");
        assert!(low_err.message.contains("int8 argument out of bounds"));

        let high_err = encode_one("int8", serde_json::json!(128)).expect_err("overflow");
        assert_eq!(high_err.code, "invalid_int");
        assert!(high_err.message.contains("int8 argument out of bounds"));
    }

    #[test]
    fn constructor_data_appends_encoded_args() {
        let abi = parse_abi(&serde_json::json!([
            {
                "type": "constructor",
                "inputs": [
                    { "type": "uint8" },
                    { "type": "string" }
                ]
            }
        ]))
        .expect("abi");
        let bytecode = hex_to_bytes("0x6000").expect("bytecode");
        let args = vec![serde_json::json!(7), serde_json::json!("ready")];

        let out = constructor_data(&abi, &bytecode, &args).expect("constructor data");
        assert_eq!(&out[..2], &[0x60, 0x00]);
        assert_eq!(out[33], 7u8);
        assert_eq!(out[65], 64u8);
        assert_eq!(out[97], 5u8);
        assert_eq!(&out[98..103], b"ready");
    }

    #[test]
    fn resolve_overload_ambiguity_is_explicit() {
        let abi = parse_abi(&serde_json::json!([
            {
                "type": "function",
                "name": "set",
                "inputs": [{ "type": "uint" }],
                "outputs": []
            },
            {
                "type": "function",
                "name": "set",
                "inputs": [{ "type": "uint256" }],
                "outputs": []
            }
        ]))
        .expect("abi");

        let err = resolve_function_call(&abi, "set", &[serde_json::json!(1)])
            .expect_err("ambiguous overload");
        assert_eq!(err.code, "function_resolution_ambiguous");
    }

    #[test]
    fn unsupported_overload_candidate_error_is_deterministic() {
        let abi = parse_abi(&serde_json::json!([
            {
                "type": "function",
                "name": "set",
                "inputs": [{ "type": "tuple" }],
                "outputs": []
            }
        ]))
        .expect("abi");

        let err = resolve_function_call(&abi, "set", &[serde_json::json!({"x": 1})])
            .expect_err("unsupported candidate");
        assert_eq!(err.code, "function_resolution_failed");
        assert!(err
            .message
            .contains("set(tuple): unsupported static ABI type"));
    }

    #[test]
    fn selector_for_transfer() {
        let selector =
            function_selector("transfer", &["address".to_string(), "uint256".to_string()]);
        assert_eq!(selector, [0xa9, 0x05, 0x9c, 0xbb]);
    }
}
