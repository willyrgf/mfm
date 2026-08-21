use std::collections::BTreeMap;

use tokio::sync::Mutex;

use crate::{
    ConfigDigest, ConfigFuture, ConfigImportResult, ConfigName, ConfigRepository,
    ConfigRepositoryError, ConfigRevision,
};

/// In-memory atomic configuration custody for hermetic composition and tests.
#[derive(Default)]
pub struct MemoryConfigRepository {
    revisions: Mutex<BTreeMap<(ConfigName, ConfigDigest), ConfigRevision>>,
}

impl ConfigRepository for MemoryConfigRepository {
    fn import_config<'a>(
        &'a self,
        revision: &'a ConfigRevision,
    ) -> ConfigFuture<'a, ConfigImportResult> {
        Box::pin(async move {
            let mut revisions = self.revisions.lock().await;
            let key = (revision.name().clone(), revision.digest().clone());
            if let Some(existing) = revisions.get(&key) {
                return if existing.canonical_bytes() == revision.canonical_bytes() {
                    Ok(ConfigImportResult::Unchanged)
                } else {
                    Err(ConfigRepositoryError::Corrupt)
                };
            }
            revisions.insert(key, revision.clone());
            Ok(ConfigImportResult::Created)
        })
    }

    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, Option<ConfigRevision>> {
        Box::pin(async move {
            Ok(self
                .revisions
                .lock()
                .await
                .get(&(name.clone(), digest.clone()))
                .cloned())
        })
    }

    fn list_configs(&self) -> ConfigFuture<'_, Vec<ConfigRevision>> {
        Box::pin(async move { Ok(self.revisions.lock().await.values().cloned().collect()) })
    }

    fn delete_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, ()> {
        Box::pin(async move {
            self.revisions
                .lock()
                .await
                .remove(&(name.clone(), digest.clone()));
            Ok(())
        })
    }
}
