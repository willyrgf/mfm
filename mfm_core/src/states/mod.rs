use mfm_machine_derive::StateMetadataReqs;

use mfm_machine::state::{
    context::ContextWrapper, DependencyStrategy, Label, StateHandler, StateMetadata, StateResult,
    Tag,
};

use crate::contexts;

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct ReadConfig {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl ReadConfig {
    fn new() -> Self {
        Self {
            label: Label::new("read_config").unwrap(),
            tags: vec![Tag::new("setup").unwrap()],
            depends_on: vec![Tag::new("config").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for ReadConfig {
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let value = context.lock().unwrap().read().unwrap();
        let data: contexts::ConfigSource = serde_json::from_value(value).unwrap();
        println!("data: {:?}", data);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use mfm_machine::state::{context::wrap_context, StateHandler};

    use crate::contexts::ConfigSource;

    use super::ReadConfig;

    #[test]
    fn test_readconfig_from_source_file() {
        let state = ReadConfig::new();
        let ctx_input = wrap_context(ConfigSource::File("test_config.toml".to_string()));
        let result = state.handler(ctx_input);
        assert!(result.is_ok())
    }

    // TODO: add a test transitioning between states and contexts.
}
