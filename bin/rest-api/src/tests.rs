use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, EventId, TrustScopeId};
use mfm_spec::v1 as spec;
use std::collections::BTreeMap;
use tower::ServiceExt;

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001";
const VALID_SCHEMA_ID: &str =
    "schema:mfm.test.public:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000002";

struct FailingSerialize;

impl Serialize for FailingSerialize {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("boom"))
    }
}

#[test]
fn serialize_response_returns_api_error_instead_of_panicking() {
    let err = serialize_response(FailingSerialize)
        .expect_err("serialization failures should be returned as api errors");

    assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(err.code, "SerializationError");
    assert_eq!(err.message, "Failed to serialize response payload");
}

#[test]
fn invalid_run_id_has_domain_error() {
    let err = parse_run_id("not-a-uuid").expect_err("dynamic ids are rejected");

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.code, "InvalidRunId");
}

#[tokio::test]
async fn run_start_accepts_entry_point_shape() {
    let _env_guard = ENV_LOCK.lock().await;
    let response = test_app()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "op": "missing_entry_point_op",
                "config": "portfolio_id = \"main\"\n"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = response_json(response).await;
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "EntryPointOpNotFound");
}

#[tokio::test]
async fn read_only_routes_ignore_malformed_runtime_config_env() {
    let _env = locked_env([(
        mfm_app::MFM_RUNTIME_CONFIG_FILE,
        "/definitely/not/runtime.toml",
    )])
    .await;
    let app = test_app();

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/runs")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    assert_eq!(list.status(), StatusCode::OK);

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("health response");
    assert_eq!(health.status(), StatusCode::OK);

    let ready = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("ready response");
    assert_eq!(ready.status(), StatusCode::OK);

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{VALID_RUN_ID}/status"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("status response");
    assert_eq!(status.status(), StatusCode::NOT_FOUND);

    let stream = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{VALID_RUN_ID}/stream"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("stream response");
    assert_eq!(stream.status(), StatusCode::NOT_FOUND);

    let replay = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{VALID_RUN_ID}/replay"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::NOT_FOUND);

    let output = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/runs/{VALID_RUN_ID}/public-output/{VALID_SCHEMA_ID}"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("public output response");
    assert_eq!(output.status(), StatusCode::NOT_FOUND);

    let fact_kinds = app
        .clone()
        .oneshot(get("/v1/facts/kinds"))
        .await
        .expect("fact kinds response");
    assert_eq!(fact_kinds.status(), StatusCode::OK);

    let fact_describe = app
        .clone()
        .oneshot(get("/v1/facts/kinds/mfm.rest.test.fact"))
        .await
        .expect("fact describe response");
    assert_eq!(fact_describe.status(), StatusCode::NOT_FOUND);

    let fact_query = app
        .clone()
        .oneshot(get(
            "/v1/facts/mfm.rest.test.fact?order=result.amount.asc&field=result.amount&limit=1",
        ))
        .await
        .expect("fact query response");
    assert_eq!(fact_query.status(), StatusCode::NOT_FOUND);

    let fact_latest = app
        .clone()
        .oneshot(get(
            "/v1/facts/mfm.rest.test.fact/latest?order=result.amount.asc&field=result.amount",
        ))
        .await
        .expect("fact latest response");
    assert_eq!(fact_latest.status(), StatusCode::NOT_FOUND);

    let fact_ref = app
        .oneshot(get(&format!("/v1/facts/ref/{}", unknown_public_ref())))
        .await
        .expect("fact ref response");
    assert_eq!(fact_ref.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn facts_routes_are_available_and_return_json_envelopes() {
    let app = test_app();

    let kinds = app
        .clone()
        .oneshot(get("/v1/facts/kinds"))
        .await
        .expect("fact kinds response");
    assert_eq!(kinds.status(), StatusCode::OK);
    let value = response_json(kinds).await;
    assert_eq!(value["status"], "success");
    assert_eq!(value["data"], json!([]));

    let describe = app
        .clone()
        .oneshot(get("/v1/facts/kinds/mfm.rest.test.fact"))
        .await
        .expect("fact describe response");
    assert_eq!(describe.status(), StatusCode::NOT_FOUND);
    let value = response_json(describe).await;
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "FactNotFound");

    let ref_lookup = app
        .oneshot(get(&format!("/v1/facts/ref/{}", unknown_public_ref())))
        .await
        .expect("fact ref response");
    assert_eq!(ref_lookup.status(), StatusCode::NOT_FOUND);
    let value = response_json(ref_lookup).await;
    assert_eq!(value["error"]["code"], "FactNotFound");
}

#[tokio::test]
async fn fact_query_routes_reject_malformed_query_shapes() {
    let app = test_app();

    let bad_limit = app
        .clone()
        .oneshot(get(
            "/v1/facts/mfm.rest.test.fact?order=result.amount.asc&field=result.amount&limit=nope",
        ))
        .await
        .expect("bad limit response");
    assert_eq!(bad_limit.status(), StatusCode::BAD_REQUEST);
    let value = response_json(bad_limit).await;
    assert_eq!(value["error"]["code"], "InvalidQuery");

    let missing_order = app
        .clone()
        .oneshot(get("/v1/facts/mfm.rest.test.fact?field=result.amount"))
        .await
        .expect("missing order response");
    assert_eq!(missing_order.status(), StatusCode::BAD_REQUEST);
    let value = response_json(missing_order).await;
    assert_eq!(value["error"]["code"], "FactOrderingMissing");

    let missing_field = app
        .clone()
        .oneshot(get("/v1/facts/mfm.rest.test.fact?order=result.amount.asc"))
        .await
        .expect("missing field response");
    assert_eq!(missing_field.status(), StatusCode::BAD_REQUEST);
    let value = response_json(missing_field).await;
    assert_eq!(value["error"]["code"], "FactReturnFieldMissing");

    let malformed_predicate = app
        .oneshot(get(
            "/v1/facts/mfm.rest.test.fact/latest?order=result.amount.asc&field=result.amount&subject=broken",
        ))
        .await
        .expect("malformed predicate response");
    assert_eq!(malformed_predicate.status(), StatusCode::BAD_REQUEST);
    let value = response_json(malformed_predicate).await;
    assert_eq!(value["error"]["code"], "FactPredicateInvalid");
}

#[tokio::test]
async fn facts_routes_expose_only_public_platform_projection_data() {
    let fixture = FactRouteFixture::new();
    let app = make_app(AppState {
        store: fixture.store.clone(),
        runtime_config_path: None,
    });

    let kinds = app
        .clone()
        .oneshot(get("/v1/facts/kinds"))
        .await
        .expect("fact kinds response");
    assert_eq!(kinds.status(), StatusCode::OK);
    let value = response_json(kinds).await;
    assert_eq!(value["data"][0]["fact_kind"], "mfm.rest.test.fact");
    assert_eq!(value["data"][0]["descriptor_count"], 1);
    assert_private_tokens_absent(&value, &fixture);

    let described = app
        .clone()
        .oneshot(get("/v1/facts/kinds/mfm.rest.test.fact"))
        .await
        .expect("fact describe response");
    assert_eq!(described.status(), StatusCode::OK);
    let value = response_json(described).await;
    assert_eq!(value["data"][0]["fields"].as_array().unwrap().len(), 2);
    assert_private_tokens_absent(&value, &fixture);

    let queried = app
        .clone()
        .oneshot(get(
            "/v1/facts/mfm.rest.test.fact?shape=mfm.rest.test.fact.v1&order=result.amount.asc&field=subject.account&field=result.amount&subject=account%3Dpublic-account&result=amount.gte%3Du64%3A10&limit=10",
        ))
        .await
        .expect("fact query response");
    assert_eq!(queried.status(), StatusCode::OK);
    let value = response_json(queried).await;
    let facts = value["data"]["facts"].as_array().expect("facts array");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["public_ref"], fixture.platform_public_ref);
    assert_eq!(facts[0]["fields"].as_array().unwrap().len(), 2);
    assert_private_tokens_absent(&value, &fixture);

    let latest = app
        .clone()
        .oneshot(get(
            "/v1/facts/mfm.rest.test.fact/latest?shape=mfm.rest.test.fact.v1&order=result.amount.asc&field=result.amount&limit=99",
        ))
        .await
        .expect("fact latest response");
    assert_eq!(latest.status(), StatusCode::OK);
    let value = response_json(latest).await;
    assert_eq!(value["data"]["facts"].as_array().unwrap().len(), 1);
    assert_private_tokens_absent(&value, &fixture);

    let resolved = app
        .clone()
        .oneshot(get(&format!(
            "/v1/facts/ref/{}",
            fixture.platform_public_ref
        )))
        .await
        .expect("fact ref response");
    assert_eq!(resolved.status(), StatusCode::OK);
    let value = response_json(resolved).await;
    assert_eq!(value["data"]["public_ref"], fixture.platform_public_ref);
    assert_private_tokens_absent(&value, &fixture);

    let control = app
        .clone()
        .oneshot(get(&format!(
            "/v1/facts/ref/{}",
            fixture.control_public_ref
        )))
        .await
        .expect("control ref response");
    assert_eq!(control.status(), StatusCode::NOT_FOUND);
    let control_error = response_json(control).await;
    assert_eq!(control_error["error"]["code"], "FactNotFound");
    assert_private_tokens_absent(&control_error, &fixture);

    let unknown = app
        .oneshot(get(&format!("/v1/facts/ref/{}", unknown_public_ref())))
        .await
        .expect("unknown ref response");
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let unknown_error = response_json(unknown).await;
    assert_eq!(unknown_error["error"], control_error["error"]);
}

