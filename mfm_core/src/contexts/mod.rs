use crate::config::Config;
use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    TomlFile(String),
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ReadConfig {
    pub config_source: ConfigSource,
    pub config: Config,
}
pub const READ_CONFIG: &str = "read_config";
