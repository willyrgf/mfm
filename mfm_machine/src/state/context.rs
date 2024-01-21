use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Error};
use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value};

pub type ContextWrapper = Arc<Mutex<Box<dyn Context>>>;

// TODO: rethink this implementation of kv store context;
// we should be able to express context constraints for each state
// at the type system level;
#[derive(Default, Serialize, Deserialize)]
pub struct Local {
    map: HashMap<String, Value>,
}

impl Local {
    pub fn new(map: HashMap<String, Value>) -> Self {
        Self { map }
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

    fn write(&mut self, key: String, value: &Value) -> Result<(), Error> {
        self.map.insert(key, value.clone());
        Ok(())
    }

    fn dump(&self) -> Result<Value, Error> {
        Ok(json!(self))
    }
}

pub trait Context {
    fn read(&self, key: String) -> Result<Value, Error>;
    fn write(&mut self, key: String, value: &Value) -> Result<(), Error>;
    fn dump(&self) -> Result<Value, Error>;
}

pub fn wrap_context<C: Context + 'static>(context: C) -> ContextWrapper {
    #[allow(clippy::arc_with_non_send_sync)]
    Arc::new(Mutex::new(Box::new(context)))
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_read_write() {
        let context_a: &mut dyn Context = &mut Local::default();

        let body = json!({"b1": "test1"});
        let key = "key1".to_string();

        context_a.write(key.clone(), &body).unwrap();

        assert_eq!(context_a.read(key).unwrap(), body);
    }
}