async fn locked_env<const N: usize>(pairs: [(&'static str, &str); N]) -> EnvGuard {
    let guard = ENV_LOCK.lock().await;
    let mut previous = Vec::new();
    for (key, value) in pairs {
        previous.push((key, std::env::var(key).ok()));
        set_env(key, value);
    }
    EnvGuard {
        _guard: guard,
        previous,
    }
}

struct EnvGuard {
    _guard: tokio::sync::MutexGuard<'static, ()>,
    previous: Vec<(&'static str, Option<String>)>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => set_env(key, value),
                None => remove_env(key),
            }
        }
    }
}

fn set_env(key: &str, value: &str) {
    // SAFETY: these tests serialize environment mutation through ENV_LOCK and
    // restore each variable before releasing that lock.
    unsafe { std::env::set_var(key, value) };
}

fn remove_env(key: &str) {
    // SAFETY: these tests serialize environment mutation through ENV_LOCK and
    // restore each variable before releasing that lock.
    unsafe { std::env::remove_var(key) };
}

fn test_app() -> axum::Router {
    make_app(AppState {
        store: store::AsyncInMemoryRunStore::default(),
        runtime_config_path: None,
    })
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("response json")
}

#[derive(Clone)]
struct FactFixtureStore {
    inner: store::AsyncInMemoryRunStore,
    projection: store::ProjectionSnapshot,
    descriptor_artifact: store::VerifiedRunArtifactBytes,
}

