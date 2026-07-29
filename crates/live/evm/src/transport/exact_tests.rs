use super::*;

const HASH: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const OTHER_HASH: &str = "0x3333333333333333333333333333333333333333333333333333333333333333";
const ADDRESS: &str = "0x1111111111111111111111111111111111111111";
const OTHER_ADDRESS: &str = "0x4444444444444444444444444444444444444444";

fn result_envelope(result: &str) -> Vec<u8> {
    format!(r#"{{"jsonrpc":"2.0","id":1,"result":{result}}}"#).into_bytes()
}

fn error_envelope(code: i64, message: &str, data: Option<&str>) -> Vec<u8> {
    let data = data.map_or_else(String::new, |value| format!(r#","data":{value}"#));
    format!(r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":{code},"message":"{message}"{data}}}}}"#)
        .into_bytes()
}

fn block(extra: &str) -> String {
    format!(r#"{{"number":"0x1","hash":"{HASH}"{extra}}}"#)
}

fn nested_arrays(depth: usize) -> String {
    format!("{}0{}", "[".repeat(depth), "]".repeat(depth))
}

fn array_with_items(count: usize) -> String {
    format!("[{}]", vec!["0"; count].join(","))
}

#[test]
fn broadcast_body_bound_and_formula_are_exact() {
    assert_eq!(
        REQUEST_PREFIX.len() + "eth_sendRawTransaction".len() + PARAMS_PREFIX.len() + 6 + 1,
        74
    );
    assert_eq!(74 + 2 * EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES, 1_048_650);
    assert_eq!(
        signed_transaction_params_len(EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES),
        Ok(6 + 2 * EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES)
    );
    assert_eq!(
        signed_transaction_params_len(EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES + 1),
        Err(())
    );
}

#[test]
fn wallet_operations_use_the_shared_descriptor_method_contract() {
    assert_eq!(
        ExactRpcOperation::SendRawTransaction.method(),
        EVM_SEND_RAW_TRANSACTION_METHOD
    );
    assert_eq!(
        ExactRpcOperation::TransactionByHash.method(),
        EVM_TRANSACTION_BY_HASH_METHOD
    );
    assert_eq!(
        ExactRpcOperation::ReceiptByHash.method(),
        EVM_RECEIPT_BY_HASH_METHOD
    );
    assert_eq!(
        ExactRpcOperation::FinalizedHead.method(),
        EVM_BLOCK_BY_NUMBER_METHOD
    );
    assert_eq!(
        ExactRpcOperation::InclusionBlock.method(),
        EVM_BLOCK_BY_NUMBER_METHOD
    );
}

#[test]
fn fixed_allowlist_request_encoding_is_exact() {
    let account = Address::from([0x11; 20]);
    let contract = Address::from([0x44; 20]);
    let hash = B256::from([0x22; 32]);
    let cases = [
            (
                ExactRpcRequest::ChainIdentity,
                r#"{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}"#.to_owned(),
            ),
            (
                ExactRpcRequest::LatestAnchor,
                r#"{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["latest",false]}"#
                    .to_owned(),
            ),
            (
                ExactRpcRequest::NativeBalance {
                    account,
                    block_hash: hash,
                },
                format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["{ADDRESS}",{{"blockHash":"{HASH}","requireCanonical":true}}]}}"#
                ),
            ),
            (
                ExactRpcRequest::TokenDecimals {
                    contract,
                    block_hash: hash,
                },
                format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{{"to":"{OTHER_ADDRESS}","data":"0x313ce567"}},{{"blockHash":"{HASH}","requireCanonical":true}}]}}"#
                ),
            ),
            (
                ExactRpcRequest::TokenBalance {
                    contract,
                    account,
                    block_hash: hash,
                },
                format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{{"to":"{OTHER_ADDRESS}","data":"0x70a08231000000000000000000000000{}"}},{{"blockHash":"{HASH}","requireCanonical":true}}]}}"#,
                    &ADDRESS[2..]
                ),
            ),
            (
                ExactRpcRequest::ConfirmAnchor { number: U256::ZERO },
                r#"{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["0x0",false]}"#
                    .to_owned(),
            ),
            (
                ExactRpcRequest::TransactionByHash {
                    transaction_hash: hash,
                },
                format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"eth_getTransactionByHash","params":["{HASH}"]}}"#
                ),
            ),
            (
                ExactRpcRequest::ReceiptByHash {
                    transaction_hash: hash,
                },
                format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"eth_getTransactionReceipt","params":["{HASH}"]}}"#
                ),
            ),
            (
                ExactRpcRequest::FinalizedHead,
                r#"{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["finalized",false]}"#
                    .to_owned(),
            ),
            (
                ExactRpcRequest::InclusionBlock {
                    number: U256::from(0x2a),
                },
                r#"{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["0x2a",false]}"#
                    .to_owned(),
            ),
        ];
    for (request, expected) in cases {
        let encoded = request.encode().expect("allowlisted request");
        assert_eq!(encoded.body.as_slice(), expected.as_bytes());
        assert_eq!(encoded.body.len(), encoded.body.capacity());
    }

    let zero = ExactRpcRequest::ConfirmAnchor { number: U256::ZERO }
        .encode()
        .expect("zero quantity request");
    assert_eq!(
        zero.body.as_slice(),
        br#"{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["0x0",false]}"#
    );

    let nonzero = ExactRpcRequest::InclusionBlock {
        number: U256::from(0x2a),
    }
    .encode()
    .expect("nonzero quantity request");
    assert_eq!(
        nonzero.body.as_slice(),
        br#"{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["0x2a",false]}"#
    );
}

