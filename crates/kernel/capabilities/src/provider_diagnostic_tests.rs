use super::*;

const PROTOTYPE_MAX_REVIEWED_DIAGNOSTIC_BYTES: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeEvmStableFailureCode {
    RoutingGenerationUnavailable,
    ConfigurationInvalid,
    RequestInvalid,
    AccessCancelled,
    TransportFailed,
    HttpStatus,
    JsonRpcError,
    ResponseInvalid,
    ResponseMissingResult,
    ResponseTooLarge,
    UnclassifiedFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeBitcoinStableFailureCode {
    RoutingGenerationUnavailable,
    ConfigurationInvalid,
    RequestInvalid,
    AccessCancelled,
    TransportFailed,
    HttpStatus,
    JsonRpcError,
    ResponseInvalid,
    ResponseMissingResult,
    ResponseTooLarge,
    UnclassifiedFailure,
    ScanBusy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeFailureClass {
    Authorization,
    Configuration,
    Request,
    Cancellation,
    Transport,
    Destination,
    UnrepresentableResponse,
    Integrity,
    ResourceConflict,
    Unclassified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeBoundaryStage {
    BeforeBoundaryEntry,
    BoundaryEntry,
    BoundaryObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeCoarseSizeClass {
    Zero,
    UpTo16Kib,
    UpTo1Mib,
    Over1Mib,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeSafeFailure<StableCode, Diagnostic> {
    stable_code: StableCode,
    failure_class: PrototypeFailureClass,
    boundary_stage: PrototypeBoundaryStage,
    coarse_size_class: Option<PrototypeCoarseSizeClass>,
    diagnostic_ref: Option<PrototypeReviewedDiagnosticRef<Diagnostic>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeEvmDiagnostic {
    HttpStatus { status: u16 },
    JsonRpcError { code: i64 },
    ResponseInvalid { kind: PrototypeResponseFailureKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeBitcoinDiagnostic {
    HttpStatus { status: u16 },
    JsonRpcError { code: i64 },
    ScanBusy,
    ResponseInvalid { kind: PrototypeResponseFailureKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeQualifiedDiagnostic {
    Evm(PrototypeEvmDiagnostic),
    Bitcoin(PrototypeBitcoinDiagnostic),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeResponseFailureKind {
    MalformedEnvelope,
    MissingResult,
    InvalidResult,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeReviewedDiagnosticRef<Diagnostic>(Diagnostic);

fn prototype_diagnostic_ref<Diagnostic>(
    diagnostic: Diagnostic,
    reviewed_diagnostic_bytes: usize,
) -> Option<PrototypeReviewedDiagnosticRef<Diagnostic>> {
    (reviewed_diagnostic_bytes <= PROTOTYPE_MAX_REVIEWED_DIAGNOSTIC_BYTES)
        .then_some(PrototypeReviewedDiagnosticRef(diagnostic))
}

fn prototype_coarse_size_class(observed_response_bytes: usize) -> PrototypeCoarseSizeClass {
    match observed_response_bytes {
        0 => PrototypeCoarseSizeClass::Zero,
        1..=16_384 => PrototypeCoarseSizeClass::UpTo16Kib,
        16_385..=1_048_576 => PrototypeCoarseSizeClass::UpTo1Mib,
        _ => PrototypeCoarseSizeClass::Over1Mib,
    }
}

fn prototype_qualify_generic_diagnostic(
    diagnostic: &RedactedProviderDiagnostic,
) -> Option<PrototypeQualifiedDiagnostic> {
    let operation = diagnostic.operation()?.as_str();
    let fields = diagnostic.fields();
    if fields.len() != 1 {
        return None;
    }
    match (
        diagnostic.provider_family().as_str(),
        diagnostic.code(),
        operation,
    ) {
        ("evm", ProviderDiagnosticCode::RpcHttpStatus, "eth_chain_id") => {
            match fields.get(&id("http_status"))? {
                ProviderDiagnosticValue::U64(status) => u16::try_from(*status).ok().map(|status| {
                    PrototypeQualifiedDiagnostic::Evm(PrototypeEvmDiagnostic::HttpStatus { status })
                }),
                _ => None,
            }
        }
        ("evm", ProviderDiagnosticCode::RpcJsonError, "eth_chain_id") => {
            match fields.get(&id("rpc_code"))? {
                ProviderDiagnosticValue::I64(code) => Some(PrototypeQualifiedDiagnostic::Evm(
                    PrototypeEvmDiagnostic::JsonRpcError { code: *code },
                )),
                _ => None,
            }
        }
        ("bitcoin_core", ProviderDiagnosticCode::RpcHttpStatus, "scantxoutset") => {
            match fields.get(&id("http_status"))? {
                ProviderDiagnosticValue::U64(status) => u16::try_from(*status).ok().map(|status| {
                    PrototypeQualifiedDiagnostic::Bitcoin(PrototypeBitcoinDiagnostic::HttpStatus {
                        status,
                    })
                }),
                _ => None,
            }
        }
        ("bitcoin_core", ProviderDiagnosticCode::RpcJsonError, "scantxoutset") => {
            match fields.get(&id("rpc_code"))? {
                ProviderDiagnosticValue::I64(code) => Some(PrototypeQualifiedDiagnostic::Bitcoin(
                    PrototypeBitcoinDiagnostic::JsonRpcError { code: *code },
                )),
                _ => None,
            }
        }
        _ => None,
    }
}

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

#[test]
fn safe_failure_prototype_defines_closed_vocabulary_without_schema_encoding() {
    let evm_stable_codes = [
        PrototypeEvmStableFailureCode::RoutingGenerationUnavailable,
        PrototypeEvmStableFailureCode::ConfigurationInvalid,
        PrototypeEvmStableFailureCode::RequestInvalid,
        PrototypeEvmStableFailureCode::AccessCancelled,
        PrototypeEvmStableFailureCode::TransportFailed,
        PrototypeEvmStableFailureCode::HttpStatus,
        PrototypeEvmStableFailureCode::JsonRpcError,
        PrototypeEvmStableFailureCode::ResponseInvalid,
        PrototypeEvmStableFailureCode::ResponseMissingResult,
        PrototypeEvmStableFailureCode::ResponseTooLarge,
        PrototypeEvmStableFailureCode::UnclassifiedFailure,
    ];
    let bitcoin_stable_codes = [
        PrototypeBitcoinStableFailureCode::RoutingGenerationUnavailable,
        PrototypeBitcoinStableFailureCode::ConfigurationInvalid,
        PrototypeBitcoinStableFailureCode::RequestInvalid,
        PrototypeBitcoinStableFailureCode::AccessCancelled,
        PrototypeBitcoinStableFailureCode::TransportFailed,
        PrototypeBitcoinStableFailureCode::HttpStatus,
        PrototypeBitcoinStableFailureCode::JsonRpcError,
        PrototypeBitcoinStableFailureCode::ResponseInvalid,
        PrototypeBitcoinStableFailureCode::ResponseMissingResult,
        PrototypeBitcoinStableFailureCode::ResponseTooLarge,
        PrototypeBitcoinStableFailureCode::UnclassifiedFailure,
        PrototypeBitcoinStableFailureCode::ScanBusy,
    ];
    let failure_classes = [
        PrototypeFailureClass::Authorization,
        PrototypeFailureClass::Configuration,
        PrototypeFailureClass::Request,
        PrototypeFailureClass::Cancellation,
        PrototypeFailureClass::Transport,
        PrototypeFailureClass::Destination,
        PrototypeFailureClass::UnrepresentableResponse,
        PrototypeFailureClass::Integrity,
        PrototypeFailureClass::ResourceConflict,
        PrototypeFailureClass::Unclassified,
    ];
    let boundary_stages = [
        PrototypeBoundaryStage::BeforeBoundaryEntry,
        PrototypeBoundaryStage::BoundaryEntry,
        PrototypeBoundaryStage::BoundaryObservation,
    ];
    let coarse_size_classes = [
        PrototypeCoarseSizeClass::Zero,
        PrototypeCoarseSizeClass::UpTo16Kib,
        PrototypeCoarseSizeClass::UpTo1Mib,
        PrototypeCoarseSizeClass::Over1Mib,
    ];

    assert_eq!(evm_stable_codes.len(), 11);
    assert_eq!(bitcoin_stable_codes.len(), 12);
    assert_eq!(failure_classes.len(), 10);
    assert_eq!(boundary_stages.len(), 3);
    assert_eq!(coarse_size_classes.len(), 4);
}

#[test]
fn safe_failure_prototype_defines_saturating_size_buckets_without_exact_lengths() {
    for (observed_response_bytes, expected) in [
        (0, PrototypeCoarseSizeClass::Zero),
        (1, PrototypeCoarseSizeClass::UpTo16Kib),
        (16_384, PrototypeCoarseSizeClass::UpTo16Kib),
        (16_385, PrototypeCoarseSizeClass::UpTo1Mib),
        (1_048_576, PrototypeCoarseSizeClass::UpTo1Mib),
        (1_048_577, PrototypeCoarseSizeClass::Over1Mib),
        (usize::MAX, PrototypeCoarseSizeClass::Over1Mib),
    ] {
        assert_eq!(
            prototype_coarse_size_class(observed_response_bytes),
            expected
        );
    }
}

#[test]
fn reviewed_diagnostic_prototype_enforces_the_16_kib_admission_limit() {
    let diagnostic = PrototypeEvmDiagnostic::ResponseInvalid {
        kind: PrototypeResponseFailureKind::TooLarge,
    };
    assert!(
        prototype_diagnostic_ref(diagnostic, PROTOTYPE_MAX_REVIEWED_DIAGNOSTIC_BYTES).is_some()
    );
    assert!(
        prototype_diagnostic_ref(diagnostic, PROTOTYPE_MAX_REVIEWED_DIAGNOSTIC_BYTES + 1).is_none()
    );
}

#[test]
fn safe_failure_prototype_has_no_inline_diagnostic_or_text_escape_hatch() {
    let failure = PrototypeSafeFailure {
        stable_code: PrototypeEvmStableFailureCode::JsonRpcError,
        failure_class: PrototypeFailureClass::Destination,
        boundary_stage: PrototypeBoundaryStage::BoundaryObservation,
        coarse_size_class: None,
        diagnostic_ref: prototype_diagnostic_ref(
            PrototypeEvmDiagnostic::JsonRpcError { code: -32_603 },
            64,
        ),
    };
    assert_eq!(
        failure.stable_code,
        PrototypeEvmStableFailureCode::JsonRpcError
    );
    assert_eq!(failure.failure_class, PrototypeFailureClass::Destination);
    assert_eq!(
        failure.boundary_stage,
        PrototypeBoundaryStage::BoundaryObservation
    );
    assert_eq!(failure.coarse_size_class, None);
    assert_eq!(
        failure.diagnostic_ref,
        Some(PrototypeReviewedDiagnosticRef(
            PrototypeEvmDiagnostic::JsonRpcError { code: -32_603 }
        ))
    );
    let rendered = format!("{failure:?}");
    for synthetic_canary in [
        "https://fixture.invalid/private",
        "Authorization: Bearer fixture-token",
        "access_token=123456",
        "raw-provider-payload",
        "letmein",
    ] {
        assert!(!rendered.contains(synthetic_canary));
    }

    for diagnostic in [
        PrototypeEvmDiagnostic::HttpStatus { status: 503 },
        PrototypeEvmDiagnostic::ResponseInvalid {
            kind: PrototypeResponseFailureKind::TooLarge,
        },
        PrototypeEvmDiagnostic::ResponseInvalid {
            kind: PrototypeResponseFailureKind::MalformedEnvelope,
        },
    ] {
        assert_ne!(
            prototype_diagnostic_ref(diagnostic, 64),
            Some(PrototypeReviewedDiagnosticRef(
                PrototypeEvmDiagnostic::JsonRpcError { code: -32_603 }
            ))
        );
    }

    for diagnostic in [
        PrototypeBitcoinDiagnostic::HttpStatus { status: 500 },
        PrototypeBitcoinDiagnostic::JsonRpcError { code: -32_603 },
        PrototypeBitcoinDiagnostic::ScanBusy,
        PrototypeBitcoinDiagnostic::ResponseInvalid {
            kind: PrototypeResponseFailureKind::MissingResult,
        },
        PrototypeBitcoinDiagnostic::ResponseInvalid {
            kind: PrototypeResponseFailureKind::InvalidResult,
        },
    ] {
        assert!(prototype_diagnostic_ref(diagnostic, 64).is_some());
    }
}

#[test]
fn strict_diagnostic_qualification_rejects_arbitrary_generic_fields() {
    let qualified =
        RedactedProviderDiagnostic::new(id("evm"), ProviderDiagnosticCode::RpcJsonError)
            .with_operation(id("eth_chain_id"))
            .with_field(id("rpc_code"), ProviderDiagnosticValue::I64(-32_603));
    assert_eq!(
        prototype_qualify_generic_diagnostic(&qualified),
        Some(PrototypeQualifiedDiagnostic::Evm(
            PrototypeEvmDiagnostic::JsonRpcError { code: -32_603 }
        ))
    );

    for unqualified in [
        qualified.clone().with_field(
            id("provider_message"),
            ProviderDiagnosticValue::Id(id("letmein")),
        ),
        RedactedProviderDiagnostic::new(id("evm"), ProviderDiagnosticCode::RpcJsonError)
            .with_operation(id("letmein"))
            .with_field(id("rpc_code"), ProviderDiagnosticValue::I64(-32_603)),
        RedactedProviderDiagnostic::new(id("letmein"), ProviderDiagnosticCode::RpcJsonError)
            .with_operation(id("eth_chain_id"))
            .with_field(id("rpc_code"), ProviderDiagnosticValue::I64(-32_603)),
    ] {
        assert!(
            prototype_qualify_generic_diagnostic(&unqualified).is_none(),
            "generic identifiers require a capability-specific closed qualifier"
        );
    }
}
