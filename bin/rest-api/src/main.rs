use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use mfm_app::{
    application_catalog, parse_admission_json, AdmitRunRequest, Application, MAX_ADMISSION_BYTES,
};
use mfm_evm::EvmConfig;
use mfm_ids::{AppendRequestId, RunId, StableId, StoreEpoch, StoreScopeId, TenantScopeId};
use mfm_portfolio::{PortfolioConfig, PortfolioId, QuoteCode};
use mfm_store::{
    ConfigurationCommitOutcome, StoreWorkLimits, StructuredStore, StructuredStoreIdentity,
};
use mfm_values::ValidatedConfig;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;

#[derive(Clone)]
struct ApiState(Arc<Application>);

struct AdmitWire {
    entry_point_id: String,
    input: serde_json::Value,
}

impl<'de> Deserialize<'de> for AdmitWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            EntryPointId,
            Input,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl<'de> Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str("`entry_point_id` or `input`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "entry_point_id" => Ok(Field::EntryPointId),
                            "input" => Ok(Field::Input),
                            _ => Err(de::Error::unknown_field(
                                value,
                                &["entry_point_id", "input"],
                            )),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct AdmitVisitor;

        impl<'de> Visitor<'de> for AdmitVisitor {
            type Value = AdmitWire;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an admission object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut entry_point_id = None;
                let mut input = None;
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        Field::EntryPointId => {
                            if entry_point_id.is_some() {
                                return Err(de::Error::duplicate_field("entry_point_id"));
                            }
                            entry_point_id = Some(map.next_value()?);
                        }
                        Field::Input => {
                            if input.is_some() {
                                return Err(de::Error::duplicate_field("input"));
                            }
                            input = Some(map.next_value()?);
                        }
                    }
                }
                Ok(AdmitWire {
                    entry_point_id: entry_point_id
                        .ok_or_else(|| de::Error::missing_field("entry_point_id"))?,
                    input: input.ok_or_else(|| de::Error::missing_field("input"))?,
                })
            }
        }

        deserializer.deserialize_map(AdmitVisitor)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tenant = TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")?;
    let store = StructuredStore::open_memory(
        StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")?,
            StoreEpoch::new(1),
            tenant,
        ),
        application_catalog()?,
        StoreWorkLimits::default(),
    )?;
    let configuration = store.configuration();
    let portfolio = configuration
        .initial_write_session::<PortfolioConfig>()
        .prepare_local(
            AppendRequestId::new("rest-portfolio-configuration-0001")?,
            ValidatedConfig::new(PortfolioConfig {
                portfolio_id: PortfolioId {
                    value: "demo-portfolio".to_owned(),
                },
                quotes: vec![QuoteCode::Usd],
            })?,
        )?;
    let portfolio = match configuration.commit(portfolio).await? {
        ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
        _ => return Err("failed to persist Portfolio configuration".into()),
    };
    let portfolio_head = portfolio.head().clone();
    let evm = configuration
        .write_session::<EvmConfig>(&portfolio_head)?
        .prepare_local(
            AppendRequestId::new("rest-evm-configuration-00000001")?,
            ValidatedConfig::new(EvmConfig {})?,
        )?;
    let evm = match configuration.commit(evm).await? {
        ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
        _ => return Err("failed to persist EVM configuration".into()),
    };
    let app = Arc::new(Application::new(
        store,
        portfolio_head,
        evm.into_head(),
        Vec::new(),
    )?);
    let router = Router::new()
        .route("/health", get(health))
        .route("/runs", post(admit))
        .route("/runs/:run_id", get(read))
        .route("/runs/:run_id/drive", post(drive))
        .route("/runs/:run_id/replay", get(replay))
        .route("/runs/:run_id/trace", get(trace))
        .route("/runs/:run_id/audit", get(audit))
        .route("/runs/:run_id/export", get(export_run))
        .layer(DefaultBodyLimit::max(MAX_ADMISSION_BYTES))
        .with_state(ApiState(app));
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 3000))).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({"status":"ok"}))
}