#[test]
fn envelope_shape_keys_and_trailing_input_are_exact() {
    assert!(matches!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            br#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#
        ),
        Ok(ExactRpcResponse::ChainIdentity(1))
    ));
    for malformed in [
        br#"[]"#.as_slice(),
        br#"[{"jsonrpc":"2.0","id":1,"result":"0x1"}]"#,
        b"\xef\xbb\xbf{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"0x1\"}",
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"\xff\"}",
        b"",
        b"{",
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"0x1\"",
        br#"{"jsonrpc":"2.0","id":1,"result":"0x1"} trailing"#.as_slice(),
        br#"{"jsonrpc":"2.0","id":1,"result":"0x1","extra":0}"#,
        br#"{"jsonrpc":"2.0","id":2,"result":"0x1"}"#,
        br#"{"jsonrpc":"2\u002e0","id":1,"result":"0x1"}"#,
        br#"{"jsonrpc":"2.0","id":1,"result":"0x1","error":{"code":1,"message":"x"}}"#,
        br#"{"jsonrpc":"2.0","id":1,"result":"0x1","res\u0075lt":"0x1"}"#,
    ] {
        assert_eq!(
            decode_response(ExactRpcOperation::ChainIdentity, malformed),
            Err(DecodeFailure::MalformedEnvelope)
        );
    }
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            br#"{"jsonrpc":"2.0","id":1}"#
        ),
        Err(DecodeFailure::MissingResult)
    );
}

#[test]
fn envelope_uses_only_exact_json_whitespace() {
    assert!(matches!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            b" \t\r\n{\"jsonrpc\" \t:\n\"2.0\",\r\"id\":1,\"result\" : \"0x1\"}\t\r\n "
        ),
        Ok(ExactRpcResponse::ChainIdentity(1))
    ));
    for malformed in [
        b"\x0b{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"0x1\"}".as_slice(),
        b"{\"jsonrpc\"\x0c:\"2.0\",\"id\":1,\"result\":\"0x1\"}",
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\x0b\"0x1\"}",
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":1,\x0c\"message\":\"x\"}}",
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"0x1\"}\x0b",
    ] {
        assert_eq!(
            decode_response(ExactRpcOperation::ChainIdentity, malformed),
            Err(DecodeFailure::MalformedEnvelope)
        );
    }
}

