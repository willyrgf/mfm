use super::*;

fn id(value: &str) -> LocalPublicId {
    LocalPublicId::new(value).expect("test id")
}

#[test]
fn diagnostic_renders_stable_code_summary_and_public_details() {
    let diagnostic =
        RedactedProviderDiagnostic::new(id("bitcoin"), ProviderDiagnosticCode::RpcHttpStatus)
            .with_operation(id("scantxoutset"))
            .with_field(id("http_status"), ProviderDiagnosticValue::U64(403));

    assert_eq!(diagnostic.stable_error_code(), "bitcoin_rpc_http_status");
    assert_eq!(
        diagnostic.summary(),
        "rpc_http_status operation=scantxoutset http_status=403"
    );
    assert_eq!(
        serde_json::to_value(&diagnostic).expect("serialize diagnostic"),
        serde_json::json!({
            "provider_family": "bitcoin",
            "code": "rpc_http_status",
            "operation": "scantxoutset",
            "fields": {
                "http_status": 403,
            },
        })
    );
}

#[test]
fn serde_round_trip_uses_the_closed_canonical_representation() {
    let diagnostic =
        RedactedProviderDiagnostic::new(id("evm"), ProviderDiagnosticCode::SourceMismatch)
            .with_operation(id("chain_id"))
            .with_field(id("expected_chain_id"), ProviderDiagnosticValue::U64(1));

    assert_eq!(
        serde_json::from_value::<RedactedProviderDiagnostic>(
            serde_json::to_value(&diagnostic).expect("serialize diagnostic")
        )
        .expect("deserialize diagnostic"),
        diagnostic
    );
}

#[test]
fn public_details_parser_rejects_unknown_or_untyped_fields() {
    let unknown_field = serde_json::json!({
        "provider_family": "evm",
        "code": "response_invalid",
        "operation": null,
        "fields": {},
        "unexpected": true,
    });
    assert!(serde_json::from_value::<RedactedProviderDiagnostic>(unknown_field).is_err());

    let untyped_field = serde_json::json!({
        "provider_family": "evm",
        "code": "response_invalid",
        "operation": null,
        "fields": {"body": {"secret": "value"}},
    });
    assert!(serde_json::from_value::<RedactedProviderDiagnostic>(untyped_field).is_err());
}

#[test]
fn diagnostic_shape_has_no_raw_string_escape_hatch() {
    let diagnostic =
        RedactedProviderDiagnostic::new(id("evm"), ProviderDiagnosticCode::TransportFailed)
            .with_operation(id("eth_chain_id"))
            .with_field(id("retryable"), ProviderDiagnosticValue::Bool(false));
    let rendered = format!("{diagnostic:?} {}", diagnostic.summary());

    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains("authorization"));
    assert!(!rendered.contains("token="));
}