async fn admit(State(ApiState(app)): State<ApiState>, body: Bytes) -> impl IntoResponse {
    let result = if body.len() > MAX_ADMISSION_BYTES {
        Err(())
    } else {
        std::str::from_utf8(&body)
            .map_err(|_| ())
            .and_then(|text| parse_admission_json(text).map_err(|_| ()))
            .and_then(|value| serde_json::from_value::<AdmitWire>(value).map_err(|_| ()))
            .and_then(|input| {
                StableId::new(input.entry_point_id)
                    .map_err(|_| ())
                    .and_then(|entry| AdmitRunRequest::new(entry, input.input).map_err(|_| ()))
            })
    };
    let result = match result {
        Ok(request) => app.admit_run(request).await.map_err(|_| ()),
        Err(error) => Err(error),
    };
    match result {
        Ok(response) => (
            StatusCode::ACCEPTED,
            axum::Json(serde_json::json!(response)),
        )
            .into_response(),
        Err(()) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"code":"AdmissionRequestInvalid"})),
        )
            .into_response(),
    }
}

async fn read(
    State(ApiState(app)): State<ApiState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let result = match RunId::parse(run_id).map_err(|_| ()) {
        Ok(run) => app.read_public_run(run).await.map_err(|_| ()),
        Err(error) => Err(error),
    };
    match result {
        Ok(response) => (StatusCode::OK, axum::Json(serde_json::json!(response))).into_response(),
        Err(()) => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"code":"RunNotFound"})),
        )
            .into_response(),
    }
}

async fn drive(
    State(ApiState(app)): State<ApiState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let result = match RunId::parse(run_id).map_err(|_| ()) {
        Ok(run) => app.drive(run).await.map_err(|_| ()),
        Err(error) => Err(error),
    };
    match result {
        Ok(response) => (StatusCode::OK, axum::Json(serde_json::json!(response))).into_response(),
        Err(()) => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"code":"RunNotFound"})),
        )
            .into_response(),
    }
}

async fn replay(
    State(ApiState(app)): State<ApiState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let result = match RunId::parse(run_id).map_err(|_| ()) {
        Ok(run) => app.replay_run(run).await.map_err(|_| ()),
        Err(error) => Err(error),
    };
    match result {
        Ok(response) => (StatusCode::OK, axum::Json(serde_json::json!(response))).into_response(),
        Err(()) => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"code":"RunNotFound"})),
        )
            .into_response(),
    }
}

async fn trace(
    State(ApiState(app)): State<ApiState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let result = match RunId::parse(run_id).map_err(|_| ()) {
        Ok(run) => app.trace_run(run).await.map_err(|_| ()),
        Err(error) => Err(error),
    };
    match result {
        Ok(response) => (StatusCode::OK, axum::Json(serde_json::json!(response))).into_response(),
        Err(()) => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"code":"RunNotFound"})),
        )
            .into_response(),
    }
}

async fn audit(
    State(ApiState(app)): State<ApiState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let result = match RunId::parse(run_id).map_err(|_| ()) {
        Ok(run) => app.audit_access(run).await.map_err(|_| ()),
        Err(error) => Err(error),
    };
    match result {
        Ok(response) => (StatusCode::OK, axum::Json(serde_json::json!(response))).into_response(),
        Err(()) => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"code":"RunNotFound"})),
        )
            .into_response(),
    }
}

async fn export_run(
    State(ApiState(app)): State<ApiState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let result = match RunId::parse(run_id).map_err(|_| ()) {
        Ok(run) => app.export_run(run).await.map_err(|_| ()),
        Err(error) => Err(error),
    };
    match result {
        Ok(export) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            export.bytes().to_vec(),
        )
            .into_response(),
        Err(()) => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"code":"RunNotFound"})),
        )
            .into_response(),
    }
}