impl store::RunEventStore for FactFixtureStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        self.inner.append_prepared_commit_bundle(bundle)
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        self.inner.load_committed_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let projection = self.projection.clone();
        Box::pin(std::future::ready(Ok(projection)))
    }
}

impl store::TrustScopeStore for FactFixtureStore {
    type Error = store::StoreError;

    fn load_trust_scope_id<'a>(&'a self) -> store::AsyncStoreFuture<'a, TrustScopeId, Self::Error> {
        self.inner.load_trust_scope_id()
    }
}

impl store::ExecutionClaimStore for FactFixtureStore {
    type Error = store::StoreError;

    fn acquire_execution_claim<'a>(
        &'a self,
        run_id: &'a RunId,
        token: store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, store::NowaitSkipAdmissionResult, Self::Error> {
        self.inner.acquire_execution_claim(run_id, token)
    }

    fn execution_claim_status<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ExecutionClaimStatus, Self::Error> {
        self.inner.execution_claim_status(run_id)
    }

    fn renew_execution_claim<'a>(
        &'a self,
        run_id: &'a RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, Option<store::AdmissionLease>, Self::Error> {
        self.inner.renew_execution_claim(run_id, token)
    }

    fn release_execution_claim<'a>(
        &'a self,
        run_id: &'a RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
        self.inner.release_execution_claim(run_id, token)
    }

    fn expired_execution_claims<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, Vec<store::ExpiredExecutionClaim>, Self::Error> {
        self.inner.expired_execution_claims()
    }

    fn reap_expired_execution_claim<'a>(
        &'a self,
        run_id: &'a RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
        self.inner.reap_expired_execution_claim(run_id, token)
    }
}

