//! Live IO implementation.
//!
//! Source of truth: `docs/design.md` (v4).
//! Not part of the stable API contract (Appendix C.1).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rand::TryRngCore;
use tokio::sync::Mutex;

use crate::engine::Stores;
use crate::errors::{ErrorCategory, ErrorInfo, IoError};
use crate::events::{Event, EventEnvelope, FactRecorded, DOMAIN_EVENT_FACT_RECORDED};
use crate::hashing::{canonical_json_bytes, put_artifact_verified, CanonicalJsonError};
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

/// In-memory index of durable `FactKey -> ArtifactId` bindings.
///
/// The engine rebuilds this index from prior domain events before executing a run.
#[derive(Clone, Default)]
pub struct FactIndex {
    inner: Arc<Mutex<HashMap<FactKey, ArtifactId>>>,
}

impl FactIndex {
    /// Rebuilds the durable fact bindings recorded in an event stream.
    ///
    /// Only the first durable binding for a given key is kept, matching the
    /// single-assignment contract used by live/replay IO.
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

    /// Returns the currently bound payload id for `key`, if one exists.
    pub async fn get(&self, key: &FactKey) -> Option<ArtifactId> {
        self.inner.lock().await.get(key).cloned()
    }

    /// Binds `key` to `payload_id` only if the key is not already bound.
    ///
    /// Returns the effective payload id together with a flag indicating whether a
    /// new binding was inserted.
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

    /// Removes the binding for `key` only when it still points to `payload_id`.
    ///
    /// This is used to roll back optimistic in-memory bindings when durable
    /// recording fails.
    pub async fn unbind_if_matches(&self, key: &FactKey, payload_id: &ArtifactId) -> bool {
        let mut inner = self.inner.lock().await;
        match inner.get(key) {
            Some(existing) if existing == payload_id => {
                inner.remove(key);
                true
            }
            _ => false,
        }
    }
}

/// Namespace-specific live IO transport used by [`LiveIo`].
#[async_trait]
pub trait LiveIoTransport: Send {
    /// Executes an opaque IO call and returns its canonical JSON response.
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError>;
}

/// Runtime context passed to a [`LiveIoTransportFactory`] when creating a transport.
#[derive(Clone)]
pub struct LiveIoEnv {
    /// Stores used by the active run.
    pub stores: Stores,
    /// Parent run identifier.
    pub run_id: RunId,
    /// State currently issuing live IO.
    pub state_id: StateId,
    /// Attempt number for the active state.
    pub attempt: u32,
}

/// Factory for creating transports for one namespace group.
pub trait LiveIoTransportFactory: Send + Sync {
    /// Returns the namespace group handled by transports built from this factory.
    fn namespace_group(&self) -> &str;

    /// Creates a transport scoped to a particular run/state attempt.
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

/// Fallback transport factory used when live IO is not configured.
#[derive(Clone, Default)]
pub struct UnimplementedLiveIoTransportFactory;

impl LiveIoTransportFactory for UnimplementedLiveIoTransportFactory {
    fn namespace_group(&self) -> &str {
        "unimplemented"
    }

    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(UnimplementedLiveIoTransport)
    }
}

/// Live-mode IO provider that records deterministic facts for later replay.
pub struct LiveIo {
    run_id: RunId,
    state_id: StateId,
    attempt: u32,
    call_ordinal: u64,
    artifacts: Arc<dyn ArtifactStore>,
    facts: FactIndex,
    fact_recorder: Arc<dyn FactRecorder>,
    transport: Box<dyn LiveIoTransport>,
}

impl LiveIo {
    /// Creates a live IO provider for a specific state attempt.
    pub fn new(
        run_id: RunId,
        state_id: StateId,
        attempt: u32,
        artifacts: Arc<dyn ArtifactStore>,
        facts: FactIndex,
        fact_recorder: Arc<dyn FactRecorder>,
        transport: Box<dyn LiveIoTransport>,
    ) -> Self {
        Self {
            run_id,
            state_id,
            attempt,
            call_ordinal: 0,
            artifacts,
            facts,
            fact_recorder,
            transport,
        }
    }

    fn derived_fact_key(&mut self, kind: &str) -> FactKey {
        let ord = self.call_ordinal;
        self.call_ordinal += 1;
        FactKey(format!(
            "mfm:{kind}|run:{}|state:{}|attempt:{}|ord:{ord}",
            self.run_id.0,
            self.state_id.as_str(),
            self.attempt
        ))
    }

