//! Replay IO implementation.
//!
//! Source of truth: `docs/design.md` (v4).
//! Not part of the stable API contract (Appendix C.1).

use std::sync::Arc;

use async_trait::async_trait;
use zeroize::Zeroizing;

use crate::errors::{
    ErrorCategory, ErrorInfo, IoError, RunError, StorageError, CODE_MISSING_FACT_KEY,
};
use crate::hashing::{artifact_id_for_bytes, canonical_json_bytes, CanonicalJsonError};
use crate::ids::{ArtifactId, ErrorCode, FactKey, RunId, StateId};
use crate::io::{IoCall, IoProvider, IoResult};
use crate::live_io::FactIndex;
use crate::stores::{ArtifactKind, ArtifactStore};

fn info(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: &'static str,
) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.to_string(),
        details: None,
    }
}

fn json_payload_read_error(err: RunError) -> IoError {
    match err {
        RunError::Storage(StorageError::Corruption(info)) => IoError::Other(info),
        RunError::Storage(_) => IoError::Other(info(
            "fact_payload_get_failed",
            ErrorCategory::Storage,
            false,
            "failed to read fact payload",
        )),
        _ => IoError::Other(info(
            "fact_payload_decode_failed",
            ErrorCategory::ParsingInput,
            false,
            "failed to decode fact payload",
        )),
    }
}

fn validate_record_value_bytes(
    value: &serde_json::Value,
    payload_id: &ArtifactId,
) -> Result<(), IoError> {
    let expected_bytes = canonical_json_bytes(value).map_err(|err| match err {
        CanonicalJsonError::FloatNotAllowed => IoError::Other(info(
            "fact_payload_not_canonical",
            ErrorCategory::ParsingInput,
            false,
            "fact payload is not canonical-json-hashable (floats are forbidden)",
        )),
        CanonicalJsonError::SecretsNotAllowed => IoError::Other(info(
            "secrets_detected",
            ErrorCategory::Unknown,
            false,
            "fact payload contained secrets (policy forbids persisting secrets)",
        )),
    })?;

    let expected_id = artifact_id_for_bytes(&expected_bytes);
    if &expected_id != payload_id {
        return Err(IoError::Other(info(
            "fact_payload_conflict",
            ErrorCategory::Storage,
            false,
            "fact key was already bound to a different payload",
        )));
    }

    Ok(())
}

/// [`IoProvider`] implementation that replays previously recorded facts instead of performing live IO.
///
/// Missing deterministic facts are surfaced as structured [`IoError`] values so resume and retry
/// policy can stay consistent with the run configuration.
pub struct ReplayIo {
    run_id: RunId,
    state_id: StateId,
    attempt: u32,
    call_ordinal: u64,
    replay_missing_fact_retryable: bool,
    artifacts: Arc<dyn ArtifactStore>,
    facts: FactIndex,
}

impl ReplayIo {
    /// Creates a replay provider scoped to a specific run, state attempt, and fact index.
    pub fn new(
        run_id: RunId,
        state_id: StateId,
        attempt: u32,
        artifacts: Arc<dyn ArtifactStore>,
        facts: FactIndex,
        replay_missing_fact_retryable: bool,
    ) -> Self {
        Self {
            run_id,
            state_id,
            attempt,
            call_ordinal: 0,
            replay_missing_fact_retryable,
            artifacts,
            facts,
        }
    }

    fn missing_fact_key() -> IoError {
        IoError::MissingFactKey(info(
            CODE_MISSING_FACT_KEY,
            ErrorCategory::ParsingInput,
            false,
            "missing fact key for deterministic IO in replay mode",
        ))
    }

