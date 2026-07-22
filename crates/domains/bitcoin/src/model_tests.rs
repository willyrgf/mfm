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
