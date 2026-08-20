use std::collections::BTreeMap;
use std::future::Future;
use std::ops::Bound::{Excluded, Unbounded};
use std::pin::Pin;

use tokio::sync::Mutex;

use crate::{
    CatalogDeleteResult, CatalogEntry, CatalogError, CatalogInsertResult, CatalogPage,
    ConfigCatalog, ConfigCursor, ConfigDigest, ConfigName, PageLimit, MAX_CONFIG_ENTRIES,
};

/// In-memory atomic config custody for hermetic composition and tests.
pub struct MemoryCatalog {
    entries: Mutex<BTreeMap<ConfigName, CatalogEntry>>,
}

impl MemoryCatalog {
    /// Constructs an empty catalog.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }
}

impl Default for MemoryCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigCatalog for MemoryCatalog {
    fn insert_config<'a>(
        &'a self,
        entry: &'a CatalogEntry,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogInsertResult, CatalogError>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = self.entries.lock().await;
            if let Some(retained) = entries.get(entry.name()) {
                return Ok(
                    if retained.digest() == entry.digest()
                        && retained.canonical_bytes() == entry.canonical_bytes()
                    {
                        CatalogInsertResult::Unchanged
                    } else {
                        CatalogInsertResult::Conflict
                    },
                );
            }
            if entries.len() >= MAX_CONFIG_ENTRIES {
                return Err(CatalogError::Capacity);
            }
            entries.insert(entry.name().clone(), entry.clone());
            Ok(CatalogInsertResult::Inserted)
        })
    }

    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
    ) -> Pin<Box<dyn Future<Output = Result<Option<CatalogEntry>, CatalogError>> + Send + 'a>> {
        Box::pin(async move { Ok(self.entries.lock().await.get(name).cloned()) })
    }

    fn list_configs<'a>(
        &'a self,
        cursor: Option<&'a ConfigCursor>,
        limit: PageLimit,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogPage, CatalogError>> + Send + 'a>> {
        Box::pin(async move {
            let entries = self.entries.lock().await;
            let start = cursor.map_or(Unbounded, |cursor| Excluded(cursor.after()));
            let mut page = entries
                .range::<ConfigName, _>((start, Unbounded))
                .take(limit.get() + 1)
                .map(|(_, entry)| entry.clone())
                .collect::<Vec<_>>();
            let has_more = page.len() > limit.get();
            if has_more {
                page.pop();
            }
            let next_cursor = if has_more {
                let last = page.last().ok_or(CatalogError::Corrupt)?;
                Some(ConfigCursor::after_name(last.name().clone()))
            } else {
                None
            };
            CatalogPage::new(page, next_cursor)
        })
    }

    fn delete_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogDeleteResult, CatalogError>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = self.entries.lock().await;
            let Some(retained) = entries.get(name) else {
                return Ok(CatalogDeleteResult::Absent);
            };
            if retained.digest() != digest {
                return Ok(CatalogDeleteResult::DigestMismatch);
            }
            entries.remove(name);
            Ok(CatalogDeleteResult::Deleted)
        })
    }
}
