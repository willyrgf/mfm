//! Internal context + snapshot implementations for the v4 engine.
//!
//! Not part of the stable API contract (Appendix C.1).

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::context::DynContext;
use crate::errors::{ContextError, ErrorCategory, ErrorInfo, RunError};
use crate::hashing::{
    canonical_json_bytes, put_artifact_verified, verify_artifact_bytes, CanonicalJsonError,
};
use crate::ids::{ArtifactId, ContextKey, ErrorCode};
use crate::stores::{ArtifactKind, ArtifactStore};

fn info(code: &'static str, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category: ErrorCategory::Context,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

/// A simple JSON key/value context implementation backed by a map.
///
/// This is intended for internal engine use and tests. It is not part of the stable API contract.
#[derive(Clone, Default)]
pub(crate) struct JsonContext {
    entries: BTreeMap<String, serde_json::Value>,
}

impl JsonContext {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_snapshot(snapshot: serde_json::Value) -> Result<Self, ContextError> {
        let serde_json::Value::Object(m) = snapshot else {
            return Err(ContextError::Serialization(info(
                "context_snapshot_not_object",
                "context snapshot must be a JSON object",
            )));
        };

        let mut entries = BTreeMap::new();
        for (k, v) in m {
            entries.insert(k, v);
        }

        Ok(Self { entries })
    }
}

impl DynContext for JsonContext {
    fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
        Ok(self.entries.get(&key.0).cloned())
    }

    fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
        self.entries.insert(key.0, value);
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
        self.entries.remove(&key.0);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, ContextError> {
        let mut m = serde_json::Map::with_capacity(self.entries.len());
        for (k, v) in &self.entries {
            m.insert(k.clone(), v.clone());
        }
        Ok(serde_json::Value::Object(m))
    }
}

/// Overlay context used for attempt-scoped staging.
///
/// Reads fall back to the base snapshot; writes and deletes are recorded in the overlay.
pub(crate) struct StagedContext {
    base: JsonContext,
    overlay: BTreeMap<String, Option<serde_json::Value>>,
}

impl StagedContext {
    pub(crate) fn new(base: JsonContext) -> Self {
        Self {
            base,
            overlay: BTreeMap::new(),
        }
    }

    pub(crate) fn commit(mut self) -> JsonContext {
        for (k, v) in self.overlay {
            match v {
                Some(v) => {
                    self.base.entries.insert(k, v);
                }
                None => {
                    self.base.entries.remove(&k);
                }
            }
        }

        self.base
    }

    pub(crate) fn discard(self) -> JsonContext {
        self.base
    }
}

impl DynContext for StagedContext {
    fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
        if let Some(v) = self.overlay.get(&key.0) {
            return Ok(v.clone());
        }
        self.base.read(key)
    }

    fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
        self.overlay.insert(key.0, Some(value));
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
        self.overlay.insert(key.0.clone(), None);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, ContextError> {
        let mut entries = self.base.entries.clone();
        for (k, v) in &self.overlay {
            match v {
                Some(v) => {
                    entries.insert(k.clone(), v.clone());
                }
                None => {
                    entries.remove(k);
                }
            }
        }

        let mut m = serde_json::Map::with_capacity(entries.len());
        for (k, v) in entries {
            m.insert(k, v);
        }

        Ok(serde_json::Value::Object(m))
    }
}

pub(crate) async fn write_full_snapshot(
    artifacts: &dyn ArtifactStore,
    ctx: &dyn DynContext,
) -> Result<ArtifactId, RunError> {
    let snapshot = ctx.dump().map_err(RunError::Context)?;
    write_full_snapshot_value(artifacts, snapshot).await
}

pub(crate) async fn write_full_snapshot_value(
    artifacts: &dyn ArtifactStore,
    snapshot: serde_json::Value,
) -> Result<ArtifactId, RunError> {
    let bytes = canonical_json_bytes(&snapshot).map_err(|e| match e {
        CanonicalJsonError::FloatNotAllowed => {
            RunError::Context(ContextError::Serialization(info(
                "context_snapshot_not_canonical",
                "context snapshot is not canonical-json-hashable (floats are forbidden)",
            )))
        }
        CanonicalJsonError::SecretsNotAllowed => RunError::Context(ContextError::Other(info(
            "secrets_detected",
            "context snapshot contained secrets (policy forbids persisting secrets)",
        ))),
    })?;

    put_artifact_verified(artifacts, ArtifactKind::ContextSnapshot, bytes)
        .await
        .map_err(RunError::Storage)
}