    fn missing_fact(&self, key: FactKey) -> IoError {
        IoError::MissingFact {
            key,
            info: info(
                "missing_fact",
                ErrorCategory::Rpc,
                self.replay_missing_fact_retryable,
                "missing recorded fact",
            ),
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

    async fn read_json_payload(
        &self,
        payload_id: &ArtifactId,
    ) -> Result<serde_json::Value, IoError> {
        crate::context_runtime::read_canonical_json_artifact(
            self.artifacts.as_ref(),
            ArtifactKind::FactPayload,
            payload_id,
        )
        .await
        .map_err(json_payload_read_error)
    }

    async fn read_bytes_payload(&self, payload_id: &ArtifactId) -> Result<Vec<u8>, IoError> {
        self.artifacts.get(payload_id).await.map_err(|_| {
            IoError::Other(info(
                "fact_payload_get_failed",
                ErrorCategory::Storage,
                false,
                "failed to read fact payload",
            ))
        })
    }

    async fn read_protected_payload(&self, key: &FactKey) -> Result<Zeroizing<Vec<u8>>, IoError> {
        let Some(payload_id) = self.facts.get(key).await else {
            return Err(self.missing_fact(key.clone()));
        };

        self.artifacts
            .get_protected_bytes(&payload_id)
            .await
            .map_err(|_| {
                IoError::Other(info(
                    "protected_artifact_get_failed",
                    ErrorCategory::Storage,
                    false,
                    "failed to read protected artifact",
                ))
            })
    }
}

#[async_trait]
impl IoProvider for ReplayIo {
    async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
        let Some(key) = call.fact_key else {
            return Err(Self::missing_fact_key());
        };

        let Some(payload_id) = self.facts.get(&key).await else {
            return Err(self.missing_fact(key));
        };

        let response = self.read_json_payload(&payload_id).await?;
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
        let Some(payload_id) = self.facts.get(&key).await else {
            return Err(self.missing_fact(key));
        };
        self.read_json_payload(&payload_id).await?;
        validate_record_value_bytes(&value, &payload_id)?;
        Ok(payload_id)
    }

    async fn record_protected_bytes(
        &mut self,
        key: FactKey,
        _bytes: Zeroizing<Vec<u8>>,
    ) -> Result<ArtifactId, IoError> {
        let Some(payload_id) = self.facts.get(&key).await else {
            return Err(self.missing_fact(key));
        };
        Ok(payload_id)
    }

    async fn read_protected_bytes(&mut self, key: &FactKey) -> Result<Zeroizing<Vec<u8>>, IoError> {
        self.read_protected_payload(key).await
    }

    async fn now_millis(&mut self) -> Result<u64, IoError> {
        let key = self.derived_fact_key("now_millis");
        let Some(payload_id) = self.facts.get(&key).await else {
            return Err(self.missing_fact(key));
        };
        let v = self.read_json_payload(&payload_id).await?;
        v.as_u64().ok_or_else(|| {
            IoError::Other(info(
                "fact_payload_invalid",
                ErrorCategory::ParsingInput,
                false,
                "recorded time fact payload was not a u64",
            ))
        })
    }

    async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        let key = self.derived_fact_key("random_bytes");
        let Some(payload_id) = self.facts.get(&key).await else {
            return Err(self.missing_fact(key));
        };
        let bytes = self.read_bytes_payload(&payload_id).await?;
        if bytes.len() != n {
            return Err(IoError::Other(info(
                "fact_payload_invalid",
                ErrorCategory::ParsingInput,
                false,
                "recorded random_bytes fact payload had unexpected length",
            )));
        }
        Ok(bytes)
    }

