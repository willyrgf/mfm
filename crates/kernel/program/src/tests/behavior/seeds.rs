use super::*;

#[test]
fn duplicate_seed_keys_reject() {
    let error = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = launch_seed(1, "a");
        let second = launch_seed(2, "b");
        let _ = root.seed(SeedKey::new("same")?, first)?;
        let _ = root.seed(SeedKey::new("same")?, second)?;
        unreachable!("duplicate seed must reject before binding outputs")
    })
    .expect_err("duplicate seed rejects");

    assert_eq!(error, PlanError::DuplicateSeedKey("same".to_owned()));
}

#[test]
fn canonical_seed_from_canonical_json_accepts_matching_shape_and_rejects_mismatch() {
    let valid = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        "{\"amount\":7,\"label\":\"canonical\"}",
    )
    .expect("canonical json");
    let seed = CanonicalSeed::<LaunchValue>::from_canonical_json(valid).expect("seed");

    assert_eq!(seed.byte_len(), 32);
    assert_eq!(
        seed.content_digest().as_str(),
        "content:sha256-jcs-v1:2b2d92093ac043c94672798bbc5c79761eec80b4aed995400305e8f8a06927e2"
    );
    assert_eq!(
        seed.canonical_json().as_str(),
        "{\"amount\":7,\"label\":\"canonical\"}"
    );

    let invalid = mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{\"amount\":7}")
        .expect("canonical json");
    let error =
        CanonicalSeed::<LaunchValue>::from_canonical_json(invalid).expect_err("missing label");

    assert!(matches!(error, PlanError::Canonical(message) if message.contains("missing field")));
}

#[test]
fn root_scope_id_is_stable_for_key() {
    let left = build_root(ScopeKey::new("same").expect("scope key"), |root| {
        let seed = launch_seed(1, "a");
        let handle = root.seed(SeedKey::new("input")?, seed)?;
        let outputs = LaunchPublicOutputs { result: handle };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("left");
    let right = build_root(ScopeKey::new("same").expect("scope key"), |root| {
        let seed = launch_seed(2, "b");
        let handle = root.seed(SeedKey::new("input")?, seed)?;
        let outputs = LaunchPublicOutputs { result: handle };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("right");

    assert_eq!(left.root_scope_id(), right.root_scope_id());
}
