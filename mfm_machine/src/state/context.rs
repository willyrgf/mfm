use std::collections::HashMap;

use anyhow::{anyhow, Error, Result};
use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A trait for context storage implementations
/// Contexts store data that gets passed between states
pub trait Context: Send + Sync {
    /// Read a value from the context
    fn read(&self, key: String) -> Result<Value, Error>;

    /// Write a value to the context, returning a new context
    /// This approach enforces immutability - operations don't modify the context but create a new one
    fn write(&self, key: String, value: &Value) -> Result<Box<dyn Context>, Error>;

    /// Dumps the entire context as a JSON value
    fn dump(&self) -> Result<Value, Error>;

    /// Creates a snapshot of the current context state
    fn snapshot(&self) -> Result<Box<dyn Context>, Error>;

    /// Returns the history of context changes if supported by the implementation
    fn history(&self) -> Option<Vec<Box<dyn Context>>>;
}

/// A local in-memory context implementation with history tracking
#[derive(Default, Serialize, Deserialize, Clone)]
pub struct Local {
    map: HashMap<String, Value>,
    history: Vec<HashMap<String, Value>>,
}

impl Local {
    /// Creates a new local context with the given initial data
    pub fn new(map: HashMap<String, Value>) -> Self {
        Self {
            map,
            history: Vec::new(),
        }
    }

    /// Creates a new empty local context
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
            history: Vec::new(),
        }
    }

    /// Get a reference to the internal map
    pub fn get_map(&self) -> &HashMap<String, Value> {
        &self.map
    }
}

impl Context for Local {
    fn read(&self, key: String) -> Result<Value, Error> {
        Ok(self
            .map
            .get(&key)
            .ok_or_else(|| anyhow!("key not found"))?
            .clone())
    }

    fn write(&self, key: String, value: &Value) -> Result<Box<dyn Context>, Error> {
        // Create a new map with the updated value
        let mut new_map = self.map.clone();
        new_map.insert(key, value.clone());

        // Create a new context with the updated map and extended history
        let mut new_history = self.history.clone();
        new_history.push(self.map.clone());

        Ok(Box::new(Self {
            map: new_map,
            history: new_history,
        }))
    }

    fn dump(&self) -> Result<Value, Error> {
        Ok(json!(self))
    }

    fn snapshot(&self) -> Result<Box<dyn Context>, Error> {
        Ok(Box::new(self.clone()))
    }

    fn history(&self) -> Option<Vec<Box<dyn Context>>> {
        let history: Vec<Box<dyn Context>> = self
            .history
            .iter()
            .map(|map| {
                let ctx = Local {
                    map: map.clone(),
                    history: Vec::new(),
                };
                Box::new(ctx) as Box<dyn Context>
            })
            .collect();

        if history.is_empty() {
            None
        } else {
            Some(history)
        }
    }
}

/// A trait for defining context operations in a type-safe way
pub trait TypedContext<T: 'static> {
    /// Read a value of type T from the context
    fn read_typed(&self, key: &str) -> Result<T, Error>
    where
        T: serde::de::DeserializeOwned;

    /// Write a value of type T to the context
    fn write_typed(&self, key: &str, value: &T) -> Result<Box<dyn Context>, Error>
    where
        T: serde::Serialize;
}

impl<C: Context, T: serde::Serialize + serde::de::DeserializeOwned + 'static> TypedContext<T>
    for C
{
    fn read_typed(&self, key: &str) -> Result<T, Error> {
        let value = self.read(key.to_string())?;
        let typed_value: T = serde_json::from_value(value)?;
        Ok(typed_value)
    }

    fn write_typed(&self, key: &str, value: &T) -> Result<Box<dyn Context>, Error> {
        let json_value = serde_json::to_value(value)?;
        self.write(key.to_string(), &json_value)
    }
}

// Implement TypedContext for Box<dyn Context>
impl<T: serde::Serialize + serde::de::DeserializeOwned + 'static> TypedContext<T>
    for Box<dyn Context>
{
    fn read_typed(&self, key: &str) -> Result<T, Error> {
        let value = self.read(key.to_string())?;
        let typed_value: T = serde_json::from_value(value)?;
        Ok(typed_value)
    }

    fn write_typed(&self, key: &str, value: &T) -> Result<Box<dyn Context>, Error> {
        let json_value = serde_json::to_value(value)?;
        self.write(key.to_string(), &json_value)
    }
}

impl std::fmt::Debug for Box<dyn Context> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.dump() {
            Ok(value) => write!(f, "{:?}", value),
            Err(e) => write!(f, "Error dumping context: {:?}", e),
        }
    }
}

#[cfg(test)]
mod test {
    use serde_derive::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    #[test]
    fn test_read_write() {
        let context = Local::default();

        let body = json!({"b1": "test1"});
        let key = "key1".to_string();

        let new_context = context.write(key.clone(), &body).unwrap();
        assert_eq!(new_context.read(key.clone()).unwrap(), body);

        // Original context should be unchanged
        assert!(context.read(key).is_err());
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestData {
        name: String,
        value: i32,
    }

    #[test]
    fn test_typed_context() {
        let context = Local::default();

        let test_data = TestData {
            name: "test".to_string(),
            value: 42,
        };

        let new_context = context.write_typed("test_data", &test_data).unwrap();
        let read_data: TestData = new_context.read_typed("test_data").unwrap();

        assert_eq!(read_data, test_data);
    }

    #[test]
    fn test_context_history() {
        let context = Local::default();

        // Initial state has no history
        assert!(context.history().is_none());

        // Create a new context with a value
        let context1 = context.write("key1".to_string(), &json!("value1")).unwrap();

        // Add another value
        let context2 = context1
            .write("key2".to_string(), &json!("value2"))
            .unwrap();

        // Check history
        let history = context2.history().unwrap();
        assert_eq!(history.len(), 2);

        // First history item should be the empty context
        let first_history_item = &history[0];
        assert!(first_history_item.read("key1".to_string()).is_err());

        // Second history item should have key1
        let second_history_item = &history[1];
        assert_eq!(
            second_history_item.read("key1".to_string()).unwrap(),
            json!("value1")
        );
        assert!(second_history_item.read("key2".to_string()).is_err());
    }
}