    async fn sleep_ms(&mut self, _duration_ms: u64) -> Result<(), IoError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCategory;
    use crate::hashing::artifact_id_for_bytes;
    use crate::stores::{ArtifactKind, ArtifactStore};
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    #[derive(Clone, Default)]
    struct MemArtifactStore {
        inner: Arc<Mutex<HashMap<ArtifactId, Vec<u8>>>>,
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
            let inner = self.inner.lock().await;
            inner.get(id).cloned().ok_or_else(|| {
                StorageError::NotFound(info(
                    "not_found",
                    ErrorCategory::Storage,
                    false,
                    "artifact not found",
                ))
            })
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(self.inner.lock().await.contains_key(id))
        }
    }

    #[tokio::test]
    async fn deterministic_call_missing_fact_key_is_stable() {
        let artifacts = Arc::new(MemArtifactStore::default());
        let facts = FactIndex::default();
        let mut io = ReplayIo::new(
            RunId(uuid::Uuid::new_v4()),
            StateId::must_new("machine.main.s1".to_string()),
            0,
            artifacts,
            facts,
            false,
        );

        let err = io
            .call(IoCall {
                namespace: "test".to_string(),
                request: serde_json::json!({}),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::MissingFactKey(info) => assert_eq!(info.code.0, "missing_fact_key"),
            other => panic!("expected MissingFactKey, got: {other:?}"),
        }
    }

    fn replay_io(artifacts: Arc<MemArtifactStore>, facts: FactIndex) -> ReplayIo {
        ReplayIo::new(
            RunId(uuid::Uuid::new_v4()),
            StateId::must_new("machine.main.s1".to_string()),
            0,
            artifacts,
            facts,
            false,
        )
    }

    async fn bind_fact_payload(
        artifacts: &Arc<MemArtifactStore>,
        facts: &FactIndex,
        key: &FactKey,
        bytes: Vec<u8>,
    ) -> ArtifactId {
        let id = artifacts
            .put(ArtifactKind::FactPayload, bytes)
            .await
            .expect("put fact payload");
        facts.bind_if_unset(key.clone(), id.clone()).await;
        id
    }

    async fn replay_call_with_payload(bytes: Vec<u8>) -> Result<IoResult, IoError> {
        let artifacts = Arc::new(MemArtifactStore::default());
        let facts = FactIndex::default();
        let key = FactKey("fact:json".to_string());
        bind_fact_payload(&artifacts, &facts, &key, bytes).await;
        let mut io = replay_io(artifacts, facts);

        io.call(IoCall {
            namespace: "test".to_string(),
            request: serde_json::json!({}),
            fact_key: Some(key),
        })
        .await
    }

    fn assert_other_code(err: IoError, expected: &str) {
        match err {
            IoError::Other(info) => assert_eq!(info.code.0, expected),
            other => panic!("expected IoError::Other({expected}), got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_call_rejects_noncanonical_json_fact_payload() {
        let err = replay_call_with_payload(br#"{ "a": 1 }"#.to_vec())
            .await
            .expect_err("noncanonical JSON fact payload must fail closed");

        assert_other_code(err, "artifact_not_canonical");
    }

    #[tokio::test]
    async fn replay_call_rejects_float_fact_payload() {
        let err = replay_call_with_payload(br#"{"a":1.5}"#.to_vec())
            .await
            .expect_err("float JSON fact payload must fail closed");

        assert_other_code(err, "artifact_float_not_allowed");
    }

    #[tokio::test]
    async fn replay_call_rejects_secret_shaped_fact_payload() {
        let err = replay_call_with_payload(br#"{"password":"not-a-real-secret"}"#.to_vec())
            .await
            .expect_err("secret-shaped JSON fact payload must fail closed");

        assert_other_code(err, "secrets_detected");
    }

    #[tokio::test]
    async fn replay_call_rejects_fact_payload_content_address_mismatch() {
        let artifacts = Arc::new(MemArtifactStore::default());
        let facts = FactIndex::default();
        let key = FactKey("fact:json".to_string());
        let bad_id = ArtifactId::must_new("0".repeat(64));
        artifacts
            .inner
            .lock()
            .await
            .insert(bad_id.clone(), br#"{"a":1}"#.to_vec());
        facts.bind_if_unset(key.clone(), bad_id).await;
        let mut io = replay_io(artifacts, facts);

        let err = io
            .call(IoCall {
                namespace: "test".to_string(),
                request: serde_json::json!({}),
                fact_key: Some(key),
            })
            .await
            .expect_err("content address mismatch must fail closed");

        assert_other_code(err, "artifact_content_address_mismatch");
    }

    #[tokio::test]
    async fn replay_random_bytes_keeps_byte_facts_out_of_json_validation() {
        let artifacts = Arc::new(MemArtifactStore::default());
        let facts = FactIndex::default();
        let key = FactKey(format!(
            "mfm:random_bytes|run:{}|state:{}|attempt:{}|ord:{}",
            "00000000-0000-0000-0000-000000000000", "machine.main.s1", 0, 0
        ));
        bind_fact_payload(&artifacts, &facts, &key, vec![0, 1, 2, 255]).await;
        let mut io = ReplayIo::new(
            RunId(uuid::Uuid::nil()),
            StateId::must_new("machine.main.s1".to_string()),
            0,
            artifacts,
            facts,
            false,
        );

        let bytes = io
            .random_bytes(4)
            .await
            .expect("raw byte facts should not be decoded as JSON");

        assert_eq!(bytes, vec![0, 1, 2, 255]);
    }

    #[tokio::test]
    async fn missing_fact_retryable_follows_run_config() {
        let artifacts = Arc::new(MemArtifactStore::default());
        let facts = FactIndex::default();
        let mut io = ReplayIo::new(
            RunId(uuid::Uuid::new_v4()),
            StateId::must_new("machine.main.s1".to_string()),
            0,
            artifacts,
            facts,
            true,
        );

        let err = io
            .call(IoCall {
                namespace: "test".to_string(),
                request: serde_json::json!({}),
                fact_key: Some(FactKey("k".to_string())),
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::MissingFact { info, .. } => assert!(info.retryable),
            other => panic!("expected MissingFact, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_sleep_is_a_noop_without_fact_lookup() {
        let artifacts = Arc::new(MemArtifactStore::default());
        let facts = FactIndex::default();
        let mut io = ReplayIo::new(
            RunId(uuid::Uuid::new_v4()),
            StateId::must_new("machine.main.sleep".to_string()),
            0,
            artifacts,
            facts,
            true,
        );

        io.sleep_ms(60_000)
            .await
            .expect("replay sleep should not require recorded facts");
    }
}