#[test]
fn already_known_error_is_exact_after_bounded_string_decoding() {
    for message in ["already known", r"already kn\u006fwn"] {
        assert!(matches!(
            decode_response(
                ExactRpcOperation::SendRawTransaction,
                &error_envelope(-32_000, message, None)
            ),
            Ok(ExactRpcResponse::SendRawTransaction(
                WalletBroadcastResponse::AlreadyKnown
            ))
        ));
    }
    for (code, message, data) in [
        (-32_000, "already Known", None),
        (-32_001, "already known", None),
        (-32_000, "already known", Some("null")),
    ] {
        assert_eq!(
            decode_response(
                ExactRpcOperation::SendRawTransaction,
                &error_envelope(code, message, data)
            ),
            Err(DecodeFailure::JsonRpcError(code))
        );
    }
    assert_eq!(
            decode_response(
                ExactRpcOperation::SendRawTransaction,
                br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"already known","mess\u0061ge":"already known"}}"#
            ),
            Err(DecodeFailure::MalformedEnvelope)
        );
    assert_eq!(
            decode_response(
                ExactRpcOperation::SendRawTransaction,
                br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"already known","unknown":0}}"#
            ),
            Err(DecodeFailure::MalformedEnvelope)
        );
    for malformed_message in [r"\ud800", r"invalid\q"] {
        assert_eq!(
            decode_response(
                ExactRpcOperation::SendRawTransaction,
                &error_envelope(-32_000, malformed_message, None)
            ),
            Err(DecodeFailure::MalformedEnvelope)
        );
    }
}

#[test]
fn error_message_and_projection_bounds_are_exact() {
    let exact_message = "a".repeat(JSON_RPC_ERROR_MESSAGE_MAX_BYTES);
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            &error_envelope(-32_001, &exact_message, None)
        ),
        Err(DecodeFailure::JsonRpcError(-32_001))
    );
    let too_long_message = "a".repeat(JSON_RPC_ERROR_MESSAGE_MAX_BYTES + 1);
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            &error_envelope(-32_001, &too_long_message, None)
        ),
        Err(DecodeFailure::MalformedEnvelope)
    );

    let mut exact_encoded = r"\u0061".repeat(JSON_RPC_ERROR_MESSAGE_MAX_BYTES / 6);
    exact_encoded.push_str(&"a".repeat(JSON_RPC_ERROR_MESSAGE_MAX_BYTES - exact_encoded.len()));
    assert_eq!(exact_encoded.len(), JSON_RPC_ERROR_MESSAGE_MAX_BYTES);
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            &error_envelope(-32_001, &exact_encoded, None)
        ),
        Err(DecodeFailure::JsonRpcError(-32_001))
    );
    exact_encoded.push('a');
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            &error_envelope(-32_001, &exact_encoded, None)
        ),
        Err(DecodeFailure::MalformedEnvelope)
    );

    let exact_data = array_with_items(PROJECTION_MAX_CONTAINER_ITEMS);
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            &error_envelope(-32_001, "provider text", Some(&exact_data))
        ),
        Err(DecodeFailure::JsonRpcError(-32_001))
    );
    let excessive_data = array_with_items(PROJECTION_MAX_CONTAINER_ITEMS + 1);
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            &error_envelope(-32_001, "provider text", Some(&excessive_data))
        ),
        Err(DecodeFailure::MalformedEnvelope)
    );
}

