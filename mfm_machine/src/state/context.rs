use anyhow::{anyhow, Error};
use serde_json::Value;

pub trait Context {
    fn read_input(&self) -> Result<Value, Error>;
    fn write_output(&mut self, value: &Value) -> Result<(), Error>;
}

mod test {
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
        fn read_input(&self) -> Result<Value, Error> {
            serde_json::to_value(self).map_err(|e| anyhow!(e))
        }

        fn write_output(&mut self, value: &Value) -> Result<(), Error> {
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

        assert_ne!(
            context_a.read_input().unwrap(),
            context_b.read_input().unwrap()
        );

        context_a
            .write_output(&context_b.read_input().unwrap())
            .unwrap();

        assert_eq!(
            context_a.read_input().unwrap(),
            context_b.read_input().unwrap()
        );
    }
}
