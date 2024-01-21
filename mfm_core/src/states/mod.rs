use mfm_machine_derive::StateMetadataReqs;

use mfm_machine::state::{
    context::ContextWrapper, DependencyStrategy, Label, StateHandler, StateMetadata, StateResult,
    Tag,
};

use crate::contexts::{self, READ_CONFIG};

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct ReadConfig {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl ReadConfig {
    pub fn new() -> Self {
        Self {
            label: Label::new("read_config").unwrap(),
            tags: vec![Tag::new("setup").unwrap()],
            depends_on: vec![Tag::new("config").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}
impl Default for ReadConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHandler for ReadConfig {
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let value = context
            .lock()
            .unwrap()
            .read(READ_CONFIG.to_string())
            .unwrap();

        let data: contexts::ConfigSource = serde_json::from_value(value).unwrap();
        println!("data: {:?}", data);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use mfm_machine::state::{
        context::{wrap_context, Local},
        StateHandler,
    };
    use serde_json::json;

    use crate::contexts::{ConfigSource, READ_CONFIG};

    use super::ReadConfig;

    #[test]
    fn test_readconfig_from_source_file() {
        let state = ReadConfig::new();
        let ctx_input = wrap_context(Local::new(HashMap::from([(
            READ_CONFIG.to_string(),
            json!(ConfigSource::TomlFile("test_config.toml".to_string())),
        )])));
        let result = state.handler(ctx_input);
        assert!(result.is_ok())
    }

    // TODO: add a test transitioning between states and contexts.
}
