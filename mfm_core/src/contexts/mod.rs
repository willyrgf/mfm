use crate::config::Config;
use anyhow::{anyhow, Error};
use mfm_machine::state::context::Context;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    TomlFile(String),
}

impl Context for ConfigSource {
    fn read(&self) -> Result<Value, Error> {
        serde_json::to_value(self).map_err(|e| anyhow!(e))
    }

    fn write(&mut self, _: &Value) -> Result<(), Error> {
        // do nothing
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ReadConfig {
    pub config_source: ConfigSource,
    pub config: Config,
}
