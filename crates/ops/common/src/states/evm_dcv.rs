use serde::Deserialize;

use alloy_primitives::keccak256;

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
pub struct AbiFunction {
    pub name: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AbiEvent {
    pub name: String,
    pub inputs: Vec<String>,
    pub anonymous: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ParsedAbi {
    pub constructor_inputs: Vec<String>,
    pub functions: Vec<AbiFunction>,
    pub events: Vec<AbiEvent>,
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

pub fn parse_abi(abi: &serde_json::Value) -> Result<ParsedAbi, String> {
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

pub fn normalize_hex_str(s: &str) -> Result<String, String> {
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

pub fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    let n = normalize_hex_str(s)?;
    let rest = n.strip_prefix("0x").expect("prefix");
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    hex::decode(rest).map_err(|_| "invalid hex".to_string())
}

pub fn bytes_to_hex_prefixed(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn encode_address_word(addr: &str) -> Result<Vec<u8>, String> {
    let n = normalize_hex_str(addr)?;
    let rest = n.strip_prefix("0x").unwrap_or_default();
    if rest.len() != 40 {
        return Err("address must be 20 bytes".to_string());
    }
    let mut out = vec![0u8; 32];
    for (i, chunk) in rest.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0]).ok_or_else(|| "invalid address hex".to_string())?;
        let lo = hex_nibble(chunk[1]).ok_or_else(|| "invalid address hex".to_string())?;
        out[12 + i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn encode_uint256_word(v: &serde_json::Value) -> Result<Vec<u8>, String> {
    let s = if let Some(n) = v.as_u64() {
        n.to_string()
    } else if let Some(s) = v.as_str() {
        s.to_string()
    } else {
        return Err("uint256 arg must be string/number".to_string());
    };

    let num = if s.starts_with("0x") {
        let n = normalize_hex_str(&s)?;
        let rest = n.strip_prefix("0x").unwrap_or_default();
        if rest.is_empty() {
            vec![]
        } else {
            hex::decode(rest).map_err(|_| "invalid uint256 hex".to_string())?
        }
    } else {
        // Decimal parse through U256 via hex fallback.
        let dec = s
            .parse::<u128>()
            .map_err(|_| "invalid uint256".to_string())?;
        let mut tmp = dec;
        let mut be = vec![];
        while tmp > 0 {
            be.push((tmp & 0xff) as u8);
            tmp >>= 8;
        }
        be.reverse();
        be
    };

    if num.len() > 32 {
        return Err("uint256 too large".to_string());
    }
    let mut out = vec![0u8; 32];
    out[32 - num.len()..].copy_from_slice(&num);
    Ok(out)
}

fn encode_params(types: &[String], args: &[serde_json::Value]) -> Result<Vec<u8>, String> {
    if types.len() != args.len() {
        return Err("argument count does not match ABI".to_string());
    }

    let mut out = Vec::with_capacity(32 * types.len());
    for (typ, arg) in types.iter().zip(args.iter()) {
        match typ.as_str() {
            "address" => out.extend_from_slice(&encode_address_word(
                arg.as_str()
                    .ok_or_else(|| "address arg must be string".to_string())?,
            )?),
            "uint256" | "uint" => out.extend_from_slice(&encode_uint256_word(arg)?),
            "bool" => {
                let b = arg
                    .as_bool()
                    .ok_or_else(|| "bool arg must be bool".to_string())?;
                let mut w = vec![0u8; 32];
                if b {
                    w[31] = 1;
                }
                out.extend_from_slice(&w);
            }
            _ => return Err(format!("unsupported ABI type: {typ}")),
        }
    }
    Ok(out)
}

fn function_selector(signature: &str) -> [u8; 4] {
    let h = keccak256(signature.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

fn encode_function_call(
    function: &AbiFunction,
    args: &[serde_json::Value],
) -> Result<Vec<u8>, String> {
    let sig = format!("{}({})", function.name, function.inputs.join(","));
    let selector = function_selector(&sig);
    let encoded = encode_params(&function.inputs, args)?;
    let mut out = Vec::with_capacity(4 + encoded.len());
    out.extend_from_slice(&selector);
    out.extend_from_slice(&encoded);
    Ok(out)
}

pub fn resolve_function_call(
    abi: &ParsedAbi,
    function_name: &str,
    args: &[serde_json::Value],
) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut candidates = abi
        .functions
        .iter()
        .filter(|f| f.name == function_name)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(format!("function not found in ABI: {function_name}"));
    }
    candidates.sort_by_key(|f| f.inputs.len());

    for f in candidates {
        if f.inputs.len() == args.len() {
            let data = encode_function_call(f, args)?;
            return Ok((data, f.outputs.clone()));
        }
    }

    Err(format!("function args do not match ABI: {function_name}"))
}

pub fn constructor_data(
    abi: &ParsedAbi,
    bytecode: &[u8],
    constructor_args: &[serde_json::Value],
) -> Result<Vec<u8>, String> {
    let encoded = encode_params(&abi.constructor_inputs, constructor_args)?;
    let mut out = Vec::with_capacity(bytecode.len() + encoded.len());
    out.extend_from_slice(bytecode);
    out.extend_from_slice(&encoded);
    Ok(out)
}

pub fn parse_artifact(cfg: &ContractArtifactConfig) -> Result<(ParsedAbi, Vec<u8>), String> {
    let abi = parse_abi(&cfg.abi)?;
    let bytecode = hex_to_bytes(
        cfg.bytecode
            .as_str()
            .ok_or_else(|| "artifact.bytecode must be hex string".to_string())?,
    )?;
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
    let Some(raw) = value_wei else {
        return Ok(None);
    };
    if raw.starts_with("0x") {
        return Ok(Some(normalize_hex_str(raw)?));
    }
    let v = raw
        .parse::<u128>()
        .map_err(|_| "value_wei must be decimal or 0x hex".to_string())?;
    Ok(Some(format!("0x{:x}", v)))
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
    let n = normalize_hex_str(s)?;
    let rest = n.strip_prefix("0x").unwrap_or_default();
    if rest.len() != 40 {
        return Err("address must be 20 bytes".to_string());
    }
    Ok(format!("0x{}", rest.to_ascii_lowercase()))
}

pub fn decode_single_output_to_json(
    outputs: &[String],
    raw_hex: &str,
) -> Result<serde_json::Value, String> {
    let bytes = hex_to_bytes(raw_hex)?;
    if outputs.len() != 1 {
        return Err("only single-output assertions are supported".to_string());
    }
    if bytes.len() < 32 {
        return Err("eth_call return data too short".to_string());
    }
    let out_type = outputs[0].as_str();
    let word = &bytes[bytes.len() - 32..];

    match out_type {
        "bool" => Ok(serde_json::json!(word[31] == 1u8)),
        "address" => Ok(serde_json::json!(format!("0x{}", hex::encode(&word[12..])))),
        "uint256" | "uint" => Ok(serde_json::json!(format!("0x{}", hex::encode(word)))),
        _ => Err(format!("unsupported output type for assertion: {out_type}")),
    }
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
