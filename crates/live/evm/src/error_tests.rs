use super::*;
#[test]
fn existing_owner_errors_disclose_their_upstream_gap() {
    let fields = [
        serde_json::to_value(EvmCodecError::Invalid).unwrap(),
        serde_json::to_value(mfm_signing::SigningError::Failed).unwrap(),
        serde_json::to_value(mfm_evm::custody::AuthorityError::Internal).unwrap(),
    ];
    for (fields, kind) in fields.iter().zip(["invalid", "failed", "internal"]) {
        assert_eq!(fields["kind"], kind);
        assert_eq!(
            fields["upstream_detail"],
            "unavailable_at_existing_owner_boundary"
        );
    }
}

#[test]
fn block_tag_projection_keeps_both_native_parse_layers_and_reviewed_numeric_facts() {
    use std::error::Error;
    let error = AdapterFailure::BlockTag {
        source: ParseError::BaseConvertError(BaseConvertError::InvalidDigit(17, 10)),
    };
    let source = error.source().unwrap();
    assert!(matches!(
        source.downcast_ref::<ParseError>(),
        Some(ParseError::BaseConvertError(_))
    ));
    assert!(matches!(
        source.source().unwrap().downcast_ref::<BaseConvertError>(),
        Some(BaseConvertError::InvalidDigit(17, 10))
    ));
    let projected = serde_json::to_value(&error).unwrap();
    assert_eq!(
        projected["block_tag"]["source"]["base_convert"]["source"]["invalid_digit"]["digit"],
        17
    );
    assert_eq!(
        projected["block_tag"]["source"]["base_convert"]["source"]["invalid_digit"]["base"],
        10
    );
}

#[tokio::test]
async fn task_extraction_records_cancellation_and_explicitly_withholds_panic_payloads() {
    let pending = tokio::spawn(std::future::pending::<()>());
    pending.abort();
    let cancelled = AdapterFailure::task(TaskOperation::SigningDigest, pending.await.unwrap_err());
    let projected = serde_json::to_value(cancelled).unwrap();
    assert_eq!(projected["task"]["outcome"], "cancelled");
    assert_eq!(projected["task"]["panic_payload"], "not_present");

    // A non-string payload avoids the default panic hook rendering custody-shaped fixture bytes.
    let panicked = tokio::task::spawn_blocking(|| std::panic::panic_any(vec![0xa5_u8; 32]));
    let error = AdapterFailure::task(
        TaskOperation::EncodeSignedTransaction,
        panicked.await.unwrap_err(),
    );
    let projected = serde_json::to_value(error).unwrap();
    assert_eq!(projected["task"]["operation"], "encode_signed_transaction");
    assert_eq!(projected["task"]["outcome"], "panicked");
    assert_eq!(projected["task"]["panic_payload"], "withheld");
    assert!(projected["task"].get("source").is_none());
}
