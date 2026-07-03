use std::collections::BTreeMap;

use mfm_canonical::{sha256_digest_bytes, CanonicalValue};
use mfm_ids::{ContentDigest, DigestAlgorithm, EventId, RunId, SchemaId};
use mfm_store::v1 as store;

use super::service::public_fact_ref_id_from_projection_entry_for_test;

const PUBLIC_FACT_PRIVATE_JSON_TOKENS_FOR_TEST: &[&str] = &[
    "artifact_id",
    "artifact_evidence_hash",
    "fact_descriptor_hash",
    "subject_material",
    "subject_material_hash",
    "response_hash",
    "source_run_id",
    "source_seq",
    "source_ordinal",
];

/// Public fact fixture containing Platform, Control, and RunPrivate facts for visibility tests.
#[derive(Clone)]
pub struct PublicFactVisibilityFixtureForTest {
    /// Store serving the fixture projection and retained descriptor artifact.
    pub store: store::AsyncInMemoryRunStore,
    /// Fact kind exposed by the fixture descriptor.
    pub fact_kind: String,
    /// Descriptor schema id usable as a public shape selector.
    pub shape: String,
    /// Public ref for the Platform fact.
    pub platform_public_ref: String,
    /// Public ref for the Control fact, which public routes must not resolve.
    pub control_public_ref: String,
    /// Internal tokens that public DTOs and errors must not expose.
    pub private_tokens: Vec<String>,
}

impl PublicFactVisibilityFixtureForTest {
    /// Builds a standard public fact visibility fixture.
    pub fn new() -> Self {
        let descriptor_fixture = store::test_support::fact_descriptor_projection_fixture_for_test(
            public_visibility_fact_descriptor(),
        )
        .expect("descriptor projection fixture");
        let platform = fact_projection_fixture(
            1,
            &descriptor_fixture.descriptor,
            descriptor_fixture.descriptor_hash.clone(),
            mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        );
        let control = fact_projection_fixture(
            2,
            &descriptor_fixture.descriptor,
            descriptor_fixture.descriptor_hash.clone(),
            mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Control),
        );
        let run_private = fact_projection_fixture(
            3,
            &descriptor_fixture.descriptor,
            descriptor_fixture.descriptor_hash.clone(),
            mfm_facts::FactVisibility::RunPrivate,
        );

        let platform_index = platform.index.clone().expect("platform index");
        let control_index = control.index.clone().expect("control index");
        let platform_public_ref =
            public_fact_ref_id_from_projection_entry_for_test(&platform_index)
                .expect("platform public ref")
                .as_str()
                .to_owned();
        let control_public_ref = public_fact_ref_id_from_projection_entry_for_test(&control_index)
            .expect("control public ref")
            .as_str()
            .to_owned();
        let private_tokens = vec![
            platform_index.source_run_id.as_str().to_owned(),
            platform_index.source_event_id.as_str().to_owned(),
            platform_index.artifact_id.as_str().to_owned(),
            platform_index.artifact_evidence_hash.as_str().to_owned(),
            platform_index.fact_descriptor_hash.as_str().to_owned(),
            platform_index.fact_key.as_str().to_owned(),
            platform_index.subject_material_hash.as_str().to_owned(),
            platform_index.response_hash.as_str().to_owned(),
            control_index.source_run_id.as_str().to_owned(),
            control_index.artifact_id.as_str().to_owned(),
            run_private
                .record
                .claim
                .subject()
                .subject_material_hash()
                .as_str()
                .to_owned(),
            "control".to_owned(),
            "run_private".to_owned(),
        ];

        let projection = store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_descriptors: BTreeMap::from([(
                descriptor_fixture.descriptor_hash.clone(),
                descriptor_fixture.projection,
            )]),
            fact_records: BTreeMap::from([
                (platform.record.fact_claim_id.clone(), platform.record),
                (control.record.fact_claim_id.clone(), control.record),
                (run_private.record.fact_claim_id.clone(), run_private.record),
            ]),
            fact_index_entries: BTreeMap::from([
                (platform_index.fact_claim_id.clone(), platform_index),
                (control_index.fact_claim_id.clone(), control_index),
            ]),
            fact_term_entries: BTreeMap::from_iter(
                platform
                    .terms
                    .into_iter()
                    .chain(control.terms)
                    .map(|term| ((term.fact_claim_id.clone(), term.field_id.clone()), term)),
            ),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("projection");

        let store = store::AsyncInMemoryRunStore::default();
        store
            .seed_projection_snapshot_for_test(projection, [descriptor_fixture.descriptor_artifact])
            .expect("seed public fact fixture projection");

        Self {
            store,
            fact_kind: descriptor_fixture
                .descriptor
                .fact_kind()
                .as_str()
                .to_owned(),
            shape: descriptor_fixture
                .descriptor
                .descriptor_schema_id()
                .as_str()
                .to_owned(),
            platform_public_ref,
            control_public_ref,
            private_tokens,
        }
    }

    /// Asserts a rendered public fact payload does not expose fixture-private material.
    pub fn assert_rendered_json_redacts_private_tokens(&self, rendered: &str) {
        assert_public_fact_json_redacts_private_tokens_for_test(
            rendered,
            self.private_tokens.iter().map(String::as_str),
        );
    }

    /// Asserts a JSON public fact payload does not expose fixture-private material.
    pub fn assert_json_redacts_private_tokens(&self, value: &serde_json::Value) {
        let rendered = serde_json::to_string(value).expect("render response");
        self.assert_rendered_json_redacts_private_tokens(&rendered);
    }
}