pub(crate) async fn read_full_snapshot_value(
    artifacts: &dyn ArtifactStore,
    snapshot_id: &ArtifactId,
) -> Result<serde_json::Value, RunError> {
    let bytes = artifacts
        .get(snapshot_id)
        .await
        .map_err(RunError::Storage)?;
    verify_artifact_bytes(snapshot_id, &bytes).map_err(RunError::Storage)?;
    serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
        RunError::Context(ContextError::Serialization(info(
            "context_snapshot_decode_failed",
            "failed to decode context snapshot JSON",
        )))
    })
}

pub(crate) async fn read_json_context(
    artifacts: &dyn ArtifactStore,
    snapshot_id: &ArtifactId,
) -> Result<JsonContext, RunError> {
    let snapshot = read_full_snapshot_value(artifacts, snapshot_id).await?;
    JsonContext::from_snapshot(snapshot).map_err(RunError::Context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::StorageError;
    use crate::hashing::artifact_id_for_bytes;
    use crate::stores::ArtifactKind;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Arc;
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
                StorageError::NotFound(ErrorInfo {
                    code: ErrorCode("not_found".to_string()),
                    category: ErrorCategory::Storage,
                    retryable: false,
                    message: "artifact not found".to_string(),
                    details: None,
                })
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
            unreachable!("snapshot write should not read artifacts")
        }

        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            unreachable!("snapshot write should not query existence")
        }
    }

    struct WrongBytesArtifactStore {
        bytes: Vec<u8>,
    }

    #[async_trait]
    impl ArtifactStore for WrongBytesArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            unreachable!("snapshot read should not write artifacts")
        }

        async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            Ok(self.bytes.clone())
        }

        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn full_snapshot_ids_are_deterministic_for_same_logical_context() {
        let store = MemArtifactStore::default();

        let mut a = JsonContext::new();
        a.write(ContextKey("b".to_string()), serde_json::json!(2))
            .unwrap();
        a.write(ContextKey("a".to_string()), serde_json::json!(1))
            .unwrap();
        let id_a = write_full_snapshot(&store, &a).await.unwrap();

        let mut b = JsonContext::new();
        b.write(ContextKey("a".to_string()), serde_json::json!(1))
            .unwrap();
        b.write(ContextKey("b".to_string()), serde_json::json!(2))
            .unwrap();
        let id_b = write_full_snapshot(&store, &b).await.unwrap();

        assert_eq!(id_a, id_b);
    }

    #[tokio::test]
    async fn full_snapshot_write_rejects_store_returned_wrong_id() {
        let mut ctx = JsonContext::new();
        ctx.write(ContextKey("a".to_string()), serde_json::json!(1))
            .unwrap();

        let err = write_full_snapshot(&WrongIdArtifactStore, &ctx)
            .await
            .expect_err("wrong store-returned id must fail snapshot write");

        match err {
            RunError::Storage(StorageError::Corruption(info)) => {
                assert_eq!(info.code.0, "artifact_put_id_mismatch")
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn full_snapshot_read_rejects_bytes_that_do_not_match_id() {
        let expected_id = artifact_id_for_bytes(br#"{"a":1}"#);
        let store = WrongBytesArtifactStore {
            bytes: br#"{"a":2}"#.to_vec(),
        };

        let err = read_full_snapshot_value(&store, &expected_id)
            .await
            .expect_err("wrong bytes must fail snapshot read");

        match err {
            RunError::Storage(StorageError::Corruption(info)) => {
                assert_eq!(info.code.0, "artifact_content_address_mismatch")
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn staged_context_commit_and_discard_semantics() {
        let mut base = JsonContext::new();
        base.write(ContextKey("x".to_string()), serde_json::json!(1))
            .unwrap();

        let mut staged = StagedContext::new(base.clone());
        staged
            .write(ContextKey("x".to_string()), serde_json::json!(2))
            .unwrap();
        staged
            .write(ContextKey("y".to_string()), serde_json::json!(3))
            .unwrap();
        staged.delete(&ContextKey("x".to_string())).unwrap();

        let committed = staged.commit();
        assert_eq!(committed.read(&ContextKey("x".to_string())).unwrap(), None);
        assert_eq!(
            committed.read(&ContextKey("y".to_string())).unwrap(),
            Some(serde_json::json!(3))
        );

        // Discard returns the base unchanged.
        let staged = StagedContext::new(base.clone());
        let discarded = staged.discard();
        assert_eq!(
            discarded.read(&ContextKey("x".to_string())).unwrap(),
            Some(serde_json::json!(1))
        );
    }
}
