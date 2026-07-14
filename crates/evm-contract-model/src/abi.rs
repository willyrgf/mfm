use super::*;

use alloy_primitives::keccak256;
use mfm_evm_core::{abi as common_abi, hex as common_hex};

/// Parses typed ABI JSON into the shared ABI representation.
pub fn parse_abi(abi: &AbiJson) -> Result<ParsedAbi, String> {
    let abi = abi.to_json_value()?;
    common_abi::parse_abi(&abi).map_err(|error| error.message)
}

/// Resolves and encodes a function call from ABI name plus typed JSON arguments.
pub fn resolve_function_call<T>(
    abi: &ParsedAbi,
    function_name: &str,
    args: &[T],
) -> Result<(Vec<u8>, Vec<String>), String>
where
    T: AsRef<AbiArgumentValue>,
{
    let args = args
        .iter()
        .map(|arg| arg.as_ref().to_json_value())
        .collect::<Result<Vec<_>, _>>()?;
    common_abi::resolve_function_call(abi, function_name, &args).map_err(|error| error.message)
}

/// Encodes constructor bytecode plus typed constructor arguments.
pub fn constructor_data<T>(
    abi: &ParsedAbi,
    bytecode: &[u8],
    constructor_args: &[T],
) -> Result<Vec<u8>, String>
where
    T: AsRef<AbiArgumentValue>,
{
    let constructor_args = constructor_args
        .iter()
        .map(|arg| arg.as_ref().to_json_value())
        .collect::<Result<Vec<_>, _>>()?;
    common_abi::constructor_data(abi, bytecode, &constructor_args).map_err(|error| error.message)
}

/// Parses a contract artifact into validated ABI and bytecode components.
pub fn parse_artifact(config: &ContractArtifactConfig) -> Result<(ParsedAbi, Vec<u8>), String> {
    let abi = parse_abi(&config.abi)?;
    let bytecode_json = config.bytecode.to_json_value()?;
    let bytecode = common_abi::parse_bytecode(&bytecode_json).map_err(|error| error.message)?;
    Ok((abi, bytecode))
}

fn block_selector_to_rpc_value(
    block: &Option<BlockSelector>,
    default_latest: bool,
) -> serde_json::Value {
    match block {
        Some(BlockSelector::Number { number }) => serde_json::json!(format!("0x{:x}", number)),
        Some(BlockSelector::Tag { tag }) => serde_json::json!(tag.as_str()),
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
    for assertion in read_assertions {
        let (call_data, outputs) =
            resolve_function_call(abi, assertion.function.as_str(), &assertion.args)
                .map_err(|_| "read assertion did not match ABI".to_string())?;
        reads.push(PreparedReadAssertion {
            data_hex: common_hex::bytes_to_hex_prefixed(&call_data),
            expected: assertion.expected.clone(),
            outputs,
        });
    }

    let mut events = Vec::with_capacity(event_assertions.len());
    for assertion in event_assertions {
        let event = abi
            .events
            .iter()
            .find(|event| event.name == assertion.event.as_str())
            .ok_or_else(|| "event assertion referenced unknown event".to_string())?;
        if event.anonymous {
            return Err("anonymous events are not supported for validation".to_string());
        }
        let signature = format!("{}({})", event.name, event.inputs.join(","));
        let topic0 = keccak256(signature.as_bytes());
        let topic0_hex = common_hex::bytes_to_hex_prefixed(topic0.as_slice());
        let from_block = block_selector_to_rpc_value(&assertion.from_block, false);
        let to_block = block_selector_to_rpc_value(&assertion.to_block, true);
        events.push(PreparedEventAssertion {
            event: assertion.event.to_string(),
            topic0_hex,
            min_count: assertion.min_count,
            from_block,
            to_block,
        });
    }

    Ok((reads, events))
}

/// Decodes a single-output EVM call response into JSON.
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
pub fn expected_matches(actual: &ExpectedValue, expected: &ExpectedValue) -> bool {
    let Ok(actual) = actual.to_json_value() else {
        return false;
    };
    let Ok(expected) = expected.to_json_value() else {
        return false;
    };

    if expected == actual {
        return true;
    }

    match (&actual, &expected) {
        (Value::String(actual), Value::String(expected))
            if actual.starts_with("0x") && expected.starts_with("0x") =>
        {
            common_hex::normalize_hex_str(actual).ok()
                == common_hex::normalize_hex_str(expected).ok()
        }
        _ => false,
    }
}
