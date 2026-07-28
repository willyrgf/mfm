use std::collections::BTreeMap;

use mfm_canonical::CanonicalValue;
use mfm_ids::{ContentDigest, DigestAlgorithm, EventId, RunId, SchemaId};
use mfm_store::v1 as store;

use super::ref_id::public_ref_id;

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

/// Public fact fixture backed by one store-owned query projection.
#[derive(Clone)]
pub struct PublicFactFixtureForTest {
    /// Store serving the fixture projection and retained descriptor artifact.
    pub store: store::AsyncInMemoryRunStore,
    /// Fact kind exposed by the fixture descriptor.
    pub fact_kind: String,
    /// Descriptor schema id usable as a public shape selector.
    pub shape: String,
    /// Public ref for the projected fact.
    pub public_ref: String,
    /// Internal tokens that public DTOs and errors must not expose.
    pub private_tokens: Vec<String>,
}

impl PublicFactFixtureForTest {
    /// Builds a standard public fact fixture.
    pub fn new() -> Self {
        let descriptor_fixture = store::test_support::fact_descriptor_projection_fixture_for_test(
            public_fact_descriptor(),
        )
        .expect("descriptor projection fixture");
        let fact = fact_projection_fixture(
            1,
            &descriptor_fixture.descriptor,
            descriptor_fixture.descriptor_hash.clone(),
        );

        let public_ref = public_ref_id(
            &fact
                .projection
                .internal_ref()
                .expect("projected internal ref"),
        )
        .expect("public ref")
        .as_str()
        .to_owned();
        let private_tokens = vec![
            fact.projection.source_run_id().as_str().to_owned(),
            fact.projection.source_event_id().as_str().to_owned(),
            fact.projection.artifact_id().as_str().to_owned(),
            fact.projection.artifact_evidence_hash().as_str().to_owned(),
            fact.projection.fact_descriptor_hash().as_str().to_owned(),
            fact.projection.fact_key().as_str().to_owned(),
            fact.projection.subject_material_hash().as_str().to_owned(),
            fact.projection.response_hash().as_str().to_owned(),
        ];
        let fact_claim_id = fact.projection.fact_claim_id().clone();

        let projection = store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_descriptors: BTreeMap::from([(
                descriptor_fixture.descriptor_hash.clone(),
                descriptor_fixture.projection,
            )]),
            fact_query_entries: BTreeMap::from([(fact_claim_id, fact.projection)]),
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
            public_ref,
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

impl Default for PublicFactFixtureForTest {
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

fn public_fact_descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.app.test.public_fact").expect("kind"),
        schema_id("mfm.app.test.public_fact.descriptor", 1),
        schema_id("mfm.app.test.public_fact.subject", 2),
        schema_id("mfm.app.test.public_fact.response", 3),
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
            subject: public_fact_subject(),
            response: public_fact_response(),
            response_schema_id: schema_id("mfm.app.test.public_fact.response", 3),
            response_artifact_id: None,
        },
    )
    .expect("fact projection fixture")
}

fn public_fact_subject() -> CanonicalValue {
    CanonicalValue::object([(
        "account",
        CanonicalValue::String("public-account".to_owned()),
    )])
    .expect("subject")
}

fn public_fact_response() -> CanonicalValue {
    CanonicalValue::object([("amount", CanonicalValue::Unsigned(15))]).expect("response")
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
    mfm_ids::AttemptId::from_digest(digest_bytes(n))
}

fn digest_bytes(n: u8) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array([n; 32])
}
