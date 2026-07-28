use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use mfm_app::{
    application_for_test, AccessPolicyError, AccessTarget, AdmitRunRequest, AuthorizedTenant,
    PublicJsonResponse, RunAccessGrant, RunAccessPolicy, SecretCredential, TestApplicationMode,
};
use mfm_ids::{ContentDigest, ContentRef, SchemaId};
use mfm_portfolio::{
    portfolio_snapshot_entry_point_contract, portfolio_snapshot_public_output_schema_id,
    PortfolioSnapshotSelector,
};
use mfm_values::MfmValue;

struct CountingPolicy {
    calls: AtomicUsize,
}

#[async_trait]
impl RunAccessPolicy for CountingPolicy {
    async fn authorize(
        &self,
        _credential: &SecretCredential,
        _grant: RunAccessGrant,
        _target: &AccessTarget,
    ) -> Result<AuthorizedTenant, AccessPolicyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(AccessPolicyError::AuthenticationRequired)
    }
}

#[test]
fn composed_application_caches_one_exact_entry_point_without_policy_work() {
    let expected = portfolio_snapshot_entry_point_contract(
        content_ref("planner-contract", '1'),
        content_ref("planner-implementation", '2'),
    )
    .expect("portfolio entry-point contract");
    let policy = Arc::new(CountingPolicy {
        calls: AtomicUsize::new(0),
    });
    let application = application_for_test(
        policy.clone(),
        vec![expected],
        TestApplicationMode::Sentinel,
    );

    let entries = application.entry_points();
    let repeated = application.entry_points();
    assert_eq!(repeated, entries, "cached discovery must be deterministic");
    assert_eq!(
        repeated.as_ptr(),
        entries.as_ptr(),
        "repeated discovery must borrow the cached slice"
    );
    let cloned_facade = application.clone();
    assert_eq!(
        cloned_facade.entry_points().as_ptr(),
        entries.as_ptr(),
        "facade clones must share the same sealed catalog"
    );
    assert_eq!(
        policy.calls.load(Ordering::SeqCst),
        0,
        "discovery must perform no authorization"
    );
    assert_eq!(entries.len(), 1, "v1 publishes exactly one entry point");

    let entry = &entries[0];
    assert_eq!(entry.entry_point_id().as_str(), "mfm.portfolio/snapshot@1");
    assert_eq!(
        entry.entry_point_operation_id().as_str(),
        "mfm.portfolio/snapshot"
    );

    let profile = entry.planning_profile();
    assert!(
        profile.framework_policy_refs().is_empty(),
        "the exact v1 profile has no framework policies"
    );
    assert_eq!(
        profile.canonical_profile_parameters().as_json(),
        &serde_json::json!({}),
        "the exact v1 profile has an empty parameter object"
    );
    assert_eq!(
        entry.planning_profile_ref(),
        &profile.content_ref().expect("planning-profile content ref")
    );
    assert_eq!(
        entry.input_schema_id(),
        &PortfolioSnapshotSelector::schema_id().expect("portfolio selector schema")
    );
    assert_eq!(
        entry.public_output_schema_id(),
        &portfolio_snapshot_public_output_schema_id().expect("portfolio public-output schema")
    );

    let rendered = entries.public_json().expect("render entry-point contracts");
    assert_eq!(rendered[0]["version"], "mfm.entry-point-contract.v1");
    assert_eq!(
        rendered[0]["planning_profile"]["version"],
        "mfm.planning-profile.v1"
    );
}

fn content_ref(name: &str, digest_byte: char) -> ContentRef {
    let schema_id = SchemaId::parse(format!(
        "schema:mfm.test.{name}:1:sha256-jcs-v1:{}",
        digest_byte.to_string().repeat(64)
    ))
    .expect("test schema id");
    let content_digest = ContentDigest::parse(format!(
        "content:sha256-v1:{}",
        digest_byte.to_string().repeat(64)
    ))
    .expect("test content digest");
    ContentRef::new(schema_id, content_digest).expect("test content ref")
}

#[test]
fn admission_request_is_exact_and_cannot_select_a_tenant() {
    let valid = br#"{
        "version":"mfm.admit-run-request.v1",
        "entry_point_id":"mfm.portfolio/snapshot@1",
        "invocation_identity":"de305d54-75b4-431b-adb2-eb6b9e546014",
        "input":{"target":"portfolio-name"}
    }"#;
    let request = AdmitRunRequest::decode_json(valid).expect("exact admission request");
    assert_eq!(
        request.as_bytes(),
        br#"{"entry_point_id":"mfm.portfolio/snapshot@1","input":{"target":"portfolio-name"},"invocation_identity":"de305d54-75b4-431b-adb2-eb6b9e546014","version":"mfm.admit-run-request.v1"}"#
    );

    for invalid in [
        br#"{"entry_point_id":"mfm.portfolio/snapshot@1","input":{"target":"portfolio-name"},"invocation_identity":"de305d54-75b4-431b-adb2-eb6b9e546014","tenant_scope_id":"mfm.tenant_scope.v1:11111111111111111111111111111111","version":"mfm.admit-run-request.v1"}"#.as_slice(),
        br#"{"entry_point_id":"mfm.portfolio/snapshot@1","input":{"target":"portfolio-name"},"invocation_identity":"DE305D54-75B4-431B-ADB2-EB6B9E546014","version":"mfm.admit-run-request.v1"}"#.as_slice(),
        br#"{"entry_point_id":"mfm.portfolio/snapshot@1","input":{"target":"portfolio-name"},"invocation_identity":"de305d54-75b4-131b-adb2-eb6b9e546014","version":"mfm.admit-run-request.v1"}"#.as_slice(),
        br#"{"entry_point_id":"mfm.portfolio/snapshot@1","input":{"weight":1.5},"invocation_identity":"de305d54-75b4-431b-adb2-eb6b9e546014","version":"mfm.admit-run-request.v1"}"#.as_slice(),
        br#"{"entry_point_id":"mfm.portfolio/snapshot@1","entry_point_id":"mfm.portfolio/snapshot@1","input":{"target":"portfolio-name"},"invocation_identity":"de305d54-75b4-431b-adb2-eb6b9e546014","version":"mfm.admit-run-request.v1"}"#.as_slice(),
    ] {
        assert!(AdmitRunRequest::decode_json(invalid).is_err());
    }
}
