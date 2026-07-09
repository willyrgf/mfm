//! Holding fact admission fixtures via the real FactRecorded projection path
//! (store test_support), not SQL fact_index pokes.

use mfm_canonical::CanonicalValue;
use mfm_facts::{
    fact_descriptor_hash, CoverageStatus, FactAudience, FactFieldId, FactProducerProvenance,
    FactVisibility, HoldingSourceStatus,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId, SchemaId,
};
use mfm_program::MfmFactType;
use mfm_states_btc::{
    BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact, BtcAddressBalanceSubject,
};
use mfm_states_evm::{
    EvmAddressNativeBalanceResponse, EvmAddressNativeBalanceSnapshotFact,
    EvmAddressNativeBalanceSubject,
};
use mfm_store::v1::{
    test_support::{
        fact_descriptor_projection_fixture_for_test, fact_projection_fixture_for_test,
        FactProjectionFixtureInputForTest,
    },
    CommitKey,
};

fn digest_bytes(seed: u8) -> DigestBytes {
    DigestBytes::from_array([seed; 32])
}

fn producer(
    capability_family: &str,
    capability_name: &str,
    capability_version: &str,
    adapter_family: &str,
    adapter_name: &str,
    adapter_version: &str,
    seed: u8,
) -> FactProducerProvenance {
    FactProducerProvenance::new(
        CapabilityKind::new(
            capability_family,
            capability_name,
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed),
        )
        .expect("capability kind"),
        CapabilityVersion::new(capability_version).expect("capability version"),
        AdapterKind::new(
            adapter_family,
            adapter_name,
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed + 1),
        )
        .expect("adapter kind"),
        AdapterVersion::new(adapter_version).expect("adapter version"),
    )
}

#[test]
fn btc_address_balance_admits_via_fact_recorded_projection() {
    let subject = BtcAddressBalanceSubject::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
    )
    .expect("subject");
    let response = BtcAddressBalanceResponse::new(
        100,
        "ab".repeat(32),
        21,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("response");
    let descriptor = BtcAddressBalanceSnapshotFact::descriptor().expect("descriptor");
    let descriptor_fixture =
        fact_descriptor_projection_fixture_for_test(descriptor.clone()).expect("descriptor");
    let descriptor_hash = fact_descriptor_hash(&descriptor).expect("hash");
    assert_eq!(descriptor_hash, descriptor_fixture.descriptor_hash);

    let subject_value = CanonicalValue::object([
        (
            "network",
            CanonicalValue::String(subject.network().to_owned()),
        ),
        (
            "bitcoin_network",
            CanonicalValue::String(subject.bitcoin_network().to_owned()),
        ),
        (
            "semantic_source_identity",
            CanonicalValue::String(subject.semantic_source_identity().to_owned()),
        ),
        (
            "address",
            CanonicalValue::String(subject.address().to_owned()),
        ),
    ])
    .expect("subject value");
    let response_value = CanonicalValue::object([
        (
            "anchor_height",
            CanonicalValue::Unsigned(response.anchor_height()),
        ),
        (
            "anchor_hash",
            CanonicalValue::String(response.anchor_hash().to_owned()),
        ),
        (
            "balance_sats",
            CanonicalValue::Unsigned(response.balance_sats()),
        ),
        (
            "coverage",
            CanonicalValue::String(response.coverage().to_owned()),
        ),
        (
            "source_status",
            CanonicalValue::String(response.source_status().to_owned()),
        ),
    ])
    .expect("response value");

    let fixture = fact_projection_fixture_for_test(
        &descriptor,
        descriptor_hash,
        FactProjectionFixtureInputForTest {
            run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x21)),
            source_seq: 3,
            source_ordinal: 0,
            source_event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x22)),
            node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x23)),
            attempt_id: AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x24)),
            commit_id: CommitKey::new("btc-address-balance-fixture").expect("commit"),
            store_commit_order: 17,
            recorded_at: "2026-07-02T00:00:00Z".to_owned(),
            observed_at: None,
            visibility: FactVisibility::indexed_default(FactAudience::Platform),
            subject: subject_value,
            response: response_value,
            request: None,
            response_schema_id: SchemaId::new(
                "mfm.bitcoin.fact.address_balance.response",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(0x25),
            )
            .expect("schema"),
            response_artifact_id: Some(ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(0x26),
            )),
            producer: producer(
                "mfm.bitcoin",
                "fact.record",
                "mfm.bitcoin.fact.record.v1",
                "mfm.bitcoin",
                "jsonrpc",
                "mfm.bitcoin.jsonrpc.adapter.v1",
                0x27,
            ),
        },
    )
    .expect("fact recorded projection fixture");

    let index = fixture.index.expect("platform indexed");
    assert_eq!(index.audience, FactAudience::Platform);
    assert_eq!(index.fact_kind.as_str(), "bitcoin.address_balance_snapshot");
    assert_eq!(index.store_commit_order, 17);
    assert!(fixture
        .terms
        .iter()
        .any(|term| term.field_id == FactFieldId::new("subject.address").expect("field")));
    assert!(fixture
        .terms
        .iter()
        .any(|term| term.field_id == FactFieldId::new("result.anchor_hash").expect("field")));
    assert_eq!(
        fixture.response_artifact_evidence.artifact_role.as_str(),
        "fact_response"
    );
}

