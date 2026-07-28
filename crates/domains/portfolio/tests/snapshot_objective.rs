use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId};
use mfm_portfolio::{
    portfolio_snapshot_entry_point_registration, portfolio_snapshot_routing_manifest_member_path,
    portfolio_snapshot_unit_config_member_path, portfolio_snapshot_value_contracts, PortfolioId,
    PortfolioPublicOutputs, PortfolioSnapshotEntryPointArtifacts, PortfolioSnapshotSelector,
    PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID, PORTFOLIO_SNAPSHOT_OPERATION_ID,
};
use mfm_program::unit_config_value_contract;
use mfm_values::{MfmValue, PublicOutputDescriptor};
use serde_json::json;

fn content_ref(label: &'static [u8]) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.portfolio-product",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"portfolio product schema"),
        )
        .expect("schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            mfm_canonical::sha256_digest_bytes(label),
        ),
    )
    .expect("content ref")
}

#[test]
fn sole_public_registration_has_exact_roots_and_contracts() {
    let evidence = content_ref(b"object evidence");
    let unit = unit_config_value_contract(
        StableId::new("mfm.test.portfolio.unit-config").expect("unit role"),
        evidence.clone(),
    )
    .expect("unit contract");
    let registration = portfolio_snapshot_entry_point_registration(
        PortfolioSnapshotEntryPointArtifacts::new(
            content_ref(b"planner contract"),
            content_ref(b"planner implementation"),
            evidence.clone(),
            unit.clone(),
        )
        .expect("entry artifacts"),
    )
    .expect("entry registration");

    assert_eq!(
        registration.entry_point().entry_point_id().as_str(),
        PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID
    );
    assert_eq!(
        registration
            .entry_point()
            .entry_point_operation_id()
            .as_str(),
        PORTFOLIO_SNAPSHOT_OPERATION_ID
    );
    assert!(registration
        .entry_point()
        .planning_profile()
        .framework_policy_refs()
        .is_empty());
    assert_eq!(
        registration
            .entry_point()
            .planning_profile()
            .canonical_profile_parameters()
            .as_json(),
        &json!({})
    );

    let contracts = portfolio_snapshot_value_contracts(evidence).expect("value contracts");
    assert_eq!(
        registration
            .input_contract()
            .run_admission_contract()
            .root_contract(),
        contracts.selector()
    );
    assert_eq!(
        registration
            .input_contract()
            .configured_value_contract()
            .root_contract(),
        contracts.portfolio()
    );
    assert_eq!(
        registration.public_output_contract(),
        contracts.public_outputs()
    );
    assert_eq!(
        registration.public_output_contract().schema_id(),
        &PortfolioPublicOutputs::public_schema_id().expect("public output schema")
    );

    let roots = registration.authoring_root_requirements();
    assert_eq!(roots.len(), 2);
    assert_eq!(
        roots[0].member_path(),
        &portfolio_snapshot_unit_config_member_path().expect("unit path")
    );
    assert_eq!(roots[0].source_contract().root_contract(), &unit);
    assert_eq!(
        roots[1].member_path(),
        &portfolio_snapshot_routing_manifest_member_path().expect("routing path")
    );
    assert_eq!(
        roots[1].source_contract().root_contract(),
        contracts.routing_manifest()
    );
}

#[test]
fn public_selector_is_only_one_checked_portfolio_target() {
    let selector =
        PortfolioSnapshotSelector::new(PortfolioId::new("portfolio-main").expect("target"));
    assert_eq!(selector.target().as_str(), "portfolio-main");
    assert_eq!(
        serde_json::to_value(&selector).expect("selector JSON"),
        json!({"target": "portfolio-main"})
    );
    assert_eq!(
        PortfolioSnapshotSelector::schema_id().expect("selector schema"),
        portfolio_snapshot_value_contracts(content_ref(b"evidence"))
            .expect("contracts")
            .selector()
            .schema_id()
            .clone()
    );
    assert!(serde_json::from_value::<PortfolioSnapshotSelector>(json!({
        "target": "portfolio-main",
        "routing_generation": "forbidden"
    }))
    .is_err());
}
