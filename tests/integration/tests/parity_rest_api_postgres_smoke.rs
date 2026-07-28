#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_integration_tests::test_support::{
    connect_postgres_with_retry, create_postgres_schema, drop_postgres_schema, json_post,
    response_json, schema_scoped_database_url, start_counted_portfolio_rpc_mock,
    start_portfolio_rpc_mock, unique_postgres_schema, verified_run_view,
    write_portfolio_runtime_config_for_test, UncertainFactSettlementStore,
};
use mfm_portfolio::PortfolioConfig;
use mfm_store::v1::StoreScopeStore;
use mfm_values::MfmConfig;
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000033";
#[tokio::test]
async fn parity_rest_postgres_smoke() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_postgres_schema("rest");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let _store = connect_postgres_with_retry(&scoped_database_url, 20, 250).await;
    let application = mfm_app::connect_production_application(Some(&scoped_database_url), None)
        .await
        .expect("connect REST application");
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState::new(
        mfm_rest_api::RestProcessRole::Live,
        application,
    ));

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
    let ready_v = response_json(ready).await;
    assert_eq!(ready_v["status"], "success");
    assert_eq!(ready_v["data"]["checks"]["run_store"], "ready");

    let unknown = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "entry_point": "mfm.unknown/missing@1",
                "target": "acme/primary",
            }),
        ))
        .await
        .expect("unknown entry-point response");
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
    let unknown = response_json(unknown).await;
    assert_eq!(unknown["error"]["code"], "EntryPointNotFound");

    let absent_status = app
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
    assert_eq!(absent_status.status(), StatusCode::NOT_FOUND);

    drop_postgres_schema(&database_url, &schema).await;
}

