//! Live IO implementation (Milestone 1).
//!
//! Source of truth: `REDESIGN.md` (v4).
//! Not part of the stable API contract (Appendix C.1).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rand::TryRngCore;
use tokio::sync::Mutex;

use crate::engine::Stores;
use crate::errors::{ErrorCategory, ErrorInfo, IoError};
use crate::events::{DomainEvent, Event, EventEnvelope, FactRecorded, DOMAIN_EVENT_FACT_RECORDED};
use crate::hashing::{canonical_json_bytes, CanonicalJsonError};
use crate::ids::{ArtifactId, ErrorCode, FactKey, RunId, StateId};
use crate::io::{IoCall, IoProvider, IoResult};
use crate::stores::{ArtifactKind, ArtifactStore};

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn io_other(code: &'static str, category: ErrorCategory, message: &'static str) -> IoError {
    IoError::Other(info(code, category, message))
}

#[derive(Clone, Default)]
pub struct FactIndex {
    inner: Arc<Mutex<HashMap<FactKey, ArtifactId>>>,
}

impl FactIndex {
    pub fn from_event_stream(stream: &[EventEnvelope]) -> Self {
        let mut m = HashMap::new();
        for e in stream {
            let Event::Domain(de) = &e.event else {
                continue;
            };
            if de.name != DOMAIN_EVENT_FACT_RECORDED {
                continue;
            }

            let Ok(fr) = serde_json::from_value::<FactRecorded>(de.payload.clone()) else {
                continue;
            };

            // Single-assignment: first durable binding wins.
            m.entry(fr.key).or_insert(fr.payload_id);
        }

        Self {
            inner: Arc::new(Mutex::new(m)),
        }
    }

    pub async fn get(&self, key: &FactKey) -> Option<ArtifactId> {
        self.inner.lock().await.get(key).cloned()
    }

    pub async fn bind_if_unset(&self, key: FactKey, payload_id: ArtifactId) -> (ArtifactId, bool) {
        let mut inner = self.inner.lock().await;
        match inner.get(&key) {
            Some(existing) => (existing.clone(), false),
            None => {
                inner.insert(key, payload_id.clone());
                (payload_id, true)
            }
        }
    }
}

#[async_trait]
pub trait LiveIoTransport: Send {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError>;
}

#[derive(Clone)]
pub struct LiveIoEnv {
    pub stores: Stores,
    pub run_id: RunId,
    pub state_id: StateId,
    pub attempt: u32,
}

pub trait LiveIoTransportFactory: Send + Sync {
    fn make(&self, env: LiveIoEnv) -> Box<dyn LiveIoTransport>;
}

struct UnimplementedLiveIoTransport;

#[async_trait]
impl LiveIoTransport for UnimplementedLiveIoTransport {
    async fn call(&mut self, _call: IoCall) -> Result<serde_json::Value, IoError> {
        Err(io_other(
            "io_unimplemented",
            ErrorCategory::Unknown,
            "live io transport is not configured",
        ))
    }
}

#[derive(Clone, Default)]
pub struct UnimplementedLiveIoTransportFactory;

impl LiveIoTransportFactory for UnimplementedLiveIoTransportFactory {
    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(UnimplementedLiveIoTransport)
    }
}

pub struct LiveIo {
    run_id: RunId,
    state_id: StateId,
    attempt: u32,
    call_ordinal: u64,
    artifacts: Arc<dyn ArtifactStore>,
    facts: FactIndex,
    transport: Box<dyn LiveIoTransport>,
    pending: Vec<DomainEvent>,
}

impl LiveIo {
    pub fn new(
        run_id: RunId,
        state_id: StateId,
        attempt: u32,
        artifacts: Arc<dyn ArtifactStore>,
        facts: FactIndex,
        transport: Box<dyn LiveIoTransport>,
    ) -> Self {
        Self {
            run_id,
            state_id,
            attempt,
            call_ordinal: 0,
            artifacts,
            facts,
            transport,
            pending: Vec::new(),
        }
    }

    pub fn drain_pending_events(&mut self) -> Vec<DomainEvent> {
        std::mem::take(&mut self.pending)
    }

    fn derived_fact_key(&mut self, kind: &str) -> FactKey {
        let ord = self.call_ordinal;
        self.call_ordinal += 1;
        FactKey(format!(
            "mfm:{kind}|run:{}|state:{}|attempt:{}|ord:{ord}",
            self.run_id.0, self.state_id.0, self.attempt
        ))
    }

    fn fact_recorded_event(key: FactKey, payload_id: ArtifactId) -> DomainEvent {
        let payload = serde_json::to_value(FactRecorded {
            key,
            payload_id,
            meta: serde_json::json!({}),
        })
        .expect("FactRecorded must be serializable");

        DomainEvent {
            name: DOMAIN_EVENT_FACT_RECORDED.to_string(),
            payload,
            payload_ref: None,
        }
    }