    async fn record_fact_json(
        &mut self,
        key: FactKey,
        value: serde_json::Value,
    ) -> Result<(serde_json::Value, ArtifactId), IoError> {
        let bytes = canonical_json_bytes(&value).map_err(|e| match e {
            CanonicalJsonError::FloatNotAllowed => io_other(
                "fact_payload_not_canonical",
                ErrorCategory::ParsingInput,
                "fact payload is not canonical-json-hashable (floats are forbidden)",
            ),
            CanonicalJsonError::SecretsNotAllowed => io_other(
                "secrets_detected",
                ErrorCategory::Unknown,
                "fact payload contained secrets (policy forbids persisting secrets)",
            ),
        })?;

        if let Some(payload_id) = self.facts.get(&key).await {
            let existing = self.artifacts.get(&payload_id).await.map_err(|_| {
                io_other(
                    "fact_payload_get_failed",
                    ErrorCategory::Storage,
                    "failed to read fact payload",
                )
            })?;
            if existing != bytes {
                return Err(io_other(
                    "fact_payload_conflict",
                    ErrorCategory::Storage,
                    "fact key was already bound to a different payload",
                ));
            }
            let v = serde_json::from_slice::<serde_json::Value>(&existing).map_err(|_| {
                io_other(
                    "fact_payload_decode_failed",
                    ErrorCategory::ParsingInput,
                    "failed to decode fact payload",
                )
            })?;
            return Ok((v, payload_id));
        }

        let expected_bytes = bytes.clone();
        let payload_id =
            put_artifact_verified(self.artifacts.as_ref(), ArtifactKind::FactPayload, bytes)
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
            if let Err(e) = self
                .fact_recorder
                .record_fact_binding(key.clone(), bound_id.clone())
                .await
            {
                // Roll back the in-memory binding so retries don't "think" the fact is durable.
                let _ = self.facts.unbind_if_matches(&key, &bound_id).await;
                return Err(e);
            }
            Ok((value, bound_id))
        } else {
            let existing = self.artifacts.get(&bound_id).await.map_err(|_| {
                io_other(
                    "fact_payload_get_failed",
                    ErrorCategory::Storage,
                    "failed to read fact payload",
                )
            })?;
            if existing != expected_bytes {
                return Err(io_other(
                    "fact_payload_conflict",
                    ErrorCategory::Storage,
                    "fact key was already bound to a different payload",
                ));
            }
            let v = serde_json::from_slice::<serde_json::Value>(&existing).map_err(|_| {
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

        let payload_id = put_artifact_verified(
            self.artifacts.as_ref(),
            ArtifactKind::FactPayload,
            bytes.clone(),
        )
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
            if let Err(e) = self
                .fact_recorder
                .record_fact_binding(key.clone(), bound_id.clone())
                .await
            {
                // Roll back the in-memory binding so retries don't "think" the fact is durable.
                let _ = self.facts.unbind_if_matches(&key, &bound_id).await;
                return Err(e);
            }
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

    async fn record_value(
        &mut self,
        key: FactKey,
        value: serde_json::Value,
    ) -> Result<ArtifactId, IoError> {
        let (_, payload_id) = self.record_fact_json(key, value).await?;
        Ok(payload_id)
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

    async fn sleep_ms(&mut self, duration_ms: u64) -> Result<(), IoError> {
        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
        Ok(())
    }
}

/// Durable binding sink for `FactKey -> payload_id` facts.
///
/// Design contract: fact bindings MUST be durable regardless of `EventProfile`.
#[async_trait]
pub trait FactRecorder: Send + Sync {
    /// Persists the durable `FactKey -> payload_id` binding for replay and resume.
    async fn record_fact_binding(
        &self,
        key: FactKey,
        payload_id: ArtifactId,
    ) -> Result<(), IoError>;
}

/// A `FactRecorder` that does nothing. Intended for tests and non-engine usage.
#[derive(Clone, Default)]
pub struct NoopFactRecorder;

#[async_trait]
impl FactRecorder for NoopFactRecorder {
    async fn record_fact_binding(
        &self,
        _key: FactKey,
        _payload_id: ArtifactId,
    ) -> Result<(), IoError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::StorageError;
    use crate::hashing::artifact_id_for_bytes;
    use async_trait::async_trait;

    #[derive(Default)]
    struct MemArtifactStore {
        inner: Mutex<HashMap<ArtifactId, Vec<u8>>>,
    }

    #[async_trait]
    impl ArtifactStore for MemArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            let id = artifact_id_for_bytes(&bytes);
            self.inner.lock().await.insert(id.clone(), bytes);
            Ok(id)
        }

        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            self.inner.lock().await.get(id).cloned().ok_or_else(|| {
                StorageError::NotFound(info(
                    "not_found",
                    ErrorCategory::Storage,
                    "artifact not found",
                ))
            })
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(self.inner.lock().await.contains_key(id))
        }
    }

    struct WrongIdArtifactStore;

    #[async_trait]
    impl ArtifactStore for WrongIdArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            Ok(ArtifactId::must_new("f".repeat(64)))
        }

        async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            unreachable!("record_value should fail during put verification")
        }

        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            unreachable!("record_value should not query existence")
        }
    }

    struct UnusedTransport;

    #[async_trait]
    impl LiveIoTransport for UnusedTransport {
        async fn call(&mut self, _call: IoCall) -> Result<serde_json::Value, IoError> {
            unreachable!("record_value should not call live transport")
        }
    }

    #[tokio::test]
    async fn record_value_rejects_store_returned_wrong_artifact_id() {
        let mut io = LiveIo::new(
            RunId(uuid::Uuid::new_v4()),
            StateId::must_new("machine.main.state"),
            1,
            Arc::new(WrongIdArtifactStore),
            FactIndex::default(),
            Arc::new(NoopFactRecorder),
            Box::new(UnusedTransport),
        );

        let err = io
            .record_value(FactKey("fact:key".to_string()), serde_json::json!({"a": 1}))
            .await
            .expect_err("wrong store-returned id must fail fact recording");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, "fact_payload_put_failed"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn record_value_rejects_conflicting_payload_for_existing_fact_key() {
        let mut io = LiveIo::new(
            RunId(uuid::Uuid::new_v4()),
            StateId::must_new("machine.main.state"),
            1,
            Arc::new(MemArtifactStore::default()),
            FactIndex::default(),
            Arc::new(NoopFactRecorder),
            Box::new(UnusedTransport),
        );
        let key = FactKey("fact:key".to_string());

        io.record_value(key.clone(), serde_json::json!({"a": 1}))
            .await
            .expect("first fact write");
        let err = io
            .record_value(key, serde_json::json!({"a": 2}))
            .await
            .expect_err("conflicting fact payload must fail");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, "fact_payload_conflict"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