impl store::RunObservationStore for FactFixtureStore {
    type Error = store::StoreError;

    fn read_run_observations<'a>(
        &'a self,
        query: store::RunObservationQuery,
    ) -> store::AsyncStoreFuture<'a, store::RunObservationPage, Self::Error> {
        self.inner.read_run_observations(query)
    }
}

impl store::RetainedArtifactReadProvider for FactFixtureStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        let artifact = self.descriptor_artifact.clone();
        let result = if requirement.artifact_id == artifact.evidence().artifact_id {
            Ok(artifact)
        } else {
            Err(store::StoreError::ArtifactEvidenceMismatch {
                artifact_id: requirement.artifact_id.clone(),
                field: "artifact_id",
            })
        };
        Box::pin(std::future::ready(result))
    }
}

impl mfm_app::PublicFactQueryExecutor for FactFixtureStore {
    fn execute_public_fact_query<'a>(
        &'a self,
        _catalog: &'a mfm_app::FactCatalogService,
        _store_scope: &'a mfm_facts::StoreScopeRef,
        _scope_decision_evidence: &'a mfm_facts::ScopeDecisionEvidence,
        request: mfm_app::PublicFactQueryRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<mfm_app::PublicFactQueryPage, AppError>>
                + Send
                + 'a,
        >,
    > {
        let projection = self.projection.clone();
        Box::pin(async move {
            if request.fact_kind != "mfm.rest.test.fact" {
                return Err(AppError::not_found(
                    "FactNotFound",
                    "Fact was not found or is not available through the public fact service",
                ));
            }
            let facts = projection
                .fact_index_entries()
                .filter(|(_claim_id, entry)| {
                    entry.audience == mfm_facts::FactAudience::Platform
                        && entry.visibility_scope == mfm_facts::FactVisibilityScope::Default
                })
                .map(|(_claim_id, entry)| public_fact_from_projection(&projection, entry))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(mfm_app::PublicFactQueryPage {
                facts,
                next_cursor: None,
            })
        })
    }
}

struct FactRouteFixture {
    store: FactFixtureStore,
    platform_public_ref: String,
    control_public_ref: String,
    private_tokens: Vec<String>,
}