    async fn record_fact_json(
        &mut self,
        key: FactKey,
        value: serde_json::Value,
    ) -> Result<(serde_json::Value, ArtifactId), IoError> {
        if let Some(payload_id) = self.facts.get(&key).await {
            let bytes = self.artifacts.get(&payload_id).await.map_err(|_| {
                io_other(
                    "fact_payload_get_failed",
                    ErrorCategory::Storage,
                    "failed to read fact payload",
                )
            })?;
            let v = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                io_other(
                    "fact_payload_decode_failed",
                    ErrorCategory::ParsingInput,
                    "failed to decode fact payload",
                )
            })?;
            return Ok((v, payload_id));
        }

        let bytes = canonical_json_bytes(&value).map_err(|e| match e {
            CanonicalJsonError::FloatNotAllowed => io_other(
                "fact_payload_not_canonical",
                ErrorCategory::ParsingInput,
                "fact payload is not canonical-json-hashable (floats are forbidden)",
            ),
            CanonicalJsonError::SecretsNotAllowed => io_other(
                "secrets_detected",
                ErrorCategory::Unknown,
                "fact payload contained secrets (Milestone 1 forbids persisting secrets)",
            ),
        })?;

        let payload_id = self
            .artifacts
            .put(ArtifactKind::FactPayload, bytes)
            .await
            .map_err(|_| {
                io_other(
                    "fact_payload_put_failed",
                    ErrorCategory::Storage,
                    "failed to store fact payload",
                )
            })?;

        let (bound_id, inserted) = self.facts.bind_if_unset(key.clone(), payload_id).await;
        if inserted {
            self.pending
                .push(Self::fact_recorded_event(key, bound_id.clone()));
            Ok((value, bound_id))
        } else {
            // Single-assignment: ignore this value and reuse the existing one.
            let bytes = self.artifacts.get(&bound_id).await.map_err(|_| {
                io_other(
                    "fact_payload_get_failed",
                    ErrorCategory::Storage,
                    "failed to read fact payload",
                )
            })?;
            let v = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                io_other(
                    "fact_payload_decode_failed",
                    ErrorCategory::ParsingInput,
                    "failed to decode fact payload",
                )
            })?;
            Ok((v, bound_id))
        }
    }

    async fn record_fact_bytes(
        &mut self,
        key: FactKey,
        bytes: Vec<u8>,
    ) -> Result<(Vec<u8>, ArtifactId), IoError> {
        if let Some(payload_id) = self.facts.get(&key).await {
            let got = self.artifacts.get(&payload_id).await.map_err(|_| {
                io_other(
                    "fact_payload_get_failed",
                    ErrorCategory::Storage,
                    "failed to read fact payload",
                )
            })?;
            return Ok((got, payload_id));
        }

        let payload_id = self
            .artifacts
            .put(ArtifactKind::FactPayload, bytes.clone())
            .await
            .map_err(|_| {
                io_other(
                    "fact_payload_put_failed",
                    ErrorCategory::Storage,
                    "failed to store fact payload",
                )
            })?;

        let (bound_id, inserted) = self.facts.bind_if_unset(key.clone(), payload_id).await;
        if inserted {
            self.pending
                .push(Self::fact_recorded_event(key, bound_id.clone()));
        }

        Ok((bytes, bound_id))
    }
}

#[async_trait]
impl IoProvider for LiveIo {
    async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
        let Some(key) = call.fact_key.clone() else {
            let response = self.transport.call(call).await?;
            return Ok(IoResult {
                response,
                recorded_payload_id: None,
            });
        };

        if let Some(payload_id) = self.facts.get(&key).await {
            let bytes = self.artifacts.get(&payload_id).await.map_err(|_| {
                io_other(
                    "fact_payload_get_failed",
                    ErrorCategory::Storage,
                    "failed to read fact payload",
                )
            })?;
            let response = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                io_other(
                    "fact_payload_decode_failed",
                    ErrorCategory::ParsingInput,
                    "failed to decode fact payload",
                )
            })?;
            return Ok(IoResult {
                response,
                recorded_payload_id: Some(payload_id),
            });
        }

        let response = self.transport.call(call).await?;
        let (response, payload_id) = self.record_fact_json(key, response).await?;
        Ok(IoResult {
            response,
            recorded_payload_id: Some(payload_id),
        })
    }

    async fn get_recorded_fact(&mut self, key: &FactKey) -> Result<Option<ArtifactId>, IoError> {
        Ok(self.facts.get(key).await)
    }

    async fn now_millis(&mut self) -> Result<u64, IoError> {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                io_other(
                    "time_unavailable",
                    ErrorCategory::Unknown,
                    "system time not available",
                )
            })?
            .as_millis() as u64;

        let key = self.derived_fact_key("now_millis");
        let (v, _payload_id) = self
            .record_fact_json(key, serde_json::Value::Number(ms.into()))
            .await?;

        let n = v.as_u64().ok_or_else(|| {
            io_other(
                "fact_payload_invalid",
                ErrorCategory::ParsingInput,
                "recorded time fact payload was not a u64",
            )
        })?;
        Ok(n)
    }

    async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        let mut bytes = vec![0u8; n];
        let mut rng = rand::rngs::OsRng;
        rng.try_fill_bytes(&mut bytes).map_err(|_| {
            io_other(
                "random_unavailable",
                ErrorCategory::Unknown,
                "os randomness not available",
            )
        })?;

        let key = self.derived_fact_key("random_bytes");
        let (got, _payload_id) = self.record_fact_bytes(key, bytes).await?;
        Ok(got)
    }
}
