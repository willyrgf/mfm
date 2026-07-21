use super::*;
use std::collections::BTreeSet;

use bitcoin::{OutPoint, Txid};

fn binding() -> BitcoinSourceBinding {
    BitcoinSourceBinding::new(
        BitcoinNetworkId::new("bitcoin-mainnet").expect("network"),
        BitcoinSourceIdentity::new("public-bitcoin-core").expect("source"),
        BitcoinNetworkTag::Main,
    )
}

fn request(selection: BtcHeadSelection) -> BtcChainHeadRequest {
    BtcChainHeadRequest::new(selection)
}

#[test]
fn confirmation_depth_rejects_zero() {
    let error = BtcHeadSelection::confirmed(0).expect_err("zero confirmations");

    assert_eq!(
        error,
        BtcCapabilityError::InvalidRequest {
            reason: BtcInvalidRequest::ZeroConfirmations,
        }
    );
}

#[test]
fn block_hash_validation_rejects_non_hash_values() {
    let error = BitcoinBlockHash::new("not-a-block-hash").expect_err("invalid hash");

    assert_eq!(
        error,
        BtcCapabilityError::InvalidRequest {
            reason: BtcInvalidRequest::InvalidBlockHash,
        }
    );
}

#[test]
fn block_hash_validation_normalizes_to_lowercase() {
    let hash =
        BitcoinBlockHash::new("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
            .expect("hash");

    assert_eq!(
        hash.to_string(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
}

#[test]
fn network_tags_cover_every_supported_bitcoin_core_chain() {
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
        BtcCapabilityError::InvalidRequest {
            reason: BtcInvalidRequest::InvalidBitcoinNetwork,
        }
    );
}

#[test]
fn addresses_require_canonical_rust_bitcoin_rendering() {
    let canonical = BitcoinAddress::new("bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw")
        .expect("canonical address");
    assert_eq!(
        canonical.as_str(),
        "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw"
    );

    for (case, value, reason) in [
        (
            "uppercase",
            "BC1QVZVKJN4Q3NSZQXRV3NRAGA2R822XJTY3YKVKUW",
            BtcInvalidRequest::NonCanonicalAddress,
        ),
        (
            "checksum",
            "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuq",
            BtcInvalidRequest::InvalidAddress,
        ),
        (
            "whitespace",
            " bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw",
            BtcInvalidRequest::InvalidAddress,
        ),
    ] {
        let error = BitcoinAddress::new(value).unwrap_err();
        assert_eq!(
            error,
            BtcCapabilityError::InvalidRequest { reason },
            "{case}"
        );
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(value), "{case}");
    }
}

#[test]
fn address_network_checks_preserve_shared_test_family_encodings() {
    let main =
        BitcoinAddress::new("bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw").expect("main address");
    let shared =
        BitcoinAddress::new("tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7")
            .expect("shared test address");
    let regtest = BitcoinAddress::new("bcrt1q2nfxmhd4n3c8834pj72xagvyr9gl57n5r94fsl")
        .expect("regtest address");

    main.require_network(BitcoinNetworkTag::Main).expect("main");
    for network in [
        BitcoinNetworkTag::Test,
        BitcoinNetworkTag::Testnet4,
        BitcoinNetworkTag::Signet,
    ] {
        shared.require_network(network).expect("shared test family");
    }
    regtest
        .require_network(BitcoinNetworkTag::Regtest)
        .expect("regtest");

    for (address, network) in [
        (&main, BitcoinNetworkTag::Testnet4),
        (&shared, BitcoinNetworkTag::Main),
        (&regtest, BitcoinNetworkTag::Signet),
    ] {
        assert_eq!(
            address.require_network(network).unwrap_err(),
            BtcCapabilityError::InvalidRequest {
                reason: BtcInvalidRequest::AddressNetworkMismatch,
            }
        );
    }
}

#[test]
fn address_scripts_and_outpoints_use_rust_bitcoin_primitives() {
    let address =
        BitcoinAddress::new("1BoatSLRHtKNngkdXEeobR76b53LETtpyT").expect("legacy main address");
    let duplicate = BitcoinAddress::new(address.as_str()).expect("duplicate address");
    let mut scripts = BTreeSet::new();
    assert!(scripts.insert(address.script_pubkey()));
    assert!(!scripts.insert(duplicate.script_pubkey()));

    let txid = "4d3f4f6f0b669a9a00909506f9bd30770f5ac37c1f91c3669e5d9f4f85b8f2f2"
        .parse::<Txid>()
        .expect("transaction id");
    let outpoint = OutPoint::new(txid, 7);
    assert_eq!(outpoint.txid, txid);
    assert_eq!(outpoint.vout, 7);
    assert!("not-a-txid".parse::<Txid>().is_err());
}

#[test]
fn address_order_is_canonical_rendered_utf8_order() {
    let mut addresses = [
        "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
        "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw",
        "1BoatSLRHtKNngkdXEeobR76b53LETtpyT",
    ]
    .map(|address| BitcoinAddress::new(address).expect("canonical address"));
    addresses.sort();

    assert_eq!(
        addresses.map(String::from),
        [
            "1BoatSLRHtKNngkdXEeobR76b53LETtpyT".to_owned(),
            "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw".to_owned(),
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7".to_owned(),
        ]
    );
}

#[test]
fn chain_head_request_is_operation_only() {
    let request = request(BtcHeadSelection::best());

    assert_eq!(request.selection(), BtcHeadSelection::best());
}

#[test]
fn response_evidence_is_built_from_provider_binding() {
    let binding = binding();
    let evidence =
        RedactedBtcSourceEvidence::from_binding(&binding, "main", BtcSourceStatus::Synced)
            .expect("evidence");

    assert_eq!(evidence.network_id, binding.network_id().clone());
    assert_eq!(evidence.source_identity, binding.source_identity().clone());
    assert_eq!(evidence.bitcoin_network, binding.bitcoin_network().as_str());
    assert_eq!(
        evidence.observed_bitcoin_network,
        binding.bitcoin_network().as_str()
    );
    assert_eq!(evidence.source_status, BtcSourceStatus::Synced);
}

#[test]
fn source_mismatch_diagnostic_stays_redacted() {
    let binding = binding();
    let mismatched = RedactedBtcSourceEvidence {
        source_identity: BitcoinSourceIdentity::new("different-semantic-source").expect("source"),
        ..RedactedBtcSourceEvidence::from_binding(&binding, "main", BtcSourceStatus::Unknown)
            .expect("evidence")
    };

    let diagnostic = mismatched.source_mismatch_diagnostic();
    let rendered = format!("{diagnostic:?} {diagnostic}");

    assert!(rendered.contains("different-semantic-source"));
    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains(concat!("Bear", "er")));
    assert!(!rendered.contains("secret"));
}

#[test]
fn response_evidence_rejects_observed_bitcoin_network_mismatch() {
    let error =
        RedactedBtcSourceEvidence::from_binding(&binding(), "test", BtcSourceStatus::Synced)
            .expect_err("observed network mismatch");

    assert!(matches!(error, BtcCapabilityError::SourceMismatch { .. }));
}

#[test]
fn provider_failure_carries_only_closed_diagnostics() {
    let diagnostic = btc_diagnostic(ProviderDiagnosticCode::RpcHttpStatus)
        .with_operation(btc_public_id("scantxoutset"))
        .with_field(
            btc_public_id("http_status"),
            ProviderDiagnosticValue::U64(403),
        );
    let error = BtcCapabilityError::provider_failure(diagnostic.clone());
    let rendered = format!("{error:?} {error}");

    assert_eq!(error, BtcCapabilityError::Provider { diagnostic });
    assert!(!rendered.contains("node.invalid"));
    assert!(!rendered.contains(concat!("Author", "ization")));
    assert!(!rendered.contains("secret"));
}