impl FactRouteFixture {
    fn new() -> Self {
        let descriptor = rest_fact_descriptor();
        let descriptor_bytes =
            mfm_facts::canonical_fact_descriptor_bytes(&descriptor).expect("descriptor bytes");
        let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor).expect("hash");
        let descriptor_artifact_id =
            ArtifactId::from_digest(descriptor_hash.algorithm(), *descriptor_hash.digest());
        let descriptor_evidence = store::ArtifactEvidenceRef {
            artifact_id: descriptor_artifact_id.clone(),
            digest: descriptor_hash.clone(),
            byte_len: descriptor_bytes.as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactDescriptor,
        };
        let descriptor_requirement = store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::FactDescriptor,
            artifact_id: descriptor_artifact_id.clone(),
            digest: Some(descriptor_hash.clone()),
            byte_len: None,
            media_type: Some(spec::MediaType::new("application/json").expect("media")),
            schema_id: Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: Some(events::ArtifactRole::FactDescriptor),
        };
        let descriptor_artifact = store::VerifiedRunArtifactBytes::new(
            descriptor_bytes.to_vec(),
            descriptor_evidence.clone(),
            &descriptor_requirement,
        )
        .expect("descriptor artifact");

        let namespace_hash = mfm_facts::fact_subject_namespace(&descriptor)
            .and_then(|namespace| mfm_facts::fact_subject_namespace_hash(&namespace))
            .expect("namespace hash");
        let platform = fact_projection_row(
            1,
            descriptor_hash.clone(),
            namespace_hash.clone(),
            mfm_facts::FactAudience::Platform,
        );
        let control = fact_projection_row(
            2,
            descriptor_hash.clone(),
            namespace_hash.clone(),
            mfm_facts::FactAudience::Control,
        );
        let run_private = private_fact_record(3, descriptor_hash.clone(), namespace_hash.clone());

        let platform_public_ref = public_ref_for_claim(&platform.index.fact_claim_id);
        let control_public_ref = public_ref_for_claim(&control.index.fact_claim_id);
        let private_tokens = vec![
            platform.index.source_run_id.as_str().to_owned(),
            platform.index.source_event_id.as_str().to_owned(),
            platform.index.artifact_id.as_str().to_owned(),
            platform.index.artifact_evidence_hash.as_str().to_owned(),
            platform.index.fact_descriptor_hash.as_str().to_owned(),
            platform.index.fact_key.as_str().to_owned(),
            platform.index.subject_material_hash.as_str().to_owned(),
            platform.index.response_hash.as_str().to_owned(),
            control.index.source_run_id.as_str().to_owned(),
            control.index.artifact_id.as_str().to_owned(),
            run_private
                .claim
                .subject()
                .subject_material_hash()
                .as_str()
                .to_owned(),
            "control".to_owned(),
            "run_private".to_owned(),
            "source_seq".to_owned(),
            "source_ordinal".to_owned(),
            "artifact_evidence_hash".to_owned(),
            "subject_material".to_owned(),
            "subject_material_hash".to_owned(),
        ];

        let projection = store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_descriptors: BTreeMap::from([(
                descriptor_hash.clone(),
                store::FactDescriptorProjection {
                    descriptor_hash: descriptor_hash.clone(),
                    descriptor_artifact_id,
                    descriptor_artifact_evidence: descriptor_evidence,
                    fact_kind: descriptor.fact_kind().clone(),
                    descriptor_schema_id: descriptor.descriptor_schema_id().clone(),
                    subject_schema_id: descriptor.subject_schema_id().clone(),
                    response_schema_id: descriptor.response_schema_id().clone(),
                    fact_subject_namespace_hash: namespace_hash,
                    source_event_id: event_id(90),
                },
            )]),
            fact_records: BTreeMap::from([
                (platform.record.fact_claim_id.clone(), platform.record),
                (control.record.fact_claim_id.clone(), control.record),
                (run_private.fact_claim_id.clone(), run_private),
            ]),
            fact_index_entries: BTreeMap::from([
                (platform.index.fact_claim_id.clone(), platform.index),
                (control.index.fact_claim_id.clone(), control.index),
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

        Self {
            store: FactFixtureStore {
                inner: store::AsyncInMemoryRunStore::default(),
                projection,
                descriptor_artifact,
            },
            platform_public_ref,
            control_public_ref,
            private_tokens,
        }
    }
}

struct IndexedFactFixture {
    record: store::FactRecordProjection,
    index: store::FactIndexProjection,
    terms: Vec<store::FactIndexTermProjection>,
}

