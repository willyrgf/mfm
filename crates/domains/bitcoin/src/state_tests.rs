use super::*;

use mfm_facts::MfmFactType;
use mfm_program::ValidatedConfig;

const LEGACY_MAIN: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
const SEGWIT_MAIN: &str = "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw";
const ANCHOR_HASH: &str = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const OTHER_HASH: &str = "00000000000000000002b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";

fn config() -> BitcoinBalanceCollectionConfig {
    BitcoinBalanceCollectionConfig::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        vec![LEGACY_MAIN.to_owned(), SEGWIT_MAIN.to_owned()],
    )
    .expect("config")
}

fn plan() -> BitcoinBalanceCollectionPlan {
    let state = <CollectBitcoinBalancesState as StateSpec>::new(
        ValidatedConfig::new(config()).expect("validated config"),
    )
    .expect("state");
    state
        .plan(&(), &mfm_program::CertifiedContext::no_context())
        .expect("plan")
}

fn evidence() -> BitcoinBalanceCollectionEvidence {
    BitcoinBalanceCollectionEvidence {
        network_id: "bitcoin-mainnet".to_owned(),
        bitcoin_network: "main".to_owned(),
        semantic_source_identity: "public-bitcoin-core".to_owned(),
        implementation_id: BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID.to_owned(),
        anchor_height: 850_000,
        anchor_hash: ANCHOR_HASH.to_owned(),
        balances: vec![(LEGACY_MAIN.to_owned(), 3), (SEGWIT_MAIN.to_owned(), 4)],
        final_canonical_hash: ANCHOR_HASH.to_owned(),
    }
}

#[test]
fn aggregate_state_identities_and_automatic_fact_descriptor_are_exact() {
    assert_eq!(
        CollectBitcoinBalancesState::name(),
        "mfm.bitcoin.collect_balances"
    );
    assert_eq!(
        CollectBitcoinBalancesState::version()
            .expect("state version")
            .as_str(),
        "mfm.bitcoin.state.collect_balances.v1"
    );
    let bindings = CollectBitcoinBalancesState::adapter_bindings().expect("adapter binding");
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].adapter_version.as_str(),
        "mfm.bitcoin.jsonrpc.adapter.v2"
    );
    assert_eq!(
        BitcoinBalanceSnapshotFact::descriptor()
            .expect("fact descriptor")
            .fact_kind()
            .as_str(),
        "bitcoin.balance_snapshot"
    );
}

#[test]
fn reducer_emits_one_ordered_fact_per_address_and_a_minimal_aligned_receipt() {
    let (receipt, facts) =
        reduce_bitcoin_balance_collection(&plan(), &evidence()).expect("reduction");

    assert_eq!(facts.values().len(), 2);
    assert_eq!(facts.values()[0].subject().address(), LEGACY_MAIN);
    assert_eq!(facts.values()[0].response().balance_sats(), 3);
    assert_eq!(facts.values()[1].subject().address(), SEGWIT_MAIN);
    assert_eq!(facts.values()[1].response().balance_sats(), 4);
    assert!(facts
        .values()
        .iter()
        .all(|fact| fact.response().anchor_hash() == ANCHOR_HASH));

    assert_eq!(receipt.network_id(), "bitcoin-mainnet");
    assert_eq!(receipt.bitcoin_network(), "main");
    assert_eq!(receipt.semantic_source_identity(), "public-bitcoin-core");
    assert_eq!(receipt.anchor_height(), 850_000);
    assert_eq!(receipt.anchor_hash(), ANCHOR_HASH);
    assert_eq!(receipt.addresses(), [LEGACY_MAIN, SEGWIT_MAIN]);
    assert_eq!(receipt.fact_content_identities().len(), 2);
}

#[test]
fn reducer_rejects_wrong_source_implementation_reorg_order_and_coverage() {
    let plan = plan();
    let mut cases = Vec::new();

    let mut wrong_source = evidence();
    wrong_source.semantic_source_identity = "other-source".to_owned();
    cases.push(wrong_source);

    let mut wrong_implementation = evidence();
    wrong_implementation.implementation_id = "mfm.bitcoin.jsonrpc.other.v1".to_owned();
    cases.push(wrong_implementation);

    let mut reorg = evidence();
    reorg.final_canonical_hash = OTHER_HASH.to_owned();
    cases.push(reorg);

    let mut reordered = evidence();
    reordered.balances.swap(0, 1);
    cases.push(reordered);

    let mut missing = evidence();
    missing.balances.pop();
    cases.push(missing);

    let mut extra = evidence();
    extra.balances.push(extra.balances[0].clone());
    cases.push(extra);

    let mut excessive = evidence();
    excessive.balances[0].1 = Amount::MAX_MONEY.to_sat() + 1;
    cases.push(excessive);

    for tampered in cases {
        assert!(reduce_bitcoin_balance_collection(&plan, &tampered).is_err());
    }
}

#[test]
fn response_decoder_requires_canonical_closed_domain_valid_json() {
    let canonical =
        format!(r#"{{"anchor_hash":"{ANCHOR_HASH}","anchor_height":850000,"balance_sats":7}}"#);
    let decoded =
        decode_bitcoin_balance_snapshot_response(canonical.as_bytes()).expect("canonical response");
    assert_eq!(decoded.anchor_height(), 850_000);
    assert_eq!(decoded.anchor_hash(), ANCHOR_HASH);
    assert_eq!(decoded.balance_sats(), 7);

    for invalid in [
        format!(r#"{{"anchor_height":850000,"anchor_hash":"{ANCHOR_HASH}","balance_sats":7}}"#),
        format!(
            r#"{{"anchor_hash":"{ANCHOR_HASH}","anchor_height":850000,"balance_sats":7,"extra":true}}"#
        ),
        r#"{"anchor_hash":"invalid","anchor_height":850000,"balance_sats":7}"#.to_owned(),
        format!(
            r#"{{"anchor_hash":"{ANCHOR_HASH}","anchor_height":850000,"balance_sats":{}}}"#,
            Amount::MAX_MONEY.to_sat() + 1
        ),
        format!("{canonical} true"),
    ] {
        assert!(decode_bitcoin_balance_snapshot_response(invalid.as_bytes()).is_err());
    }
}

#[test]
fn pure_bitcoin_manifest_stays_inside_domain_boundaries() {
    let manifest = include_str!("../Cargo.toml");

    for forbidden in [
        "mfm-app",
        "mfm-runtime",
        "mfm-replay",
        "mfm-store",
        "mfm-storage-postgres",
        "mfm-bitcoin-live",
        "mfm-keystore",
        "reqwest",
        "tokio",
        "url",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "pure Bitcoin crate must not depend on forbidden boundary crate {forbidden}"
        );
    }
}
