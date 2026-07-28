use crate::*;

use bitcoin::{BlockHash, Network};

const LEGACY_MAIN: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
const SEGWIT_MAIN: &str = "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw";
const SHARED_TEST_ADDRESS: &str = "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn";

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
        BitcoinModelError::InvalidRequest {
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
            BitcoinModelError::InvalidRequest { reason }
        );
    }
    assert_eq!(
        BitcoinAddress::parse(SEGWIT_MAIN, BitcoinNetworkTag::Test).expect_err("wrong network"),
        BitcoinModelError::InvalidRequest {
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
        BitcoinModelError::InvalidRequest {
            reason: BitcoinInvalidRequest::AddressNetworkMismatch,
        }
    );
}

#[test]
fn scan_request_requires_a_bounded_sorted_unique_address_set() {
    let request = BitcoinScanRequest::new(
        BitcoinNetworkTag::Main,
        vec![LEGACY_MAIN.to_owned(), SEGWIT_MAIN.to_owned()],
    )
    .expect("sorted request");
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
            vec![LEGACY_MAIN.to_owned(); BITCOIN_SCAN_ADDRESS_LIMIT + 1],
            BitcoinInvalidRequest::AddressCount,
        ),
    ] {
        assert_eq!(
            BitcoinScanRequest::new(BitcoinNetworkTag::Main, addresses)
                .expect_err("invalid request"),
            BitcoinModelError::InvalidRequest { reason }
        );
    }
}

#[test]
fn one_rpc_results_do_not_claim_aggregate_or_confirmation_authority() {
    let anchor = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc"
        .parse::<BlockHash>()
        .expect("block hash");
    let scan = BitcoinScanResult::new(
        true,
        850_000,
        anchor,
        vec![
            BitcoinScannedBalance::new(LEGACY_MAIN.to_owned(), 1),
            BitcoinScannedBalance::new(SEGWIT_MAIN.to_owned(), 2),
        ],
    );
    assert!(scan.success());
    assert_eq!(scan.height(), 850_000);
    assert_eq!(scan.anchor_hash(), anchor);
    assert_eq!(scan.balances()[0].address(), LEGACY_MAIN);
    assert_eq!(scan.balances()[1].balance_sats(), 2);

    let info = BitcoinBlockchainInfo::new("main".to_owned(), false);
    assert_eq!(info.chain(), "main");
    assert!(!info.initial_block_download());
}