#[tokio::test]
async fn parity_portfolio_snapshot_admission_resolves_the_current_configured_target() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_postgres_schema("portfolio_snapshot_admission");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_database_url, 20, 250).await;

    let publications = mfm_app::import_setup_toml(
        &store,
        include_bytes!("../../../examples/setup/organization.toml"),
    )
    .await
    .expect("publish portfolio-only setup");
    assert_eq!(publications.len(), 1, "setup exposes one portfolio config");
    let publication = &publications[0];
    assert_eq!(
        publication.schema_id,
        PortfolioConfig::schema_id().expect("portfolio schema")
    );

    let store_scope_id = store.load_store_scope_id().await.expect("store scope");
    let launch = mfm_app::prepare_entry_point_run_launch(
        &store,
        "mfm.portfolio/snapshot@1",
        &publication.target,
        &mfm_app::production_certification_registry().expect("production certification registry"),
        store_scope_id.clone(),
        Some(mfm_app::InvocationKey::new("postgres-portfolio-admission").expect("invocation key")),
    )
    .await
    .expect("prepare current portfolio snapshot admission");

    assert_eq!(
        launch.evidence.entry_point.entry_point_id.as_str(),
        "mfm.portfolio/snapshot@1"
    );
    assert_eq!(launch.evidence.entry_point.configured_targets.len(), 1);
    let source = &launch.evidence.entry_point.configured_targets[0];
    assert_eq!(source.target.as_str(), publication.target);
    assert_eq!(source.schema_id, publication.schema_id);
    assert_eq!(source.digest, publication.digest);
    assert!(launch
        .evidence
        .config_artifacts
        .iter()
        .any(|artifact| artifact.evidence.schema_id.as_ref() == Some(&publication.schema_id)));

    let registry = mfm_app::production_certification_registry()
        .expect("production certification registry for rejection checks");
    let invalid_target = mfm_app::prepare_entry_point_run_launch(
        &store,
        "mfm.portfolio/snapshot@1",
        "mfm.reserved",
        &registry,
        store_scope_id.clone(),
        None,
    )
    .await
    .expect_err("invalid target grammar must be rejected");
    assert_eq!(invalid_target.code, "ConfiguredTargetInvalid");

    let canonical_json = mfm_app::export_setup_target(&store, &publication.target)
        .await
        .expect("export current portfolio configuration");
    let configured_pool = PgPool::connect(&scoped_database_url)
        .await
        .expect("connect for current configuration rejection checks");
    sqlx::query("UPDATE configured_values SET schema_id = $2 WHERE target = $1")
        .bind(&publication.target)
        .bind(
            "schema:mfm.test.wrong_current_config:1:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .execute(&configured_pool)
        .await
        .expect("replace current schema for rejection check");
    let wrong_schema = mfm_app::prepare_entry_point_run_launch(
        &store,
        "mfm.portfolio/snapshot@1",
        &publication.target,
        &registry,
        store_scope_id.clone(),
        None,
    )
    .await
    .expect_err("wrong current schema must be rejected");
    assert_eq!(wrong_schema.code, "ConfiguredValueSchemaInvalid");

    let restored = mfm_app::import_setup_toml(
        &store,
        include_bytes!("../../../examples/setup/organization.toml"),
    )
    .await
    .expect("restore current portfolio configuration after schema check");
    assert_eq!(restored.len(), 1);
    assert_eq!(
        restored[0].status,
        mfm_app::SetupConfigPublicationStatus::Updated
    );

    sqlx::query(
        "INSERT INTO configured_values (target, schema_id, digest, canonical_json) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind("acme/other")
    .bind(publication.schema_id.as_str())
    .bind(publication.digest.as_str())
    .bind(canonical_json)
    .execute(&configured_pool)
    .await
    .expect("publish mismatched target row");
    let mismatch = mfm_app::prepare_entry_point_run_launch(
        &store,
        "mfm.portfolio/snapshot@1",
        "acme/other",
        &registry,
        store_scope_id,
        None,
    )
    .await
    .expect_err("embedded portfolio id must match the selected target");
    assert_eq!(mismatch.code, "ConfiguredTargetMismatch");

    configured_pool.close().await;

    drop_postgres_schema(&database_url, &schema).await;
}

#[tokio::test]
async fn parity_postgres_uncertain_fact_settlement_does_not_repeat_live_io() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_postgres_schema("uncertain_fact_settlement");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_database_url, 20, 250).await;
    let publication = mfm_app::import_setup_toml(
        &store,
        include_bytes!("../../../examples/setup/organization.toml"),
    )
    .await
    .expect("publish portfolio setup")
    .into_iter()
    .next()
    .expect("one setup publication");
    let rpc = start_counted_portfolio_rpc_mock().await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_config_path =
        write_portfolio_runtime_config_for_test(runtime_dir.path(), rpc.url());
    let uncertain = std::sync::Arc::new(UncertainFactSettlementStore::new(store.clone()));
    let runners =
        mfm_app::production_runner_registry_for_test(uncertain.clone(), Some(&runtime_config_path))
            .await
            .expect("production snapshot runners");
    let certification = mfm_app::production_certification_registry()
        .expect("production snapshot certification registry");
    let launch = mfm_app::prepare_entry_point_run_launch(
        &store,
        "mfm.portfolio/snapshot@1",
        &publication.target,
        &certification,
        store.load_store_scope_id().await.expect("store scope"),
        Some(
            mfm_app::InvocationKey::new("postgres-uncertain-fact-settlement")
                .expect("invocation key"),
        ),
    )
    .await
    .expect("prepare snapshot launch");
    let run_id = launch.run_id.clone();
    let services = mfm_app::make_run_services(runners, uncertain.clone(), certification.clone());

    let response = services
        .launch_run(launch)
        .await
        .expect("uncertain snapshot launch")
        .into_response_parts()
        .1
        .expect("uncertain snapshot response");

    assert_eq!(response.run_mode, mfm_app::RunModeStatus::Completed);
    assert!(
        uncertain.injected(),
        "the fact settlement must be uncertain"
    );
    let view = verified_run_view(&store, &certification, &run_id).await;
    let lifecycle = mfm_store::v1::current_lifecycle::read(&view);
    let mut facts = Vec::new();
    let _ = lifecycle.visit_records(|record| {
        if let mfm_store::v1::current_lifecycle::CurrentRecordKindRef::FactRecorded(fact) =
            record.kind()
        {
            facts.push((
                fact,
                record.commit_key().clone(),
                record.store_commit_order(),
            ));
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert_eq!(
        facts.len(),
        2,
        "both collector facts must commit completely"
    );
    for (fact, fact_commit_key, fact_store_commit_order) in facts {
        let mut shares_commit_with_output = false;
        let _ = lifecycle.visit_records(|record| {
            if let mfm_store::v1::current_lifecycle::CurrentRecordKindRef::CellProduced(payload) =
                record.kind()
            {
                shares_commit_with_output = payload.node_id == fact.node_id
                    && payload.attempt_id == fact.attempt_id
                    && record.commit_key() == &fact_commit_key
                    && record.store_commit_order() == fact_store_commit_order;
            }
            if shares_commit_with_output {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        });
        assert!(
            shares_commit_with_output,
            "every fact must share its commit with the collection output"
        );
    }

    let methods = rpc.methods();
    for method in [
        "eth_chainId",
        "eth_getBalance",
        "getblockchaininfo",
        "scantxoutset",
        "getblockhash",
    ] {
        assert_eq!(method_count(&methods, method), 1, "{methods:?}");
    }
    assert_eq!(
        method_count(&methods, "eth_getBlockByNumber"),
        2,
        "{methods:?}"
    );
    assert_eq!(
        methods.len(),
        7,
        "unexpected or repeated live IO: {methods:?}"
    );

    drop(services);
    drop_postgres_schema(&database_url, &schema).await;
}

fn method_count(methods: &[String], expected: &str) -> usize {
    methods.iter().filter(|method| *method == expected).count()
}

#[tokio::test]
async fn parity_rest_snapshot_start_retains_configured_target_evidence_and_replays_without_live_inputs(
) {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_postgres_schema("portfolio_snapshot_rest");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_database_url, 20, 250).await;
    let publication = mfm_app::import_setup_toml(
        &store,
        include_bytes!("../../../examples/setup/organization.toml"),
    )
    .await
    .expect("publish portfolio-only setup")
    .into_iter()
    .next()
    .expect("one setup publication");
    let rpc_url = start_portfolio_rpc_mock().await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_config_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &rpc_url);
    let application = mfm_app::connect_production_application(
        Some(&scoped_database_url),
        Some(&runtime_config_path),
    )
    .await
    .expect("connect live REST application");
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState::new(
        mfm_rest_api::RestProcessRole::Live,
        application,
    ));
    let request = json!({
        "entry_point": "mfm.portfolio/snapshot@1",
        "target": publication.target.clone(),
        "invocation_key": "postgres-rest-portfolio-snapshot",
    });

    for malformed_request in [
        json!({
            "entry_point": "mfm.portfolio/snapshot@1",
            "request": {},
        }),
        json!({
            "entry_point": "mfm.portfolio/snapshot@1",
            "target": publication.target.clone(),
            "request": {},
        }),
    ] {
        let malformed = app
            .clone()
            .oneshot(json_post("/v1/runs/start", malformed_request))
            .await
            .expect("REST malformed-request response");
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        let malformed = response_json(malformed).await;
        assert_eq!(malformed["error"]["code"], "InvalidJson");
    }

    let response = app
        .clone()
        .oneshot(json_post("/v1/runs/start", request))
        .await
        .expect("REST start response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = response_json(response).await;
    assert_eq!(response["status"], "success");
    assert_eq!(response["data"]["outcome"], "admitted");
    assert_eq!(response["data"]["run"]["run_mode"], "completed");
    let run_id: mfm_ids::RunId = response["data"]["run"]["run_id"]
        .as_str()
        .expect("started run id")
        .parse()
        .expect("typed started run id");

    let certification = mfm_app::production_certification_registry()
        .expect("production snapshot certification registry");
    let view = verified_run_view(&store, &certification, &run_id).await;
    let lifecycle = mfm_store::v1::current_lifecycle::read(&view);
    let admitted = lifecycle.admission().expect("run admission evidence");
    assert_eq!(
        admitted.entry_point().entry_point_id.as_str(),
        "mfm.portfolio/snapshot@1"
    );
    assert_eq!(admitted.entry_point().configured_targets.len(), 1);
    let source = &admitted.entry_point().configured_targets[0];
    assert_eq!(source.target.as_str(), publication.target);
    assert_eq!(source.schema_id, publication.schema_id);
    assert_eq!(source.digest, publication.digest);

    std::fs::remove_file(&runtime_config_path).expect("remove live runtime config");
    let application = mfm_app::connect_production_application(Some(&scoped_database_url), None)
        .await
        .expect("connect read REST application");
    let reader = mfm_rest_api::make_app(mfm_rest_api::AppState::new(
        mfm_rest_api::RestProcessRole::Read,
        application,
    ));
    let replay = reader
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/replay"))
                .body(Body::empty())
                .expect("replay request"),
        )
        .await
        .expect("REST replay response");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = response_json(replay).await;
    assert_eq!(replay["status"], "success");
    assert_eq!(replay["data"]["run_mode"], "completed");

    drop_postgres_schema(&database_url, &schema).await;
}