fn rest_fact_descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.rest.test.fact").expect("kind"),
        schema_id("mfm.rest.test.fact.descriptor", 1),
        schema_id("mfm.rest.test.fact.subject", 2),
        schema_id("mfm.rest.test.fact.response", 3),
        vec![
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("subject.account").expect("field"),
                mfm_facts::FactFieldPath::new("subject.account").expect("path"),
                mfm_facts::FactFieldValueType::String,
                mfm_facts::FactFieldAccessor::SubjectPath(
                    mfm_facts::CanonicalValuePath::new("account").expect("accessor"),
                ),
                vec![mfm_facts::FactQueryOperator::Equal],
                mfm_facts::FactFieldExposure::Returnable,
                None,
                None,
                false,
                true,
            )
            .expect("subject field"),
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("result.amount").expect("field"),
                mfm_facts::FactFieldPath::new("result.amount").expect("path"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldAccessor::ResponsePath(
                    mfm_facts::CanonicalValuePath::new("amount").expect("accessor"),
                ),
                vec![
                    mfm_facts::FactQueryOperator::Equal,
                    mfm_facts::FactQueryOperator::GreaterThanOrEqual,
                ],
                mfm_facts::FactFieldExposure::Returnable,
                None,
                None,
                true,
                true,
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

fn fact_projection_row(
    n: u8,
    descriptor_hash: ContentDigest,
    namespace_hash: ContentDigest,
    audience: mfm_facts::FactAudience,
) -> IndexedFactFixture {
    let claim_id = mfm_facts::FactClaimId::new(run_id(n), n as u64, 0).expect("claim id");
    let subject = subject_evidence(namespace_hash.clone());
    let response_hash = digest(40 + n);
    let artifact_id = artifact_id(50 + n);
    let response_artifact_evidence = fact_artifact_evidence(
        artifact_id.clone(),
        response_hash.clone(),
        schema_id("mfm.rest.test.fact.response", 3),
        Some(node_id(n)),
        events::ArtifactRole::FactResponse,
    );
    let artifact_evidence_hash = artifact_evidence_hash(&response_artifact_evidence);
    let producer = producer();
    let claim = mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::Indexed {
            audience,
            scope: mfm_facts::FactVisibilityScope::Default,
        },
        fact_kind: mfm_facts::FactKind::new("mfm.rest.test.fact").expect("kind"),
        fact_descriptor_hash: descriptor_hash.clone(),
        subject: subject.clone(),
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        request: None,
        response: mfm_facts::FactResponseEvidence::new(
            schema_id("mfm.rest.test.fact.response", 3),
            response_hash.clone(),
            artifact_id.clone(),
            artifact_evidence_hash.clone(),
        ),
        producer: producer.clone(),
    })
    .expect("claim");
    let index = store::FactIndexProjection {
        fact_claim_id: claim_id.clone(),
        source_run_id: run_id(n),
        source_seq: n as u64,
        source_ordinal: 0,
        source_event_id: event_id(n),
        producer_node_id: node_id(n),
        commit_id: store::CommitKey::new(format!("commit-{n}")).expect("commit"),
        store_commit_order: n as u64,
        recorded_at: "2026-07-02T00:00:00Z".to_owned(),
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        audience,
        visibility_scope: mfm_facts::FactVisibilityScope::Default,
        fact_kind: mfm_facts::FactKind::new("mfm.rest.test.fact").expect("kind"),
        fact_descriptor_hash: descriptor_hash.clone(),
        fact_subject_namespace_hash: namespace_hash,
        fact_key: subject.fact_key().clone(),
        subject_material_hash: subject.subject_material_hash().clone(),
        request_schema_id: None,
        request_hash: None,
        response_schema_id: schema_id("mfm.rest.test.fact.response", 3),
        response_hash,
        artifact_id,
        artifact_evidence_hash,
        capability_kind: producer.capability_kind().clone(),
        capability_version: producer.capability_version().clone(),
        adapter_kind: producer.adapter_kind().clone(),
        adapter_version: producer.adapter_version().clone(),
    };
    let record = store::FactRecordProjection {
        fact_claim_id: claim_id.clone(),
        source_event_id: index.source_event_id.clone(),
        source_run_id: index.source_run_id.clone(),
        source_seq: index.source_seq,
        source_ordinal: index.source_ordinal,
        node_id: node_id(n),
        attempt_id: attempt_id(n),
        response_artifact_evidence: Some(response_artifact_evidence),
        claim,
    };
    let terms = vec![
        store::FactIndexTermProjection {
            fact_claim_id: claim_id.clone(),
            fact_descriptor_hash: descriptor_hash.clone(),
            field_id: mfm_facts::FactFieldId::new("subject.account").expect("field"),
            source: mfm_facts::FactFieldSource::Subject,
            value_type: mfm_facts::FactFieldValueType::String,
            value: mfm_facts::FactCanonicalScalar::String("public-account".to_owned()),
            unit: None,
            scale: None,
        },
        store::FactIndexTermProjection {
            fact_claim_id: claim_id,
            fact_descriptor_hash: descriptor_hash,
            field_id: mfm_facts::FactFieldId::new("result.amount").expect("field"),
            source: mfm_facts::FactFieldSource::Result,
            value_type: mfm_facts::FactFieldValueType::UnsignedInteger,
            value: mfm_facts::FactCanonicalScalar::UnsignedInteger(15),
            unit: None,
            scale: None,
        },
    ];
    IndexedFactFixture {
        record,
        index,
        terms,
    }
}

