use crate::*;

use bitcoin::{BlockHash, Network};
use mfm_capabilities::CapabilitySpec;
use mfm_capabilities::ProviderDiagnosticCode;

const LEGACY_MAIN: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
const SEGWIT_MAIN: &str = "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw";
const SHARED_TEST_ADDRESS: &str = "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn";

fn binding() -> BitcoinSourceBinding {
    BitcoinSourceBinding::new(
        BitcoinNetworkId::new("bitcoin-mainnet").expect("network"),
        BitcoinNetworkTag::Main,
        BitcoinSourceIdentity::new("public-bitcoin-core").expect("source"),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeReadObservation {
    DidNotEnter,
    Indeterminate,
    ReturnedSemanticFailure,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeBitcoinDiagnostic {
    HttpStatus { status: u16 },
    JsonRpcError { code: i64 },
    ScanBusy,
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
    stable_code: Option<PrototypeBitcoinStableFailureCode>,
    failure_class: Option<PrototypeFailureClass>,
    boundary_stage: Option<PrototypeBoundaryStage>,
    coarse_size_class: Option<PrototypeCoarseSizeClass>,
    diagnostic: Option<PrototypeBitcoinDiagnostic>,
}

#[derive(Debug, Clone, Copy)]
enum PrototypeBitcoinReadFailure {
    RoutingGenerationUnavailable,
    ConfigurationInvalid,
    RequestInvalid,
    CancelBeforeEntry,
    CancelAfterEntry,
    TransportBeforeEntry,
    TransportAfterPossibleEntry,
    HttpStatus(u16),
    JsonRpcError(i64),
    ScanBusy,
    UnrepresentableResponse {
        kind: PrototypeResponseFailureKind,
        bytes: usize,
    },
    UnsafeUnknown,
    SourceMismatch,
    ScanIncomplete,
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

fn prototype_bitcoin_failure(failure: PrototypeBitcoinReadFailure) -> PrototypeReadClassification {
    use PrototypeBitcoinStableFailureCode::{
        AccessCancelled, ConfigurationInvalid, HttpStatus, JsonRpcError, RequestInvalid,
        ResponseInvalid, ResponseMissingResult, ResponseTooLarge, RoutingGenerationUnavailable,
        ScanBusy, TransportFailed, UnclassifiedFailure,
    };
    use PrototypeBoundaryStage::{BeforeBoundaryEntry, BoundaryEntry, BoundaryObservation};
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
        PrototypeBitcoinReadFailure::RoutingGenerationUnavailable => safe(
            DidNotEnter,
            RoutingGenerationUnavailable,
            Authorization,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeBitcoinReadFailure::ConfigurationInvalid => safe(
            DidNotEnter,
            ConfigurationInvalid,
            Configuration,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeBitcoinReadFailure::RequestInvalid => safe(
            DidNotEnter,
            RequestInvalid,
            Request,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeBitcoinReadFailure::CancelBeforeEntry => safe(
            DidNotEnter,
            AccessCancelled,
            Cancellation,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeBitcoinReadFailure::CancelAfterEntry => safe(
            Indeterminate,
            AccessCancelled,
            Cancellation,
            BoundaryEntry,
            None,
            None,
        ),
        PrototypeBitcoinReadFailure::TransportBeforeEntry => safe(
            DidNotEnter,
            TransportFailed,
            Transport,
            BeforeBoundaryEntry,
            None,
            None,
        ),
        PrototypeBitcoinReadFailure::TransportAfterPossibleEntry => safe(
            Indeterminate,
            TransportFailed,
            Transport,
            BoundaryEntry,
            None,
            None,
        ),
        PrototypeBitcoinReadFailure::HttpStatus(status) => safe(
            Indeterminate,
            HttpStatus,
            Destination,
            BoundaryObservation,
            None,
            Some(PrototypeBitcoinDiagnostic::HttpStatus { status }),
        ),
        PrototypeBitcoinReadFailure::JsonRpcError(code) => safe(
            Indeterminate,
            JsonRpcError,
            Destination,
            BoundaryObservation,
            None,
            Some(PrototypeBitcoinDiagnostic::JsonRpcError { code }),
        ),
        PrototypeBitcoinReadFailure::ScanBusy => safe(
            Indeterminate,
            ScanBusy,
            Destination,
            BoundaryObservation,
            None,
            Some(PrototypeBitcoinDiagnostic::ScanBusy),
        ),
        PrototypeBitcoinReadFailure::UnrepresentableResponse { kind, bytes } => {
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
                Some(PrototypeBitcoinDiagnostic::ResponseInvalid { kind }),
            )
        }
        PrototypeBitcoinReadFailure::UnsafeUnknown => safe(
            Indeterminate,
            UnclassifiedFailure,
            Unclassified,
            BoundaryObservation,
            None,
            None,
        ),
        PrototypeBitcoinReadFailure::SourceMismatch
        | PrototypeBitcoinReadFailure::ScanIncomplete
        | PrototypeBitcoinReadFailure::AnchorChanged => PrototypeReadClassification {
            observation: ReturnedSemanticFailure,
            stable_code: None,
            failure_class: None,
            boundary_stage: None,
            coarse_size_class: None,
            diagnostic: None,
        },
    }
}

#[test]
fn aggregate_capability_identity_is_stable() {
    assert_eq!(
        BitcoinBalanceCollectionReadCapability::name(),
        "mfm.bitcoin.balance_collection.read"
    );
    assert_eq!(
        BitcoinBalanceCollectionReadCapability::version()
            .expect("version")
            .as_str(),
        "mfm.bitcoin.balance_collection.read.v1"
    );
    assert!(BitcoinBalanceCollectionReadCapability::kind()
        .expect("kind")
        .to_string()
        .starts_with("capability:mfm.bitcoin:balance_collection.read:"));
}

#[test]
fn network_tags_cover_supported_bitcoin_core_chains() {
    for (tag, expected, network) in [
        ("main", BitcoinNetworkTag::Main, Network::Bitcoin),
        ("test", BitcoinNetworkTag::Test, Network::Testnet),
        ("testnet4", BitcoinNetworkTag::Testnet4, Network::Testnet4),
        ("signet", BitcoinNetworkTag::Signet, Network::Signet),
        ("regtest", BitcoinNetworkTag::Regtest, Network::Regtest),
    ] {
        let parsed = BitcoinNetworkTag::new(tag).expect(tag);
        assert_eq!(parsed, expected);
        assert_eq!(parsed.as_str(), tag);
        assert_eq!(parsed.network(), network);
    }
    assert_eq!(
        BitcoinNetworkTag::new("testnet3").expect_err("unsupported alias"),
        BitcoinCapabilityError::InvalidRequest {
            reason: BitcoinInvalidRequest::InvalidBitcoinNetwork,
        }
    );
}

#[test]
fn addresses_use_canonical_rust_bitcoin_identity_and_network_checks() {
    let legacy = BitcoinAddress::parse(LEGACY_MAIN, BitcoinNetworkTag::Main).expect("legacy");
    let segwit = BitcoinAddress::parse(SEGWIT_MAIN, BitcoinNetworkTag::Main).expect("segwit");
    assert_eq!(legacy.as_str(), LEGACY_MAIN);
    assert_eq!(segwit.as_str(), SEGWIT_MAIN);
    assert_eq!(segwit.scan_descriptor(), format!("addr({SEGWIT_MAIN})"));
    assert_ne!(legacy.script_pubkey(), segwit.script_pubkey());

    for (value, reason) in [
        (
            "BC1QVZVKJN4Q3NSZQXRV3NRAGA2R822XJTY3YKVKUW",
            BitcoinInvalidRequest::NonCanonicalAddress,
        ),
        ("not-an-address", BitcoinInvalidRequest::InvalidAddress),
    ] {
        assert_eq!(
            BitcoinAddress::parse_any(value).expect_err("invalid address"),
            BitcoinCapabilityError::InvalidRequest { reason }
        );
    }
    assert_eq!(
        BitcoinAddress::parse(SEGWIT_MAIN, BitcoinNetworkTag::Test).expect_err("wrong network"),
        BitcoinCapabilityError::InvalidRequest {
            reason: BitcoinInvalidRequest::AddressNetworkMismatch,
        }
    );
}

#[test]
fn shared_test_family_address_encoding_is_bound_by_the_selected_rpc_chain() {
    for network in [
        BitcoinNetworkTag::Test,
        BitcoinNetworkTag::Testnet4,
        BitcoinNetworkTag::Signet,
        BitcoinNetworkTag::Regtest,
    ] {
        assert_eq!(
            BitcoinAddress::parse(SHARED_TEST_ADDRESS, network)
                .expect("shared test-family address")
                .as_str(),
            SHARED_TEST_ADDRESS
        );
    }
    assert_eq!(
        BitcoinAddress::parse(SHARED_TEST_ADDRESS, BitcoinNetworkTag::Main)
            .expect_err("mainnet mismatch"),
        BitcoinCapabilityError::InvalidRequest {
            reason: BitcoinInvalidRequest::AddressNetworkMismatch,
        }
    );
}

#[test]
fn aggregate_request_requires_a_bounded_sorted_unique_address_set() {
    let request = BitcoinBalanceCollectionRequest::new(
        binding(),
        vec![LEGACY_MAIN.to_owned(), SEGWIT_MAIN.to_owned()],
    )
    .expect("sorted request");
    assert_eq!(request.binding(), &binding());
    assert_eq!(
        request
            .addresses()
            .iter()
            .map(BitcoinAddress::as_str)
            .collect::<Vec<_>>(),
        vec![LEGACY_MAIN, SEGWIT_MAIN]
    );

    for (addresses, reason) in [
        (Vec::new(), BitcoinInvalidRequest::AddressCount),
        (
            vec![SEGWIT_MAIN.to_owned(), LEGACY_MAIN.to_owned()],
            BitcoinInvalidRequest::AddressOrder,
        ),
        (
            vec![LEGACY_MAIN.to_owned(), LEGACY_MAIN.to_owned()],
            BitcoinInvalidRequest::AddressOrder,
        ),
        (
            vec![LEGACY_MAIN.to_owned(); BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT + 1],
            BitcoinInvalidRequest::AddressCount,
        ),
    ] {
        assert_eq!(
            BitcoinBalanceCollectionRequest::new(binding(), addresses)
                .expect_err("invalid request"),
            BitcoinCapabilityError::InvalidRequest { reason }
        );
    }
}

#[test]
fn aggregate_response_preserves_binding_anchor_and_request_order() {
    let anchor = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc"
        .parse::<BlockHash>()
        .expect("block hash");
    let response = BitcoinBalanceCollectionResponse::new(
        binding(),
        BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
        850_000,
        anchor,
        vec![
            BitcoinAddressBalance::new(LEGACY_MAIN.to_owned(), 1),
            BitcoinAddressBalance::new(SEGWIT_MAIN.to_owned(), 2),
        ],
        anchor,
    );
    assert_eq!(response.binding(), &binding());
    assert_eq!(response.anchor_height(), 850_000);
    assert_eq!(response.anchor_hash(), anchor);
    assert_eq!(response.final_canonical_hash(), anchor);
    assert_eq!(
        response.implementation_id(),
        BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID
    );
    assert_eq!(response.balances()[0].address(), LEGACY_MAIN);
    assert_eq!(response.balances()[1].balance_sats(), 2);
}

#[test]
fn provider_failures_expose_only_closed_diagnostics_and_retryability() {
    let retryable = BitcoinCapabilityError::provider(
        ProviderDiagnosticCode::TransportFailed,
        "aggregate_read",
        true,
    );
    assert!(retryable.is_retryable());
    let permanent = BitcoinCapabilityError::provider(
        ProviderDiagnosticCode::ResponseInvalid,
        "aggregate_read",
        false,
    );
    assert!(!permanent.is_retryable());
    let rendered = format!("{permanent:?} {permanent}");
    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains("secret"));
}

#[test]
fn recoverability_safe_failure_prototype_classifies_bitcoin_reads_exhaustively() {
    use PrototypeBitcoinStableFailureCode::{
        AccessCancelled, ConfigurationInvalid, HttpStatus, JsonRpcError, RequestInvalid,
        ResponseInvalid, ResponseMissingResult, ResponseTooLarge, RoutingGenerationUnavailable,
        ScanBusy, TransportFailed, UnclassifiedFailure,
    };
    use PrototypeBoundaryStage::{BeforeBoundaryEntry, BoundaryEntry, BoundaryObservation};
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
            PrototypeBitcoinReadFailure::RoutingGenerationUnavailable,
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
            PrototypeBitcoinReadFailure::ConfigurationInvalid,
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
            PrototypeBitcoinReadFailure::RequestInvalid,
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
            PrototypeBitcoinReadFailure::CancelBeforeEntry,
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
            PrototypeBitcoinReadFailure::CancelAfterEntry,
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
            PrototypeBitcoinReadFailure::TransportBeforeEntry,
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
            PrototypeBitcoinReadFailure::TransportAfterPossibleEntry,
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
            PrototypeBitcoinReadFailure::ScanBusy,
            compact(
                Indeterminate,
                Some(ScanBusy),
                Some(Destination),
                Some(BoundaryObservation),
                None,
                Some(PrototypeBitcoinDiagnostic::ScanBusy),
            ),
        ),
        (
            PrototypeBitcoinReadFailure::UnsafeUnknown,
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
        assert_eq!(prototype_bitcoin_failure(failure), expected);
    }

    for status in [400, 401, 403, 408, 429, 500, 502, 503] {
        let classification =
            prototype_bitcoin_failure(PrototypeBitcoinReadFailure::HttpStatus(status));
        assert_eq!(classification.stable_code, Some(HttpStatus));
        assert_eq!(classification.failure_class, Some(Destination));
        assert_eq!(
            classification.diagnostic,
            Some(PrototypeBitcoinDiagnostic::HttpStatus { status })
        );
    }
    for code in [-8, -32_603, -32_600, 3] {
        let classification =
            prototype_bitcoin_failure(PrototypeBitcoinReadFailure::JsonRpcError(code));
        assert_eq!(classification.stable_code, Some(JsonRpcError));
        assert_eq!(classification.failure_class, Some(Destination));
        assert_eq!(
            classification.diagnostic,
            Some(PrototypeBitcoinDiagnostic::JsonRpcError { code })
        );
    }

    for (kind, stable_code) in [
        (MalformedEnvelope, ResponseInvalid),
        (MissingResult, ResponseMissingResult),
        (InvalidResult, ResponseInvalid),
        (TooLarge, ResponseTooLarge),
    ] {
        let classification =
            prototype_bitcoin_failure(PrototypeBitcoinReadFailure::UnrepresentableResponse {
                kind,
                bytes: 42,
            });
        assert_eq!(classification.stable_code, Some(stable_code));
        assert_eq!(classification.failure_class, Some(UnrepresentableResponse));
        assert_eq!(classification.boundary_stage, Some(BoundaryObservation));
        assert_eq!(
            classification.diagnostic,
            Some(PrototypeBitcoinDiagnostic::ResponseInvalid { kind })
        );
    }

    for (bytes, size) in [
        (0, PrototypeCoarseSizeClass::Zero),
        (1, PrototypeCoarseSizeClass::UpTo16Kib),
        (16_384, PrototypeCoarseSizeClass::UpTo16Kib),
        (16_385, PrototypeCoarseSizeClass::UpTo1Mib),
        (1_048_576, PrototypeCoarseSizeClass::UpTo1Mib),
        (1_048_577, PrototypeCoarseSizeClass::Over1Mib),
        (usize::MAX, PrototypeCoarseSizeClass::Over1Mib),
    ] {
        assert_eq!(
            prototype_bitcoin_failure(PrototypeBitcoinReadFailure::UnrepresentableResponse {
                kind: TooLarge,
                bytes,
            })
            .coarse_size_class,
            Some(size)
        );
    }
    for semantic in [
        PrototypeBitcoinReadFailure::SourceMismatch,
        PrototypeBitcoinReadFailure::ScanIncomplete,
        PrototypeBitcoinReadFailure::AnchorChanged,
    ] {
        assert_eq!(
            prototype_bitcoin_failure(semantic),
            compact(ReturnedSemanticFailure, None, None, None, None, None)
        );
    }
}
