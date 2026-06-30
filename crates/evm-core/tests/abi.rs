use mfm_evm_core::abi::*;
use mfm_evm_core::hex::hex_to_bytes;
use mfm_evm_core::util_error::UtilError;

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
    let selector = function_selector("transfer", &["address".to_string(), "uint256".to_string()]);
    assert_eq!(selector, [0xa9, 0x05, 0x9c, 0xbb]);
}
