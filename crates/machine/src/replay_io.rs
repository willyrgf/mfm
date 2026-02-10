//! Replay IO implementation (Milestone 1).
//!
//! Source of truth: `REDESIGN.md` (v4).
//! Not part of the stable API contract (Appendix C.1).

use std::sync::Arc;

use async_trait::async_trait;

use crate::errors::{ErrorCategory, ErrorInfo, IoError, CODE_MISSING_FACT_KEY};
use crate::ids::{ArtifactId, ErrorCode, FactKey, RunId, StateId};
use crate::io::{IoCall, IoProvider, IoResult};
use crate::live_io::FactIndex;
use crate::stores::ArtifactStore;

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
            self.run_id.0, self.state_id.0, self.attempt
        ))
    }

    async fn read_json_payload(
        &self,
        payload_id: &ArtifactId,
    ) -> Result<serde_json::Value, IoError> {
        let bytes = self.artifacts.get(payload_id).await.map_err(|_| {
            IoError::Other(info(
                "fact_payload_get_failed",
                ErrorCategory::Storage,
                false,
                "failed to read fact payload",
            ))
        })?;
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
            IoError::Other(info(
                "fact_payload_decode_failed",
                ErrorCategory::ParsingInput,
                false,
                "failed to decode fact payload",
            ))
        })
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{ErrorCategory, StorageError};
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
            StateId("machine.main.s1".to_string()),
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

    #[tokio::test]
    async fn missing_fact_retryable_follows_run_config() {
        let artifacts = Arc::new(MemArtifactStore::default());
        let facts = FactIndex::default();
        let mut io = ReplayIo::new(
            RunId(uuid::Uuid::new_v4()),
            StateId("machine.main.s1".to_string()),
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
}
