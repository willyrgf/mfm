//! Portfolio entry-point ingress contract after the catalog cutover.

#![allow(clippy::disallowed_methods)]

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

mod support;
use support::{json_post, response_json};

#[tokio::test]
async fn portfolio_snapshot_requires_catalog_authority() {
    let app = rest_test_app();
    let response = app
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "entry_point": "mfm.portfolio/portfolio_snapshot@1",
                "request": {}
            }),
        ))
        .await
        .expect("portfolio snapshot response");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "CatalogStoreUnavailable");
}

fn rest_test_app() -> axum::Router {
    mfm_rest_api::make_app(support::in_memory_rest_app_state())
}