fn private_fact_record(
    n: u8,
    descriptor_hash: ContentDigest,
    namespace_hash: ContentDigest,
) -> store::FactRecordProjection {
    let subject = subject_evidence(namespace_hash);
    let response_hash = digest(43);
    let artifact_id = artifact_id(53);
    let response_artifact_evidence = fact_artifact_evidence(
        artifact_id.clone(),
        response_hash.clone(),
        schema_id("mfm.rest.test.fact.response", 3),
        Some(node_id(n)),
        events::ArtifactRole::FactResponse,
    );
    let artifact_evidence_hash = artifact_evidence_hash(&response_artifact_evidence);
    store::FactRecordProjection {
        fact_claim_id: mfm_facts::FactClaimId::new(run_id(n), n as u64, 0).expect("claim id"),
        source_event_id: event_id(n),
        source_run_id: run_id(n),
        source_seq: n as u64,
        source_ordinal: 0,
        node_id: node_id(n),
        attempt_id: attempt_id(n),
        response_artifact_evidence: Some(response_artifact_evidence),
        claim: mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
            visibility: mfm_facts::FactVisibility::RunPrivate,
            fact_kind: mfm_facts::FactKind::new("mfm.rest.test.fact").expect("kind"),
            fact_descriptor_hash: descriptor_hash,
            subject: subject.clone(),
            observed_at: None,
            request: None,
            response: mfm_facts::FactResponseEvidence::new(
                schema_id("mfm.rest.test.fact.response", 3),
                response_hash,
                artifact_id,
                artifact_evidence_hash,
            ),
            producer: producer(),
        })
        .expect("claim"),
    }
}

fn subject_evidence(namespace_hash: ContentDigest) -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactSubjectValueV1::new(
        mfm_facts::FactFieldId::new("subject.account").expect("field"),
        mfm_facts::FactFieldValueType::String,
        mfm_facts::FactCanonicalScalar::String("public-account".to_owned()),
    )
    .expect("subject value")])
    .expect("subject material");
    mfm_facts::FactSubjectEvidence::from_material(namespace_hash, &material)
        .expect("subject evidence")
}

fn producer() -> mfm_facts::FactProducerProvenance {
    mfm_facts::FactProducerProvenance::new(
        mfm_ids::CapabilityKind::new(
            "mfm.rest.test",
            "fact-read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"capability"),
        )
        .expect("capability kind"),
        mfm_ids::CapabilityVersion::new("mfm.rest.test.fact_read.v1").expect("capability version"),
        mfm_ids::AdapterKind::new(
            "mfm.rest.test",
            "fact-adapter",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"adapter"),
        )
        .expect("adapter kind"),
        mfm_ids::AdapterVersion::new("mfm.rest.test.fact_adapter.v1").expect("adapter version"),
    )
}

fn public_ref_for_claim(claim_id: &mfm_facts::FactClaimId) -> String {
    let claim_id = mfm_facts::canonical_fact_claim_id_bytes(claim_id).expect("claim bytes");
    let material = [
        b"mfm.public-fact-ref.v1:".as_slice(),
        claim_id.as_bytes(),
        b":platform:default".as_slice(),
    ]
    .concat();
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&material));
    format!(
        "pfr_{}",
        digest.as_str().rsplit(':').next().unwrap_or_default()
    )
}

fn unknown_public_ref() -> String {
    format!("pfr_{}", "a".repeat(64))
}

fn assert_private_tokens_absent(value: &serde_json::Value, fixture: &FactRouteFixture) {
    let rendered = serde_json::to_string(value).expect("render response");
    for token in &fixture.private_tokens {
        assert!(
            !rendered.contains(token),
            "response exposed private token {token}: {rendered}"
        );
    }
}