#[test]
fn semantic_keys_are_bounded_zeroizing_and_duplicate_checked() {
    let exact_key = "k".repeat(JSON_OBJECT_KEY_MAX_BYTES);
    assert!(RawObject::parse(format!(r#"{{"{exact_key}":0}}"#).as_bytes()).is_ok());
    let excessive_key = "k".repeat(JSON_OBJECT_KEY_MAX_BYTES + 1);
    assert!(RawObject::parse(format!(r#"{{"{excessive_key}":0}}"#).as_bytes()).is_err());
    let mut exact_encoded_key = r"\u006b".repeat(JSON_OBJECT_KEY_MAX_BYTES / 6);
    exact_encoded_key.push_str(&"k".repeat(JSON_OBJECT_KEY_MAX_BYTES - exact_encoded_key.len()));
    assert!(RawObject::parse(format!(r#"{{"{exact_encoded_key}":0}}"#).as_bytes()).is_ok());
    exact_encoded_key.push('k');
    assert!(RawObject::parse(format!(r#"{{"{exact_encoded_key}":0}}"#).as_bytes()).is_err());

    for duplicate in [
        br#"{"hash":0,"h\u0061sh":1}"#.as_slice(),
        "{\"😀\":0,\"\\ud83d\\ude00\":1}".as_bytes(),
    ] {
        assert!(RawObject::parse(duplicate).is_err());
    }
    for malformed in [br#"{"\ud800":0}"#.as_slice(), br#"{"bad\q":0}"#] {
        assert!(RawObject::parse(malformed).is_err());
    }
}

#[test]
fn read_result_size_precedes_bounded_projection_classification() {
    let exact = format!(
        "\"{}\"",
        "x".repeat(EVM_READ_MAX_RESPONSE_BYTES.saturating_sub(2))
    );
    assert_eq!(exact.len(), EVM_READ_MAX_RESPONSE_BYTES);
    assert_eq!(
        decode_response(ExactRpcOperation::ChainIdentity, &result_envelope(&exact)),
        Err(DecodeFailure::InvalidResult)
    );

    let excessive = format!("\"{}\"", "x".repeat(EVM_READ_MAX_RESPONSE_BYTES - 1));
    assert_eq!(excessive.len(), EVM_READ_MAX_RESPONSE_BYTES + 1);
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            &result_envelope(&excessive)
        ),
        Err(DecodeFailure::ResultTooLarge(
            EVM_READ_MAX_RESPONSE_BYTES + 1
        ))
    );
}

#[test]
fn result_projection_depth_and_container_bounds_map_to_invalid_result() {
    let exact_items = array_with_items(PROJECTION_MAX_CONTAINER_ITEMS);
    assert!(matches!(
        decode_response(
            ExactRpcOperation::LatestAnchor,
            &result_envelope(&block(&format!(r#","extra":{exact_items}"#)))
        ),
        Ok(ExactRpcResponse::LatestAnchor(_))
    ));
    let excessive_items = array_with_items(PROJECTION_MAX_CONTAINER_ITEMS + 1);
    assert_eq!(
        decode_response(
            ExactRpcOperation::LatestAnchor,
            &result_envelope(&block(&format!(r#","extra":{excessive_items}"#)))
        ),
        Err(DecodeFailure::InvalidResult)
    );

    let exact_depth = nested_arrays(PROJECTION_MAX_DEPTH - 1);
    assert!(matches!(
        decode_response(
            ExactRpcOperation::LatestAnchor,
            &result_envelope(&block(&format!(r#","extra":{exact_depth}"#)))
        ),
        Ok(ExactRpcResponse::LatestAnchor(_))
    ));
    let excessive_depth = nested_arrays(PROJECTION_MAX_DEPTH);
    assert_eq!(
        decode_response(
            ExactRpcOperation::LatestAnchor,
            &result_envelope(&block(&format!(r#","extra":{excessive_depth}"#)))
        ),
        Err(DecodeFailure::InvalidResult)
    );
}

#[test]
fn envelope_range_discovery_is_stack_safe_for_far_deep_values() {
    let deep_read_result = nested_arrays(50_000);
    assert!(deep_read_result.len() < EVM_READ_MAX_RESPONSE_BYTES);
    assert_eq!(
        decode_response(
            ExactRpcOperation::LatestAnchor,
            &result_envelope(&deep_read_result)
        ),
        Err(DecodeFailure::InvalidResult)
    );

    let deep_wallet_result = nested_arrays(100_000);
    assert_eq!(
        decode_response(
            ExactRpcOperation::SendRawTransaction,
            &result_envelope(&deep_wallet_result)
        ),
        Err(DecodeFailure::InvalidResult)
    );
    assert_eq!(
        decode_response(
            ExactRpcOperation::ChainIdentity,
            &error_envelope(-32_001, "provider text", Some(&deep_wallet_result))
        ),
        Err(DecodeFailure::MalformedEnvelope)
    );
}

#[test]
fn nested_unknown_members_are_accepted_but_semantic_duplicates_are_rejected() {
    let accepted =
        block(r#","extra":{"inner":[true,null,{"note":"ignored","nested":{"value":1}}]}"#);
    assert!(matches!(
        decode_response(ExactRpcOperation::LatestAnchor, &result_envelope(&accepted)),
        Ok(ExactRpcResponse::LatestAnchor(_))
    ));
    let duplicate = block(r#","extra":{"inner":{"hash":0,"h\u0061sh":1}}"#);
    assert_eq!(
        decode_response(
            ExactRpcOperation::LatestAnchor,
            &result_envelope(&duplicate)
        ),
        Err(DecodeFailure::InvalidResult)
    );
}

#[test]
fn transaction_and_receipt_decoders_enforce_canonical_coherent_fields() {
    let transaction = format!(
        r#"{{"accessList":[{{"address":"{ADDRESS}","storageKeys":["{HASH}"]}}],"blockHash":null,"blockNumber":null,"chainId":"0x1","from":"{ADDRESS}","gas":"0x124f8","hash":"{HASH}","input":"0xdead","maxFeePerGas":"0x14","maxPriorityFeePerGas":"0x2","nonce":"0x7","to":"{OTHER_ADDRESS}","transactionIndex":null,"type":"0x2","value":"0x5"}}"#
    );
    assert!(matches!(
        decode_response(
            ExactRpcOperation::TransactionByHash,
            &result_envelope(&transaction)
        ),
        Ok(ExactRpcResponse::TransactionByHash(Some(_)))
    ));
    assert_eq!(
        decode_response(
            ExactRpcOperation::TransactionByHash,
            &result_envelope(&transaction.replace(r#""nonce":"0x7""#, r#""nonce":"0x07""#))
        ),
        Err(DecodeFailure::InvalidResult)
    );

    let log = format!(
        r#"{{"address":"{OTHER_ADDRESS}","blockHash":"{OTHER_HASH}","blockNumber":"0x64","data":"0x","logIndex":"0x0","removed":false,"topics":[],"transactionHash":"{HASH}","transactionIndex":"0x2"}}"#
    );
    let receipt = format!(
        r#"{{"blockHash":"{OTHER_HASH}","blockNumber":"0x64","contractAddress":null,"cumulativeGasUsed":"0xa410","from":"{ADDRESS}","gasUsed":"0xa410","logs":[{log}],"status":"0x1","to":"{OTHER_ADDRESS}","transactionHash":"{HASH}","transactionIndex":"0x2","type":"0x2"}}"#
    );
    assert!(matches!(
        decode_response(ExactRpcOperation::ReceiptByHash, &result_envelope(&receipt)),
        Ok(ExactRpcResponse::ReceiptByHash(Some(_)))
    ));
    let absent_type = receipt.replace(r#","type":"0x2""#, "");
    assert!(matches!(
        decode_response(
            ExactRpcOperation::ReceiptByHash,
            &result_envelope(&absent_type)
        ),
        Ok(ExactRpcResponse::ReceiptByHash(Some(_)))
    ));
    for invalid in [
        receipt.replace(r#""type":"0x2""#, r#""type":"0x1""#),
        receipt.replace(r#""type":"0x2""#, r#""type":"0x02""#),
        receipt.replacen(
            &format!(r#""blockHash":"{OTHER_HASH}""#),
            &format!(r#""blockHash":"{HASH}""#),
            1,
        ),
    ] {
        assert_eq!(
            decode_response(ExactRpcOperation::ReceiptByHash, &result_envelope(&invalid)),
            Err(DecodeFailure::InvalidResult)
        );
    }
}