impl Default for PublicFactVisibilityFixtureForTest {
    fn default() -> Self {
        Self::new()
    }
}

/// Asserts a rendered public fact payload does not expose internal fact fields or tokens.
pub fn assert_public_fact_json_redacts_private_tokens_for_test<'a>(
    rendered: &str,
    extra_private_tokens: impl IntoIterator<Item = &'a str>,
) {
    for token in PUBLIC_FACT_PRIVATE_JSON_TOKENS_FOR_TEST
        .iter()
        .copied()
        .chain(extra_private_tokens)
    {
        assert!(
            !rendered.contains(token),
            "response exposed private token {token}: {rendered}"
        );
    }
}

fn public_visibility_fact_descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.app.test.public_fact_visibility").expect("kind"),
        schema_id("mfm.app.test.public_fact_visibility.descriptor", 1),
        schema_id("mfm.app.test.public_fact_visibility.subject", 2),
        schema_id("mfm.app.test.public_fact_visibility.response", 3),
        vec![
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("subject.account").expect("field"),
                mfm_facts::FactFieldValueType::String,
                mfm_facts::FactFieldExtraction::Subject(
                    mfm_facts::CanonicalValuePath::new("account").expect("extraction"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .required(),
            )
            .expect("subject field"),
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("result.amount").expect("field"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldExtraction::Response(
                    mfm_facts::CanonicalValuePath::new("amount").expect("extraction"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![
                        mfm_facts::FactQueryOperator::Equal,
                        mfm_facts::FactQueryOperator::GreaterThanOrEqual,
                    ],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .sortable()
                .required(),
            )
            .expect("result field"),
        ],
        vec![mfm_facts::FactOrderingPolicy::new(
            mfm_facts::FactOrderingName::new("result.amount.asc").expect("ordering"),
            vec![mfm_facts::FactOrderingTerm::new(
                mfm_facts::FactFieldId::new("result.amount").expect("field"),
                mfm_facts::SortDirection::Ascending,
                mfm_facts::NullOrdering::Last,
                false,
            )],
        )
        .expect("ordering")],
    )
    .expect("descriptor")
}

fn fact_projection_fixture(
    n: u8,
    descriptor: &mfm_facts::FactDescriptor,
    descriptor_hash: ContentDigest,
    visibility: mfm_facts::FactVisibility,
) -> store::test_support::FactProjectionFixtureForTest {
    store::test_support::fact_projection_fixture_for_test(
        descriptor,
        descriptor_hash,
        store::test_support::FactProjectionFixtureInputForTest {
            run_id: run_id(n),
            source_seq: n as u64,
            source_ordinal: 0,
            source_event_id: event_id(n),
            node_id: node_id(n),
            attempt_id: attempt_id(n),
            commit_id: store::CommitKey::new(format!("commit-{n}")).expect("commit"),
            store_commit_order: n as u64,
            recorded_at: "2026-07-02T00:00:00Z".to_owned(),
            observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
            visibility,
            subject: public_visibility_fact_subject(),
            response: public_visibility_fact_response(),
            request: None,
            response_schema_id: schema_id("mfm.app.test.public_fact_visibility.response", 3),
            response_artifact_id: None,
            producer: producer(),
        },
    )
    .expect("fact projection fixture")
}

fn public_visibility_fact_subject() -> CanonicalValue {
    CanonicalValue::object([(
        "account",
        CanonicalValue::String("public-account".to_owned()),
    )])
    .expect("subject")
}

fn public_visibility_fact_response() -> CanonicalValue {
    CanonicalValue::object([("amount", CanonicalValue::Unsigned(15))]).expect("response")
}

fn producer() -> mfm_facts::FactProducerProvenance {
    mfm_facts::FactProducerProvenance::new(
        mfm_ids::CapabilityKind::new(
            "mfm.app.test",
            "fact-read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"capability"),
        )
        .expect("capability kind"),
        mfm_ids::CapabilityVersion::new("mfm.app.test.fact_read.v1").expect("capability version"),
        mfm_ids::AdapterKind::new(
            "mfm.app.test",
            "fact-adapter",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"adapter"),
        )
        .expect("adapter kind"),
        mfm_ids::AdapterVersion::new("mfm.app.test.fact_adapter.v1").expect("adapter version"),
    )
}

fn schema_id(name: &str, n: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest_bytes(n)).expect("schema id")
}

fn run_id(n: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(n))
}

fn event_id(n: u8) -> EventId {
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(n))
}

fn node_id(n: u8) -> mfm_ids::NodeId {
    mfm_ids::NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(n))
}

fn attempt_id(n: u8) -> mfm_ids::AttemptId {
    mfm_ids::AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(n))
}

fn digest_bytes(n: u8) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array([n; 32])
}
