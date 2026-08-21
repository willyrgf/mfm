use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use tokio::sync::Mutex;

use crate::{
    CatalogEntries, CatalogEntry, CatalogError, CatalogPutResult, ConfigCatalog, ConfigName,
    MAX_CONFIG_ENTRIES,
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
    fn put_config<'a>(
        &'a self,
        entry: &'a CatalogEntry,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogPutResult, CatalogError>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = self.entries.lock().await;
            if let Some(retained) = entries.get(entry.name()) {
                if retained.digest() == entry.digest()
                    && retained.canonical_bytes() == entry.canonical_bytes()
                {
                    return Ok(CatalogPutResult::Unchanged);
                }
                entries.insert(entry.name().clone(), entry.clone());
                return Ok(CatalogPutResult::Updated);
            }
            if entries.len() >= MAX_CONFIG_ENTRIES {
                return Err(CatalogError::Capacity);
            }
            entries.insert(entry.name().clone(), entry.clone());
            Ok(CatalogPutResult::Inserted)
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
    ) -> Pin<Box<dyn Future<Output = Result<CatalogEntries, CatalogError>> + Send + 'a>> {
        Box::pin(async move {
            let entries = self.entries.lock().await;
            let items = entries.values().cloned().collect::<Vec<_>>();
            CatalogEntries::new(items)
        })
    }
}
