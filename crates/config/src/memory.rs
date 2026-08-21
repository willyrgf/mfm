use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use tokio::sync::Mutex;

use crate::{
    ConfigDigest, ConfigImportResult, ConfigName, ConfigRepository, ConfigRepositoryError,
    ConfigRevision, ConfigRevisions, RetainedConfigRevision,
};

struct RetainedName {
    current: ConfigDigest,
    revisions: BTreeMap<ConfigDigest, ConfigRevision>,
}

/// In-memory atomic configuration custody for hermetic composition and tests.
pub struct MemoryConfigRepository {
    names: Mutex<BTreeMap<ConfigName, RetainedName>>,
}

impl MemoryConfigRepository {
    /// Constructs an empty repository.
    pub fn new() -> Self {
        Self {
            names: Mutex::new(BTreeMap::new()),
        }
    }
}

impl Default for MemoryConfigRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigRepository for MemoryConfigRepository {
    fn import_config<'a>(
        &'a self,
        revision: &'a ConfigRevision,
    ) -> Pin<Box<dyn Future<Output = Result<ConfigImportResult, ConfigRepositoryError>> + Send + 'a>>
    {
        Box::pin(async move {
            let mut names = self.names.lock().await;
            let Some(retained) = names.get_mut(revision.name()) else {
                names.insert(
                    revision.name().clone(),
                    RetainedName {
                        current: revision.digest().clone(),
                        revisions: BTreeMap::from([(revision.digest().clone(), revision.clone())]),
                    },
                );
                return Ok(ConfigImportResult::Created);
            };
            if let Some(existing) = retained.revisions.get(revision.digest()) {
                if existing.canonical_bytes() != revision.canonical_bytes() {
                    return Err(ConfigRepositoryError::Corrupt);
                }
                if retained.current == *revision.digest() {
                    return Ok(ConfigImportResult::Unchanged);
                }
            } else {
                retained
                    .revisions
                    .insert(revision.digest().clone(), revision.clone());
            }
            retained.current = revision.digest().clone();
            Ok(ConfigImportResult::Updated)
        })
    }

    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: Option<&'a ConfigDigest>,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<ConfigRevision>, ConfigRepositoryError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let names = self.names.lock().await;
            let Some(retained) = names.get(name) else {
                return Ok(None);
            };
            let digest = digest.unwrap_or(&retained.current);
            Ok(retained.revisions.get(digest).cloned())
        })
    }

    fn list_configs<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<ConfigRevisions, ConfigRepositoryError>> + Send + 'a>>
    {
        Box::pin(async move {
            let names = self.names.lock().await;
            let items = names
                .values()
                .flat_map(|retained| {
                    retained.revisions.values().cloned().map(|revision| {
                        let current = revision.digest() == &retained.current;
                        RetainedConfigRevision::new(revision, current)
                    })
                })
                .collect();
            ConfigRevisions::new(items)
        })
    }
}
