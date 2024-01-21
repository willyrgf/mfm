use crate::config::Config;
use mfm_machine::state::Label;
use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    YamlFile(String),
}
pub const CONFIG_SOURCE_CTX: Label = Label("config_source_ctx");

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ConfigCtx {
    pub config_source: ConfigSource,
    pub config: Config,
}
pub const CONFIG_CTX: Label = Label("config_ctx");
