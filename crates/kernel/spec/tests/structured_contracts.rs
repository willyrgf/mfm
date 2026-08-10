use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId};
use mfm_spec::structured::{
    StructuredEffectEntryContract, StructuredFailureContract,
    StructuredSafeFailureDispositionContract, StructuredStateContract,
    StructuredStateExecutionContract,
};
use mfm_spec::{CanonicalJsonValue, PlanningProfile};

fn stable(value: &str) -> StableId {
    StableId::new(value).expect("fixture stable id")
}

fn fixture_ref(label: &str) -> ContentRef {
    let schema = SchemaId::new(
        &format!("mfm.test.{label}"),
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(format!("schema:{label}").as_bytes()),
    )
    .expect("fixture schema");
    ContentRef::new(
        schema,
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            mfm_canonical::sha256_digest_bytes(format!("value:{label}").as_bytes()),
        ),
    )
    .expect("fixture content reference")
}

#[test]
fn planning_profile_is_strict_and_binds_its_exact_bytes() {
    let profile = PlanningProfile::new(
        fixture_ref("expansion-contract"),
        fixture_ref("expansion-implementation"),
        Vec::new(),
        CanonicalJsonValue::empty_object(),
    )
    .expect("planning profile");
    let canonical = profile.canonical_json().expect("canonical profile");
    assert_eq!(
        PlanningProfile::from_canonical_json(canonical.as_bytes()).expect("strict decode"),
        profile,
    );
    assert!(
        PlanningProfile::from_canonical_json(format!(" {}", canonical.as_str()).as_bytes())
            .is_err()
    );
}

#[test]
fn live_state_contract_rejects_missing_or_mismatched_safe_failure_disposition() {
    let state = StructuredStateContract::new(
        stable("read-state"),
        StructuredStateExecutionContract::Read {
            capability_contract_ref: fixture_ref("capability"),
        },
        fixture_ref("input"),
        fixture_ref("output"),
        StructuredFailureContract::Never,
        StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {},
        None,
    )
    .expect("valid read state");

    let mut missing = serde_json::to_value(&state).expect("state JSON");
    missing
        .as_object_mut()
        .expect("state object")
        .remove("safe_failure_disposition");
    assert!(serde_json::from_value::<StructuredStateContract>(missing).is_err());

    let mut mismatched = serde_json::to_value(&state).expect("state JSON");
    mismatched["safe_failure_disposition"]["kind"] = serde_json::json!("not_applicable");
    let mismatched =
        serde_json::from_value::<StructuredStateContract>(mismatched).expect("shape-valid bytes");
    assert!(mismatched.validate().is_err());
}

#[test]
fn an_absorbing_entry_contract_rejects_a_zero_entry_budget_from_persisted_bytes() {
    // Persisted specification data is untrusted, so the illegal state must be
    // unrepresentable on the way in and not merely rejected at authoring time.
    let keyed = serde_json::json!({
        "kind": "entry_absorbing",
        "entry_key_contract_ref": fixture_ref("entry-key"),
        "max_entries": 3,
    });
    let decoded: StructuredEffectEntryContract =
        serde_json::from_value(keyed).expect("a positive entry budget decodes");
    let StructuredEffectEntryContract::EntryAbsorbing { max_entries, .. } = decoded else {
        panic!("absorbing entry contract")
    };
    assert_eq!(max_entries.get(), 3);

    for budget in [0, -1] {
        let hostile = serde_json::json!({
            "kind": "entry_absorbing",
            "entry_key_contract_ref": fixture_ref("entry-key"),
            "max_entries": budget,
        });
        assert!(
            serde_json::from_value::<StructuredEffectEntryContract>(hostile).is_err(),
            "an absorbing capability must admit at least one entry: {budget}"
        );
    }

    // Absorption without a retained entry key has no shape at all.
    let unkeyed = serde_json::json!({ "kind": "entry_absorbing", "max_entries": 3 });
    assert!(serde_json::from_value::<StructuredEffectEntryContract>(unkeyed).is_err());
}
