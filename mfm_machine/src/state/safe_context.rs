use anyhow::{anyhow, Error, Result};
use serde_json::Value;
use std::fmt;
use std::sync::{Arc, RwLock};

use super::context::{Context, Local};

/// A safer context wrapper that doesn't expose locking details to callers
/// and prevents deadlocks by ensuring safe lock patterns.
#[derive(Clone)]
pub struct SafeContext {
    inner: Arc<RwLock<Box<dyn Context>>>,
}

impl fmt::Debug for SafeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SafeContext")
            .field("inner", &"<context>")
            .finish()
    }
}

impl SafeContext {
    /// Create a new safe context wrapper
    pub fn new<C: Context + 'static>(context: C) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Box::new(context))),
        }
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

    /// Read a typed value from the context
    pub fn read_typed<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<T, Error> {
        let value = self.read_value(key)?;
        let typed_value: T = serde_json::from_value(value)?;
        Ok(typed_value)
    }

    /// Write a typed value to the context
    pub fn write_typed<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<(), Error> {
        let json_value = serde_json::to_value(value)?;
        self.write_value(key, &json_value)
    }

    /// Get a snapshot of the current context
    pub fn snapshot(&self) -> Result<SafeContext, Error> {
        let guard = self
            .inner
            .read()
            .map_err(|_| anyhow!("Failed to acquire read lock on context"))?;

        let snapshot = guard.snapshot()?;

        Ok(Self {
            inner: Arc::new(RwLock::new(snapshot)),
        })
    }

    /// Dumps the entire context as a JSON value
    pub fn dump(&self) -> Result<Value, Error> {
        let guard = self
            .inner
            .read()
            .map_err(|_| anyhow!("Failed to acquire read lock on context"))?;

        guard.dump()
    }

    /// Returns the history of context changes if supported by the implementation
    pub fn history(&self) -> Option<Vec<SafeContext>> {
        let guard = match self.inner.read() {
            Ok(guard) => guard,
            Err(_) => return None,
        };

        let history = guard.history()?;

        Some(
            history
                .into_iter()
                .map(|ctx| SafeContext {
                    inner: Arc::new(RwLock::new(ctx)),
                })
                .collect(),
        )
    }
}

/// Create a new safe context around a default Local context
pub fn create_default_safe_context() -> SafeContext {
    SafeContext::new(Local::default())
}

#[cfg(test)]
mod tests {
    use serde_derive::{Deserialize, Serialize};
    use serde_json::Value;

    use super::*;

    #[test]
    fn test_safe_context_read_write() {
        let ctx = create_default_safe_context();
        let value = Value::String("test_value".to_string());
        ctx.write_value("test_key", &value).unwrap();
        let read_value = ctx.read_value("test_key").unwrap();
        assert_eq!(read_value, value);
    }

    #[test]
    fn test_safe_context_typed() {
        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct TestData {
            field1: String,
            field2: u32,
        }

        let ctx = create_default_safe_context();
        let data = TestData {
            field1: "test".to_string(),
            field2: 42,
        };

        ctx.write_typed("typed_key", &data).unwrap();
        let read_data: TestData = ctx.read_typed("typed_key").unwrap();
        assert_eq!(read_data, data);
    }
}
