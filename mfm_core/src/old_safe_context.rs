use anyhow::{anyhow, Error, Result};
use mfm_machine::state::context::{Context, ContextWrapper, Local};
use serde_json::Value;
use std::sync::{Arc, RwLock};

/// A safer context wrapper that doesn't expose locking details to callers
/// and prevents deadlocks by ensuring safe lock patterns
pub struct SafeContext {
    inner: Arc<RwLock<Box<dyn Context>>>,
}

impl SafeContext {
    /// Create a new safe context wrapper
    pub fn new<C: Context + 'static>(context: C) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Box::new(context))),
        }
    }

    /// Create from an existing ContextWrapper
    pub fn from_wrapper(wrapper: ContextWrapper) -> Self {
        Self { inner: wrapper }
    }

    /// Convert to the underlying ContextWrapper
    pub fn into_wrapper(self) -> ContextWrapper {
        self.inner
    }

    /// Read a value atomically
    pub fn read_value(&self, key: &str) -> Result<Value, Error> {
        let guard = self
            .inner
            .read()
            .map_err(|_| anyhow!("Failed to acquire read lock on context"))?;

        guard.read(key.to_string())
    }

    /// Write a value atomically
    pub fn write_value(&self, key: &str, value: &Value) -> Result<(), Error> {
        // Immediately acquire write lock - never upgrade from read to write
        let mut guard = self
            .inner
            .write()
            .map_err(|_| anyhow!("Failed to acquire write lock on context"))?;

        // Get the new context with the updated value
        let new_context = guard.write(key.to_string(), value)?;

        // Replace the current context with the new one
        *guard = new_context;

        Ok(())
    }
}

/// Create a new safe context around a default Local context
pub fn create_default_safe_context() -> SafeContext {
    SafeContext::new(Local::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_safe_context_read_write() {
        let ctx = create_default_safe_context();

        // Write a value
        let test_value = json!({"test": "value"});
        ctx.write_value("test_key", &test_value).unwrap();

        // Read it back
        let read_value = ctx.read_value("test_key").unwrap();

        assert_eq!(read_value, test_value);
    }
}
