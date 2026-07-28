use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, ContentRef, StableId};
use mfm_spec::{
    exact_content_ref, schema_id, CanonicalExpansionPath, CanonicalExpansionStep,
    CanonicalJsonValue, CertifiedJournalProtocolContracts, PlanningProfile,
    StateImplementationManifest, StateImplementationManifestEntry,
};

fn stable(value: &str) -> StableId {
    StableId::new(value).expect("fixture stable id")
}

#[test]
fn retained_value_contract_is_one_shared_rust_type() {
    let _: fn(mfm_spec::RetainedValueContract) -> mfm_values::RetainedValueContract =
        std::convert::identity;
    let _: fn(mfm_journal::v1::RetainedValueContract) -> mfm_values::RetainedValueContract =
        std::convert::identity;
}

fn fixture_ref(label: &str) -> ContentRef {
    let json =
        serde_json::to_string(&serde_json::json!({ "fixture": label })).expect("fixture json");
    let bytes = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical fixture");
    exact_content_ref(
        schema_id("mfm.component-implementation-descriptor.v1").expect("registered schema"),
        &bytes,
    )
    .expect("fixture content ref")
}

fn base() -> Vec<CanonicalExpansionStep> {
    vec![
        CanonicalExpansionStep::EntryPoint,
        CanonicalExpansionStep::Authored {
            stable_key: stable("state"),
            ordinal: 0,
        },
    ]
}

#[test]
fn expansion_paths_enforce_framework_outer_and_one_executor_occurrence() {
    let policy_a = fixture_ref("policy.a");
    let policy_b = fixture_ref("policy.b");
    let executor = fixture_ref("executor");
    let mut valid = base();
    valid.extend([
        CanonicalExpansionStep::FrameworkProtected {
            policy_ref: policy_a.clone(),
            policy_ordinal: 0,
        },
        CanonicalExpansionStep::FrameworkPre {
            policy_ref: policy_b,
            policy_ordinal: 1,
            state_ordinal: 0,
        },
        CanonicalExpansionStep::ExecutorProtected {
            executor_contract_ref: executor.clone(),
        },
    ]);
    CanonicalExpansionPath::new(valid).expect("valid layered path");

    let mut framework_after_terminal = base();
    framework_after_terminal.extend([
        CanonicalExpansionStep::FrameworkPre {
            policy_ref: policy_a.clone(),
            policy_ordinal: 0,
            state_ordinal: 0,
        },
        CanonicalExpansionStep::FrameworkProtected {
            policy_ref: policy_a,
            policy_ordinal: 1,
        },
    ]);
    assert!(CanonicalExpansionPath::new(framework_after_terminal).is_err());

    let mut multiple_executor_occurrences = base();
    multiple_executor_occurrences.extend([
        CanonicalExpansionStep::ExecutorPre {
            executor_contract_ref: executor.clone(),
            state_ordinal: 0,
        },
        CanonicalExpansionStep::ExecutorPost {
            executor_contract_ref: executor,
            state_ordinal: 0,
        },
    ]);
    assert!(CanonicalExpansionPath::new(multiple_executor_occurrences).is_err());
}

#[test]
fn frozen_values_reject_partial_or_noncanonical_inputs() {
    assert!(StateImplementationManifest::new(Vec::new()).is_err());

    let profile = PlanningProfile::new(
        fixture_ref("planner.contract"),
        fixture_ref("planner.implementation"),
        Vec::new(),
        CanonicalJsonValue::empty_object(),
    )
    .expect("planning profile");
    let canonical = profile.canonical_json().expect("profile bytes");
    assert_eq!(
        PlanningProfile::from_canonical_json(canonical.as_bytes()).expect("strict profile decode"),
        profile,
    );
    let spaced = format!(" {}", canonical.as_str());
    assert!(PlanningProfile::from_canonical_json(spaced.as_bytes()).is_err());
}

#[test]
fn state_manifest_uses_the_frozen_content_ref_wire_order() {
    let schema_first_but_wire_last = ContentRef::new(
        schema_id("mfm.access-audit-entry.v1").expect("first registered schema"),
        ContentDigest::parse(format!("content:sha256-v1:{}", "ff".repeat(32)))
            .expect("last content digest"),
    )
    .expect("first-schema state ref");
    let schema_last_but_wire_first = ContentRef::new(
        schema_id("mfm.value-ref.v1").expect("last registered schema"),
        ContentDigest::parse(format!("content:sha256-v1:{}", "00".repeat(32)))
            .expect("first content digest"),
    )
    .expect("last-schema state ref");
    assert!(schema_first_but_wire_last < schema_last_but_wire_first);

    let manifest = StateImplementationManifest::new(vec![
        StateImplementationManifestEntry {
            state_contract_ref: schema_first_but_wire_last,
            component_implementation_ref: fixture_ref("implementation.first"),
        },
        StateImplementationManifestEntry {
            state_contract_ref: schema_last_but_wire_first.clone(),
            component_implementation_ref: fixture_ref("implementation.second"),
        },
    ])
    .expect("wire-ordered state manifest");

    assert_eq!(
        manifest.entries()[0].state_contract_ref,
        schema_last_but_wire_first
    );
    manifest
        .canonical_json()
        .expect("manifest must satisfy the frozen schema-selected order");
}

#[test]
fn journal_protocol_contracts_are_factory_exact_not_schema_only() {
    let current =
        CertifiedJournalProtocolContracts::current().expect("current journal protocol contracts");
    assert_eq!(
        current.input_manifest_contract(),
        &mfm_journal::v1::InputManifest::retained_contract().expect("input contract")
    );

    let original = current.input_manifest_contract();
    let altered = mfm_values::RetainedValueContract::new(
        original.schema_id().clone(),
        original.semantic_type_id().clone(),
        stable("altered-input-role"),
        original.media_type(),
        original.evidence_contract_ref().clone(),
    )
    .expect("same-schema altered contract");
    assert!(CertifiedJournalProtocolContracts::new(
        altered,
        current.fact_claim_envelope_contract().clone(),
        current.frozen_read_intent_contract().clone(),
    )
    .is_err());
}