fn public_fact_from_projection(
    projection: &store::ProjectionSnapshot,
    entry: &store::FactIndexProjection,
) -> Result<mfm_app::PublicFactRef, AppError> {
    Ok(mfm_app::PublicFactRef {
        public_ref: mfm_app::PublicFactRefId::new(public_ref_for_claim(&entry.fact_claim_id))?,
        fact_kind: entry.fact_kind.as_str().to_owned(),
        descriptor: mfm_app::PublicFactDescriptorRef {
            descriptor_schema_id: schema_id("mfm.rest.test.fact.descriptor", 1)
                .as_str()
                .to_owned(),
            subject_schema_id: schema_id("mfm.rest.test.fact.subject", 2)
                .as_str()
                .to_owned(),
            response_schema_id: schema_id("mfm.rest.test.fact.response", 3)
                .as_str()
                .to_owned(),
        },
        recorded_at: entry.recorded_at.clone(),
        observed_at: entry.observed_at.clone(),
        fields: projection
            .fact_term_entries()
            .filter(|((claim_id, _field_id), _term)| claim_id == &entry.fact_claim_id)
            .filter(|((_claim_id, field_id), _term)| {
                field_id.as_str() == "subject.account" || field_id.as_str() == "result.amount"
            })
            .map(|(_key, term)| public_field_from_term(term))
            .collect(),
    })
}

fn public_field_from_term(term: &store::FactIndexTermProjection) -> mfm_app::PublicFactFieldValue {
    mfm_app::PublicFactFieldValue {
        field_id: term.field_id.as_str().to_owned(),
        path: term.field_id.as_str().to_owned(),
        source: term.source.path_prefix().to_owned(),
        value_type: match term.value_type {
            mfm_facts::FactFieldValueType::String => "string",
            mfm_facts::FactFieldValueType::Boolean => "boolean",
            mfm_facts::FactFieldValueType::SignedInteger => "signed_integer",
            mfm_facts::FactFieldValueType::UnsignedInteger => "unsigned_integer",
            mfm_facts::FactFieldValueType::Timestamp => "timestamp",
            mfm_facts::FactFieldValueType::DecimalString => "decimal_string",
            mfm_facts::FactFieldValueType::Digest => "digest",
        }
        .to_owned(),
        value: match &term.value {
            mfm_facts::FactCanonicalScalar::String(value) => {
                mfm_app::PublicFactScalarValue::String(value.clone())
            }
            mfm_facts::FactCanonicalScalar::Boolean(value) => {
                mfm_app::PublicFactScalarValue::Boolean(*value)
            }
            mfm_facts::FactCanonicalScalar::SignedInteger(value) => {
                mfm_app::PublicFactScalarValue::SignedInteger(*value)
            }
            mfm_facts::FactCanonicalScalar::UnsignedInteger(value) => {
                mfm_app::PublicFactScalarValue::UnsignedInteger(*value)
            }
            mfm_facts::FactCanonicalScalar::Timestamp(value) => {
                mfm_app::PublicFactScalarValue::Timestamp(value.clone())
            }
            mfm_facts::FactCanonicalScalar::DecimalString(value) => {
                mfm_app::PublicFactScalarValue::DecimalString(value.as_str().to_owned())
            }
            mfm_facts::FactCanonicalScalar::Digest(value) => {
                mfm_app::PublicFactScalarValue::Digest(value.as_str().to_owned())
            }
        },
        unit: None,
        scale: None,
    }
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

fn artifact_id(n: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(n))
}

fn node_id(n: u8) -> mfm_ids::NodeId {
    mfm_ids::NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(n))
}

fn attempt_id(n: u8) -> mfm_ids::AttemptId {
    mfm_ids::AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(n))
}

fn fact_artifact_evidence(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    producer_node_id: Option<mfm_ids::NodeId>,
    artifact_role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id,
        producer_seed_id: None,
        artifact_role,
    }
}

fn artifact_evidence_hash(evidence: &store::ArtifactEvidenceRef) -> ContentDigest {
    evidence.evidence_hash().expect("artifact evidence hash")
}

fn digest(n: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(n))
}

fn digest_bytes(n: u8) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array([n; 32])
}
