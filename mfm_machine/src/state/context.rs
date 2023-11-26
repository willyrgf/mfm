use std::sync::{Arc, Mutex};

use anyhow::Error;
use serde_json::Value;

pub type ContextWrapper = Arc<Mutex<Box<dyn Context>>>;

pub trait Context {
    fn read(&self) -> Result<Value, Error>;
    fn write(&mut self, value: &Value) -> Result<(), Error>;
}

pub fn wrap_context<C: Context + 'static>(context: C) -> ContextWrapper {
    #[allow(clippy::arc_with_non_send_sync)]
    Arc::new(Mutex::new(Box::new(context)))
}

#[cfg(test)]
mod test {
    use anyhow::anyhow;
    use serde_derive::{Deserialize, Serialize};

    use super::*;

    #[derive(Serialize, Deserialize)]
    struct ContextA {
        a: String,
        b: u64,
    }

    impl ContextA {
        fn _new(a: String, b: u64) -> Self {
            Self { a, b }
        }
    }

    impl Context for ContextA {
        fn read(&self) -> Result<Value, Error> {
            serde_json::to_value(self).map_err(|e| anyhow!(e))
        }

        fn write(&mut self, value: &Value) -> Result<(), Error> {
            let ctx: ContextA = serde_json::from_value(value.clone()).map_err(|e| anyhow!(e))?;
            self.a = ctx.a;
            self.b = ctx.b;
            Ok(())
        }
    }

    #[test]
    fn test_read_write() {
        let context_a: &mut dyn Context = &mut ContextA::_new(String::from("hello"), 7);
        let context_b: &dyn Context = &ContextA::_new(String::from("hellow"), 9);

        assert_ne!(context_a.read().unwrap(), context_b.read().unwrap());

        context_a.write(&context_b.read().unwrap()).unwrap();

        assert_eq!(context_a.read().unwrap(), context_b.read().unwrap());
    }
}
