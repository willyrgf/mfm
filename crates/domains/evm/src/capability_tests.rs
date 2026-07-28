use super::*;

fn binding() -> EvmNetworkBinding {
    EvmNetworkBinding::new(LocalPublicId::new("mainnet").expect("network"), 1).expect("binding")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeReadObservation {
    DidNotEnter,
    Indeterminate,
    ReturnedSemanticFailure,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeEvmDiagnostic {
    HttpStatus { status: u16 },
    JsonRpcError { code: i64 },
    ResponseInvalid { kind: PrototypeResponseFailureKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeResponseFailureKind {
    MalformedEnvelope,
    MissingResult,
    InvalidResult,
    TooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrototypeReadClassification {
    observation: PrototypeReadObservation,
    stable_code: Option<PrototypeEvmStableFailureCode>,
    failure_class: Option<PrototypeFailureClass>,
    boundary_stage: Option<PrototypeBoundaryStage>,
    coarse_size_class: Option<PrototypeCoarseSizeClass>,
    diagnostic: Option<PrototypeEvmDiagnostic>,
}

#[derive(Debug, Clone, Copy)]
enum PrototypeEvmReadFailure {
    RoutingGenerationUnavailable,
    ConfigurationInvalid,
    RequestInvalid,
    CancelBeforeEntry,
    CancelAfterEntry,
    TransportBeforeEntry,
    TransportAfterPossibleEntry,
    HttpStatus(u16),
    JsonRpcError(i64),
    UnrepresentableResponse {
        kind: PrototypeResponseFailureKind,
        bytes: usize,
    },
    UnsafeUnknown,
    SourceMismatch,
    AnchorChanged,
}

fn prototype_size_class(bytes: usize) -> PrototypeCoarseSizeClass {
    match bytes {
        0 => PrototypeCoarseSizeClass::Zero,
        1..=16_384 => PrototypeCoarseSizeClass::UpTo16Kib,
        16_385..=1_048_576 => PrototypeCoarseSizeClass::UpTo1Mib,
        _ => PrototypeCoarseSizeClass::Over1Mib,
    }
}

fn prototype_evm_failure(failure: PrototypeEvmReadFailure) -> PrototypeReadClassification {
    use PrototypeBoundaryStage::{BeforeBoundaryEntry, BoundaryEntry, BoundaryObservation};
    use PrototypeEvmStableFailureCode::{
        AccessCancelled, ConfigurationInvalid, HttpStatus, JsonRpcError, RequestInvalid,
        ResponseInvalid, ResponseMissingResult, ResponseTooLarge, RoutingGenerationUnavailable,
        TransportFailed, UnclassifiedFailure,
    };
    use PrototypeFailureClass::{
        Authorization, Cancellation, Configuration, Destination, Request, Transport, Unclassified,
        UnrepresentableResponse as UnrepresentableClass,
    };
    use PrototypeReadObservation::{DidNotEnter, Indeterminate, ReturnedSemanticFailure};

    let safe =
        |observation, stable_code, failure_class, boundary_stage, coarse_size_class, diagnostic| {
            PrototypeReadClassification {
                observation,
                stable_code: Some(stable_code),
                failure_class: Some(failure_class),
                boundary_stage: Some(boundary_stage),
                coarse_size_class,
                diagnostic,
            }
        };
    match failure {
        PrototypeEvmReadFailure::RoutingGenerationUnavailable => safe(
            DidNotEnter,
            RoutingGenerationUnavailable,
            Authorization,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeEvmReadFailure::ConfigurationInvalid => safe(
            DidNotEnter,
            ConfigurationInvalid,
            Configuration,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeEvmReadFailure::RequestInvalid => safe(
            DidNotEnter,
            RequestInvalid,
            Request,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeEvmReadFailure::CancelBeforeEntry => safe(
            DidNotEnter,
            AccessCancelled,
            Cancellation,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeEvmReadFailure::CancelAfterEntry => safe(
            Indeterminate,
            AccessCancelled,
            Cancellation,
            BoundaryEntry,
            None,
            None,
        ),
        PrototypeEvmReadFailure::TransportBeforeEntry => safe(
            DidNotEnter,
            TransportFailed,
            Transport,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeEvmReadFailure::TransportAfterPossibleEntry => safe(
            Indeterminate,
            TransportFailed,
            Transport,
            BoundaryEntry,
            None,
            None,
        ),
        PrototypeEvmReadFailure::HttpStatus(status) => safe(
            Indeterminate,
            HttpStatus,
            Destination,
            BoundaryObservation,
            None,
            Some(PrototypeEvmDiagnostic::HttpStatus { status }),
        ),
        PrototypeEvmReadFailure::JsonRpcError(code) => safe(
            Indeterminate,
            JsonRpcError,
            Destination,
            BoundaryObservation,
            None,
            Some(PrototypeEvmDiagnostic::JsonRpcError { code }),
        ),
        PrototypeEvmReadFailure::UnrepresentableResponse { kind, bytes } => {
            let stable_code = match kind {
                PrototypeResponseFailureKind::MalformedEnvelope
                | PrototypeResponseFailureKind::InvalidResult => ResponseInvalid,
                PrototypeResponseFailureKind::MissingResult => ResponseMissingResult,
                PrototypeResponseFailureKind::TooLarge => ResponseTooLarge,
            };
            safe(
                Indeterminate,
                stable_code,
                UnrepresentableClass,
                BoundaryObservation,
                Some(prototype_size_class(bytes)),
                Some(PrototypeEvmDiagnostic::ResponseInvalid { kind }),
            )
        }
        PrototypeEvmReadFailure::UnsafeUnknown => safe(
            Indeterminate,
            UnclassifiedFailure,
            Unclassified,
            BoundaryObservation,
            None,
            None,
        ),
        PrototypeEvmReadFailure::SourceMismatch | PrototypeEvmReadFailure::AnchorChanged => {
            PrototypeReadClassification {
                observation: ReturnedSemanticFailure,
                stable_code: None,
                failure_class: None,
                boundary_stage: None,
                coarse_size_class: None,
                diagnostic: None,
            }
        }
    }
}

#[test]
fn network_binding_rejects_zero_chain_id() {
    let error = EvmNetworkBinding::new(LocalPublicId::new("mainnet").expect("network"), 0)
        .expect_err("zero chain id");
    assert_eq!(
        error,
        EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::ZeroExpectedChainId,
        }
    );
}

#[test]
fn session_evidence_is_one_redacted_checked_value() {
    let evidence = EvmSessionEvidence::new(
        &binding(),
        LocalPublicId::new("primary").expect("source"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation"),
    );
    let value = serde_json::to_value(&evidence).expect("JSON");

    assert_eq!(value["network_id"], "mainnet");
    assert_eq!(value["chain_id"], 1);
    assert_eq!(value["source_ref"], "primary");
    assert!(value.get("policy_id").is_none());
    assert!(value.get("observed_chain_id").is_none());
}

#[test]
fn session_evidence_deserialization_rechecks_all_identifiers() {
    let value = serde_json::json!({
        "network_id": "mainnet",
        "chain_id": 0,
        "source_ref": "primary",
        "implementation_id": EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
    });
    assert!(serde_json::from_value::<EvmSessionEvidence>(value).is_err());
}

#[test]
fn source_ref_is_audit_provenance_not_semantic_binding() {
    let binding = binding();
    let first = EvmSessionEvidence::new(
        &binding,
        LocalPublicId::new("primary").expect("source"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation"),
    );
    let resumed = EvmSessionEvidence::new(
        &binding,
        LocalPublicId::new("replacement").expect("source"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation"),
    );

    assert!(first.matches_binding(&binding));
    assert!(resumed.matches_binding(&binding));
    assert_ne!(first.source_ref(), resumed.source_ref());
}

#[test]
fn capability_failure_classification_is_closed_and_phase_aware() {
    use EvmCapabilityFailureDisposition::{OperationalBlock, TerminalValidation};
    use EvmCapabilityPhase::{AfterSubmission, BeforeSubmission, ReadOnly};

    let invalid = EvmCapabilityError::InvalidRequest {
        reason: EvmInvalidRequest::SessionAuthorityMismatch,
    };
    for phase in [ReadOnly, BeforeSubmission, AfterSubmission] {
        assert_eq!(invalid.failure_disposition(phase), TerminalValidation);
    }

    for code in [
        ProviderDiagnosticCode::ProviderConfigurationMissing,
        ProviderDiagnosticCode::ProviderConfigurationInvalid,
        ProviderDiagnosticCode::RouteUnavailable,
        ProviderDiagnosticCode::SourceUnavailable,
        ProviderDiagnosticCode::TransportFailed,
        ProviderDiagnosticCode::OperationIncomplete,
    ] {
        let error = EvmCapabilityError::provider_failure(evm_diagnostic(code));
        assert_eq!(error.failure_disposition(ReadOnly), OperationalBlock);
        assert_eq!(
            error.failure_disposition(BeforeSubmission),
            OperationalBlock
        );
        assert_eq!(error.failure_disposition(AfterSubmission), OperationalBlock);
    }

    for code in [
        ProviderDiagnosticCode::SourceNotAllowed,
        ProviderDiagnosticCode::ResponseInvalid,
        ProviderDiagnosticCode::ResponseMissingResult,
        ProviderDiagnosticCode::UnsupportedOperation,
    ] {
        let error = EvmCapabilityError::provider_failure(evm_diagnostic(code));
        assert_eq!(error.failure_disposition(ReadOnly), TerminalValidation);
        assert_eq!(
            error.failure_disposition(BeforeSubmission),
            TerminalValidation
        );
        assert_eq!(error.failure_disposition(AfterSubmission), OperationalBlock);
    }

    let mismatch = source_mismatch_error(
        &binding(),
        U256::from(2),
        &LocalPublicId::new("replacement").expect("source"),
    );
    for phase in [ReadOnly, BeforeSubmission, AfterSubmission] {
        assert_eq!(mismatch.failure_disposition(phase), OperationalBlock);
    }
}

#[test]
fn protocol_failure_classification_uses_closed_numeric_codes() {
    use EvmCapabilityFailureDisposition::{OperationalBlock, TerminalValidation};
    use EvmCapabilityPhase::{AfterSubmission, BeforeSubmission, ReadOnly};

    for status in [408, 425, 429, 500, 502, 503, 504, 507] {
        let error = EvmCapabilityError::provider_failure(
            evm_diagnostic(ProviderDiagnosticCode::RpcHttpStatus).with_field(
                public_id("http_status"),
                ProviderDiagnosticValue::U64(status),
            ),
        );
        assert_eq!(error.failure_disposition(ReadOnly), OperationalBlock);
        assert_eq!(
            error.failure_disposition(BeforeSubmission),
            OperationalBlock
        );
        assert_eq!(error.failure_disposition(AfterSubmission), OperationalBlock);
    }
    for status in [307, 400, 401, 403, 404, 413, 422, 501, 505] {
        let error = EvmCapabilityError::provider_failure(
            evm_diagnostic(ProviderDiagnosticCode::RpcHttpStatus).with_field(
                public_id("http_status"),
                ProviderDiagnosticValue::U64(status),
            ),
        );
        assert_eq!(error.failure_disposition(ReadOnly), TerminalValidation);
        assert_eq!(
            error.failure_disposition(BeforeSubmission),
            TerminalValidation
        );
        assert_eq!(error.failure_disposition(AfterSubmission), OperationalBlock);
    }

    for code in [-32603, -32001, -32002, -32005] {
        let error = EvmCapabilityError::provider_failure(
            evm_diagnostic(ProviderDiagnosticCode::RpcJsonError)
                .with_field(public_id("rpc_code"), ProviderDiagnosticValue::I64(code)),
        );
        assert_eq!(error.failure_disposition(ReadOnly), OperationalBlock);
        assert_eq!(
            error.failure_disposition(BeforeSubmission),
            OperationalBlock
        );
        assert_eq!(error.failure_disposition(AfterSubmission), OperationalBlock);
    }
    for code in [
        -32700, -32600, -32601, -32602, -32000, -32003, -32004, -32006,
    ] {
        let error = EvmCapabilityError::provider_failure(
            evm_diagnostic(ProviderDiagnosticCode::RpcJsonError)
                .with_field(public_id("rpc_code"), ProviderDiagnosticValue::I64(code)),
        );
        assert_eq!(error.failure_disposition(ReadOnly), TerminalValidation);
        assert_eq!(
            error.failure_disposition(BeforeSubmission),
            TerminalValidation
        );
        assert_eq!(error.failure_disposition(AfterSubmission), OperationalBlock);
    }

    for code in [
        ProviderDiagnosticCode::RpcHttpStatus,
        ProviderDiagnosticCode::RpcJsonError,
    ] {
        let malformed = EvmCapabilityError::provider_failure(evm_diagnostic(code));
        assert_eq!(malformed.failure_disposition(ReadOnly), TerminalValidation);
        assert_eq!(
            malformed.failure_disposition(BeforeSubmission),
            TerminalValidation
        );
        assert_eq!(
            malformed.failure_disposition(AfterSubmission),
            OperationalBlock
        );
    }
}

#[test]
fn source_mismatch_diagnostic_is_closed_and_redacted() {
    let error = source_mismatch_error(
        &binding(),
        U256::from(2),
        &LocalPublicId::new("primary").expect("source"),
    );
    let diagnostic = error.redacted_diagnostic().expect("diagnostic");
    let rendered = format!("{diagnostic:?} {diagnostic}");

    assert_eq!(diagnostic.stable_error_code(), "evm_source_mismatch");
    assert!(diagnostic.summary().contains("observed_chain_id=2"));
    for forbidden in ["http://", "Bearer", "password"] {
        assert!(!rendered.contains(forbidden));
    }
}

#[test]
fn recoverability_safe_failure_prototype_classifies_evm_reads_exhaustively() {
    use PrototypeBoundaryStage::{BeforeBoundaryEntry, BoundaryEntry, BoundaryObservation};
    use PrototypeEvmStableFailureCode::{
        AccessCancelled, ConfigurationInvalid, HttpStatus, JsonRpcError, RequestInvalid,
        ResponseInvalid, ResponseMissingResult, ResponseTooLarge, RoutingGenerationUnavailable,
        TransportFailed, UnclassifiedFailure,
    };
    use PrototypeFailureClass::{
        Authorization, Cancellation, Configuration, Destination, Integrity, Request,
        ResourceConflict, Transport, Unclassified, UnrepresentableResponse,
    };
    use PrototypeReadObservation::{DidNotEnter, Indeterminate, ReturnedSemanticFailure};
    use PrototypeResponseFailureKind::{InvalidResult, MalformedEnvelope, MissingResult, TooLarge};

    let compact = |observation, code, class, stage, size, diagnostic| PrototypeReadClassification {
        observation,
        stable_code: code,
        failure_class: class,
        boundary_stage: stage,
        coarse_size_class: size,
        diagnostic,
    };

    let universal_failure_classes = [
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
    ];
    assert_eq!(universal_failure_classes.len(), 10);

    for (failure, expected) in [
        (
            PrototypeEvmReadFailure::RoutingGenerationUnavailable,
            compact(
                DidNotEnter,
                Some(RoutingGenerationUnavailable),
                Some(Authorization),
                Some(BeforeBoundaryEntry),
                None,
                None,
            ),
        ),
        (
            PrototypeEvmReadFailure::ConfigurationInvalid,
            compact(
                DidNotEnter,
                Some(ConfigurationInvalid),
                Some(Configuration),
                Some(BeforeBoundaryEntry),
                None,
                None,
            ),
        ),
        (
            PrototypeEvmReadFailure::RequestInvalid,
            compact(
                DidNotEnter,
                Some(RequestInvalid),
                Some(Request),
                Some(BeforeBoundaryEntry),
                None,
                None,
            ),
        ),
        (
            PrototypeEvmReadFailure::CancelBeforeEntry,
            compact(
                DidNotEnter,
                Some(AccessCancelled),
                Some(Cancellation),
                Some(BeforeBoundaryEntry),
                None,
                None,
            ),
        ),
        (
            PrototypeEvmReadFailure::CancelAfterEntry,
            compact(
                Indeterminate,
                Some(AccessCancelled),
                Some(Cancellation),
                Some(BoundaryEntry),
                None,
                None,
            ),
        ),
        (
            PrototypeEvmReadFailure::TransportBeforeEntry,
            compact(
                DidNotEnter,
                Some(TransportFailed),
                Some(Transport),
                Some(BeforeBoundaryEntry),
                None,
                None,
            ),
        ),
        (
            PrototypeEvmReadFailure::TransportAfterPossibleEntry,
            compact(
                Indeterminate,
                Some(TransportFailed),
                Some(Transport),
                Some(BoundaryEntry),
                None,
                None,
            ),
        ),
        (
            PrototypeEvmReadFailure::UnsafeUnknown,
            compact(
                Indeterminate,
                Some(UnclassifiedFailure),
                Some(Unclassified),
                Some(BoundaryObservation),
                None,
                None,
            ),
        ),
    ] {
        assert_eq!(prototype_evm_failure(failure), expected);
    }

    for status in [400, 401, 403, 408, 425, 429, 500, 502, 503, 504, 507] {
        let classification = prototype_evm_failure(PrototypeEvmReadFailure::HttpStatus(status));
        assert_eq!(classification.stable_code, Some(HttpStatus));
        assert_eq!(classification.failure_class, Some(Destination));
        assert_eq!(classification.boundary_stage, Some(BoundaryObservation));
        assert_eq!(
            classification.diagnostic,
            Some(PrototypeEvmDiagnostic::HttpStatus { status })
        );
    }
    for code in [-32_603, -32_001, -32_002, -32_005, -32_600, -32_601, -8, 3] {
        let classification = prototype_evm_failure(PrototypeEvmReadFailure::JsonRpcError(code));
        assert_eq!(classification.stable_code, Some(JsonRpcError));
        assert_eq!(classification.failure_class, Some(Destination));
        assert_eq!(
            classification.diagnostic,
            Some(PrototypeEvmDiagnostic::JsonRpcError { code })
        );
    }

    for (kind, stable_code) in [
        (MalformedEnvelope, ResponseInvalid),
        (MissingResult, ResponseMissingResult),
        (InvalidResult, ResponseInvalid),
        (TooLarge, ResponseTooLarge),
    ] {
        let classification =
            prototype_evm_failure(PrototypeEvmReadFailure::UnrepresentableResponse {
                kind,
                bytes: 42,
            });
        assert_eq!(classification.stable_code, Some(stable_code));
        assert_eq!(classification.failure_class, Some(UnrepresentableResponse));
        assert_eq!(classification.boundary_stage, Some(BoundaryObservation));
        assert_eq!(
            classification.diagnostic,
            Some(PrototypeEvmDiagnostic::ResponseInvalid { kind })
        );
    }

    for (bytes, size) in [
        (0, PrototypeCoarseSizeClass::Zero),
        (1, PrototypeCoarseSizeClass::UpTo16Kib),
        (16_384, PrototypeCoarseSizeClass::UpTo16Kib),
        (16_385, PrototypeCoarseSizeClass::UpTo1Mib),
        (1_048_576, PrototypeCoarseSizeClass::UpTo1Mib),
        (1_048_577, PrototypeCoarseSizeClass::Over1Mib),
    ] {
        assert_eq!(
            prototype_evm_failure(PrototypeEvmReadFailure::UnrepresentableResponse {
                kind: TooLarge,
                bytes,
            })
            .coarse_size_class,
            Some(size)
        );
    }
    for semantic in [
        PrototypeEvmReadFailure::SourceMismatch,
        PrototypeEvmReadFailure::AnchorChanged,
    ] {
        assert_eq!(
            prototype_evm_failure(semantic),
            compact(ReturnedSemanticFailure, None, None, None, None, None)
        );
    }
}

#[test]
fn fee_policy_uses_checked_u256_arithmetic() {
    let fees = EvmFeeInputs::from_base_and_priority(U256::from(10), U256::from(3)).expect("fees");
    assert_eq!(fees.max_fee_per_gas, U256::from(23));
    assert_eq!(
        EvmFeeInputs::from_base_and_priority(U256::MAX, U256::ZERO),
        Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::FeeOverflow,
        })
    );
}

#[test]
fn estimate_admission_rejects_alloy_width_overflow_before_io() {
    let overflow_u64 = U256::from(u64::MAX) + U256::from(1);
    let overflow_u128 = U256::from(u128::MAX) + U256::from(1);

    for (chain_id, nonce, maximum, priority, reason) in [
        (
            overflow_u64,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            EvmInvalidRequest::ChainIdOutOfRange,
        ),
        (
            U256::from(1),
            overflow_u64,
            U256::ZERO,
            U256::ZERO,
            EvmInvalidRequest::NonceOutOfRange,
        ),
        (
            U256::from(1),
            U256::ZERO,
            overflow_u128,
            U256::ZERO,
            EvmInvalidRequest::MaxFeePerGasOutOfRange,
        ),
        (
            U256::from(1),
            U256::ZERO,
            U256::from(u128::MAX),
            overflow_u128,
            EvmInvalidRequest::MaxPriorityFeePerGasOutOfRange,
        ),
        (
            U256::from(1),
            U256::ZERO,
            U256::from(1),
            U256::from(2),
            EvmInvalidRequest::PriorityFeeExceedsMaxFee,
        ),
    ] {
        let error = EvmTransactionEstimate::new(
            chain_id,
            nonce,
            Address::ZERO,
            TxKind::Create,
            U256::MAX,
            Bytes::new(),
            AccessList::default(),
            maximum,
            priority,
        )
        .expect_err("invalid estimate width");
        assert_eq!(error, EvmCapabilityError::InvalidRequest { reason });
    }

    let estimate = EvmTransactionEstimate::new(
        U256::from(u64::MAX),
        U256::from(u64::MAX),
        Address::ZERO,
        TxKind::Create,
        U256::MAX,
        Bytes::new(),
        AccessList::default(),
        U256::from(u128::MAX),
        U256::from(u128::MAX),
    )
    .expect("exact boundaries");
    assert_eq!(estimate.transaction_type(), EVM_EIP1559_TRANSACTION_TYPE);
    assert_eq!(estimate.nonce(), U256::from(u64::MAX));
    assert_eq!(estimate.max_fee_per_gas(), U256::from(u128::MAX));
}

#[test]
fn call_request_requires_an_explicit_nonzero_gas_bound() {
    let selector = EvmBlockSelector::ExactHash(B256::from([3; 32]));
    let error = EvmCall::new(
        Address::ZERO,
        Address::from([4; 20]),
        U256::ZERO,
        Bytes::new(),
        U256::ZERO,
        AccessList::default(),
        selector,
        0,
    )
    .expect_err("zero gas bound");
    assert_eq!(
        error,
        EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::ZeroCallGasLimit,
        }
    );
}

#[test]
fn call_request_requires_a_bounded_result() {
    let selector = EvmBlockSelector::ExactHash(B256::from([3; 32]));
    let empty = EvmCall::new(
        Address::ZERO,
        Address::from([4; 20]),
        U256::ZERO,
        Bytes::new(),
        U256::from(1),
        AccessList::default(),
        selector.clone(),
        0,
    )
    .expect("empty result bound");
    assert_eq!(empty.max_response_bytes(), 0);

    let error = EvmCall::new(
        Address::ZERO,
        Address::from([4; 20]),
        U256::ZERO,
        Bytes::new(),
        U256::from(1),
        AccessList::default(),
        selector,
        EVM_CALL_MAX_RESPONSE_BYTES + 1,
    )
    .expect_err("oversized result bound");
    assert_eq!(
        error,
        EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::CallResponseLimitExceeded,
        }
    );
}

#[test]
fn receipt_rejects_removed_incoherent_or_reverted_logs() {
    let transaction_hash = B256::from([1; 32]);
    let block_hash = B256::from([2; 32]);
    let mut receipt = EvmReceipt {
        transaction_hash,
        transaction_index: U256::from(3),
        block: EvmBlockAnchor::new(U256::from(4), block_hash),
        from: Address::ZERO,
        to: None,
        contract_address: None,
        status: EvmReceiptStatus::Success,
        gas_used: U256::from(5),
        cumulative_gas_used: U256::from(6),
        logs: vec![EvmReceiptLog {
            address: Address::ZERO,
            topics: vec![],
            data: Bytes::new(),
            block: EvmBlockAnchor::new(U256::from(4), block_hash),
            transaction_hash,
            transaction_index: U256::from(3),
            log_index: U256::ZERO,
            removed: false,
        }],
    };
    receipt.validate().expect("coherent receipt");
    receipt.status = EvmReceiptStatus::Reverted;
    assert_eq!(
        receipt.validate(),
        Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::IncoherentReceipt,
        })
    );
    receipt.logs.clear();
    receipt.validate().expect("empty reverted receipt");

    receipt.status = EvmReceiptStatus::Success;
    receipt.logs.push(EvmReceiptLog {
        address: Address::ZERO,
        topics: vec![],
        data: Bytes::new(),
        block: EvmBlockAnchor::new(U256::from(4), block_hash),
        transaction_hash,
        transaction_index: U256::from(3),
        log_index: U256::ZERO,
        removed: false,
    });
    receipt.logs[0].removed = true;
    assert_eq!(
        receipt.validate(),
        Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::IncoherentReceipt,
        })
    );
}
