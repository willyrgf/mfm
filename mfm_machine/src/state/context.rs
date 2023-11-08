use anyhow::{anyhow, Error};
use serde_json::{json, Value};

pub trait Context {
    fn read_input(&self) -> Result<Value, Error>;
    fn write_output(&mut self, data: &Value) -> Result<(), Error>;
}

// Your custom context that implements the library's Context trait.
pub struct MyContext {
    data: Value,
}

impl MyContext {
    pub fn new() -> Self {
        Self { data: json!({}) }
    }
}

impl Context for MyContext {
    fn read_input(&self) -> Result<Value, Error> {
        Ok(self.data.clone())
    }

    fn write_output(&mut self, data: &Value) -> Result<(), Error> {
        self.data = data.clone();
        Ok(())
    }
}

// #[derive(Debug)]
// pub struct RawContext {
//     data: String,
// }
//
// impl RawContext {
//     pub fn new() -> Self {
//         Self {
//             data: "{}".to_string(),
//         }
//     }
// }
//
// impl Context for RawContext {
//     fn read<T: for<'de> Deserialize<'de>>(&self) -> Result<T, Error> {
//         serde_json::from_str(&self.data).map_err(|e| anyhow!("error on deserialize: {}", e))
//     }
//
//     fn write<T: Serialize>(&mut self, data: &T) -> Result<(), Error> {
//         self.data = serde_json::to_string(data)?;
//         Ok(())
//     }
// }
