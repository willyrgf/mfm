use mfm_machine_derive::StateMetadataReqs;

use mfm_machine::state::{
    context::ContextWrapper, DependencyStrategy, Label, StateHandler, StateMetadata, StateResult,
    Tag,
};
use serde_json::json;

use crate::{
    config::Config,
    contexts::{ConfigCtx, ConfigSource, CONFIG_CTX, CONFIG_SOURCE_CTX},
    read_yaml,
};

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct ReadConfig {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

pub static READ_CONFIG: Label = Label("read_config");

impl ReadConfig {
    pub fn new(
        tags: Vec<Tag>,
        depends_on: Vec<Tag>,
        depends_on_strategy: DependencyStrategy,
    ) -> Self {
        Self {
            label: READ_CONFIG,
            tags,
            depends_on,
            depends_on_strategy,
        }
    }
}
impl Default for ReadConfig {
    fn default() -> Self {
        Self {
            label: READ_CONFIG,
            tags: vec![Tag::new("setup").unwrap()],
            depends_on: vec![Tag::new("config").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

// TODO: implement error froms to be able to use `?` for context ops.
impl StateHandler for ReadConfig {
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let config_ctx = context
            .lock()
            .unwrap()
            .read(CONFIG_SOURCE_CTX.into())
            .unwrap();

        let config_source: ConfigSource = serde_json::from_value(config_ctx).unwrap();

        let ConfigSource::YamlFile(path) = config_source.clone();

        let config: Config = read_yaml(path).unwrap();

        let config_ctx = ConfigCtx {
            config_source,
            config,
        };

        let value = serde_json::to_value(config_ctx.clone()).unwrap();

        context
            .lock()
            .unwrap()
            .write(CONFIG_CTX.into(), &value.clone())
            .unwrap();

        // let config_value = context.lock().unwrap().read(CONFIG_CTX.into()).unwrap();
        //
        // println!("value: \n{:?}", value);
        // println!("config_value: \n{:?}", config_value);
        //
        // let config_from_ctx: ConfigCtx = serde_json::from_value(config_value.clone()).unwrap();
        // //assert_eq!(config_value, value);
        // //assert_eq!(config_ctx, config_from_ctx);

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

    use crate::{
        config::Config,
        contexts::{ConfigCtx, ConfigSource, CONFIG_CTX, CONFIG_SOURCE_CTX},
        read_yaml,
    };

    use super::ReadConfig;

    #[test]
    fn test_readconfig_from_source_file() {
        let state = ReadConfig::default();
        let path = "test_config.yml".to_string();
        let ctx_input = wrap_context(Local::new(HashMap::from([(
            CONFIG_SOURCE_CTX.into(),
            json!(ConfigSource::YamlFile(path.clone())),
        )])));
        let result = state.handler(ctx_input.clone());

        let config_value = ctx_input.lock().unwrap().read(CONFIG_CTX.into()).unwrap();
        let config_from_ctx: ConfigCtx = serde_json::from_value(config_value.clone()).unwrap();
        assert_eq!(json!(config_value), json!(""));
        let config_from_file: Config = read_yaml(path).unwrap();

        assert!(result.is_ok());
        assert_eq!(config_from_ctx.config, config_from_file);
    }

    // TODO: add a test transitioning between states and contexts.
}
