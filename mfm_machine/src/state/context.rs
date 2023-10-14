use anyhow::{anyhow, Error};
use serde::{Deserialize, Serialize};

pub trait Context {
    fn read<T: for<'de> Deserialize<'de>>(&self) -> Result<T, Error>;
    fn write<T: Serialize>(&mut self, data: &T) -> Result<(), Error>;
}

#[derive(Debug)]
pub struct RawContext {
    data: String,
}

impl RawContext {
    pub fn new() -> Self {
        Self {
            data: "{}".to_string(),
        }
    }
}

impl Context for RawContext {
    fn read<T: for<'de> Deserialize<'de>>(&self) -> Result<T, Error> {
        serde_json::from_str(&self.data).map_err(|e| anyhow!("error on deserialize: {}", e))
    }

    fn write<T: Serialize>(&mut self, data: &T) -> Result<(), Error> {
        self.data = serde_json::to_string(data)?;
        Ok(())
    }
}