#[test]
fn evm_native_balance_admits_via_fact_recorded_projection() {
    let subject = EvmAddressNativeBalanceSubject::new(
        "ethereum-mainnet",
        1,
        "0x0000000000000000000000000000000000000001",
    )
    .expect("subject");
    let response = EvmAddressNativeBalanceResponse::new(
        21_000_000,
        "0x".to_owned() + &"cd".repeat(32),
        "1000",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("response");
    let descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor().expect("descriptor");
    let descriptor_hash = fact_descriptor_hash(&descriptor).expect("hash");

    let subject_value = CanonicalValue::object([
        (
            "network",
            CanonicalValue::String(subject.network().to_owned()),
        ),
        ("chain_id", CanonicalValue::Unsigned(subject.chain_id())),
        (
            "account",
            CanonicalValue::String(subject.account().to_owned()),
        ),
    ])
    .expect("subject value");
    let response_value = CanonicalValue::object([
        (
            "block_number",
            CanonicalValue::Unsigned(response.block_number()),
        ),
        (
            "block_hash",
            CanonicalValue::String(response.block_hash().to_owned()),
        ),
        (
            "raw_wei",
            CanonicalValue::String(response.raw_wei().to_owned()),
        ),
        (
            "decimals",
            CanonicalValue::Unsigned(u64::from(response.decimals())),
        ),
        (
            "coverage",
            CanonicalValue::String(response.coverage().to_owned()),
        ),
        (
            "source_status",
            CanonicalValue::String(response.source_status().to_owned()),
        ),
    ])
    .expect("response value");

    let fixture = fact_projection_fixture_for_test(
        &descriptor,
        descriptor_hash,
        FactProjectionFixtureInputForTest {
            run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x41)),
            source_seq: 5,
            source_ordinal: 0,
            source_event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x42)),
            node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x43)),
            attempt_id: AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x44)),
            commit_id: CommitKey::new("evm-native-balance-fixture").expect("commit"),
            store_commit_order: 9,
            recorded_at: "2026-07-02T00:00:00Z".to_owned(),
            observed_at: None,
            visibility: FactVisibility::indexed_default(FactAudience::Platform),
            subject: subject_value,
            response: response_value,
            request: None,
            response_schema_id: SchemaId::new(
                "mfm.evm.fact.address_native_balance.response",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(0x45),
            )
            .expect("schema"),
            response_artifact_id: Some(ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(0x46),
            )),
            producer: producer(
                "mfm.evm",
                "fact.record",
                "mfm.evm.fact.record.v1",
                "mfm.evm",
                "rpc",
                "mfm.evm.rpc.adapter.v1",
                0x47,
            ),
        },
    )
    .expect("fact recorded projection fixture");

    let index = fixture.index.expect("platform indexed");
    assert_eq!(index.audience, FactAudience::Platform);
    assert_eq!(
        index.fact_kind.as_str(),
        "evm.address_native_balance_snapshot"
    );
    assert_eq!(index.store_commit_order, 9);
    assert!(fixture
        .terms
        .iter()
        .any(|term| term.field_id == FactFieldId::new("subject.account").expect("field")));
    assert!(fixture
        .terms
        .iter()
        .any(|term| term.field_id == FactFieldId::new("result.block_hash").expect("field")));
    assert_eq!(
        fixture.response_artifact_evidence.artifact_role.as_str(),
        "fact_response"
    );
}
