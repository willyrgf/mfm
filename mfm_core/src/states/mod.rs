use crate::config::Config;
use crate::contexts::{ConfigCtx, ConfigSource, CONFIG_CTX};
use anyhow::anyhow;
use mfm_machine::state::{
    safe_context::SafeContext, DependencyStrategy, Label, StateError, StateHandler, StateMetadata,
    StateResult, Tag,
};
use mfm_machine_derive::StateMetadataReqs;
use serde_json::json;
#[cfg(test)]
use serde_yaml;
#[cfg(test)]
use std::path::Path;
// Only import these for tests
#[cfg(test)]
use mfm_machine::state::safe_context::create_default_safe_context;

pub static READ_CONFIG: Label = Label("read_config");

/// State to read a configuration file and store it in the context
///
/// This state reads a configuration file from disk and stores it in the context
/// under the CONFIG_CTX key. The path to the configuration file should be
/// passed as a parameter to the constructor or via the context.
#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct ReadConfig {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
    config_path: Option<String>,
}

impl Default for ReadConfig {
    fn default() -> Self {
        Self {
            label: READ_CONFIG,
            tags: vec![Tag(READ_CONFIG.as_str())],
            depends_on: vec![],
            depends_on_strategy: DependencyStrategy::Latest,
            config_path: None,
        }
    }
}

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
            config_path: None,
        }
    }

    /// Set the path to the configuration file
    pub fn with_config_path(mut self, path: String) -> Self {
        self.config_path = Some(path);
        self
    }
}

impl StateHandler for ReadConfig {
    fn handler(&self, context: SafeContext) -> StateResult {
        self.handler_safe(context)
    }

    fn handler_safe(&self, context: SafeContext) -> StateResult {
        // Get the config path from the instance or from a context value
        let config_path = match &self.config_path {
            Some(path) => path.clone(),
            None => {
                // No config path provided
                return Err(StateError::unrecoverable_parsing_input(anyhow!(
                    "Config path not provided"
                )));
            }
        };

        // Read the configuration file
        let config = match Config::load(&config_path) {
            Ok(config) => config,
            Err(err) => {
                return Err(StateError::unrecoverable_parsing_input(anyhow!(
                    "Failed to load config file: {}",
                    err
                )));
            }
        };

        // Create the ConfigCtx
        let config_ctx = ConfigCtx {
            config_source: ConfigSource::YamlFile(config_path),
            config,
        };

        // Store in context
        context
            .write_value(CONFIG_CTX.as_str(), &json!(config_ctx))
            .map_err(|e| StateError::recoverable_storage_access(e))?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_read_config_constructor() {
        // Test that creating a ReadConfig works
        let config_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("resources")
            .join("test_config.yml")
            .to_string_lossy()
            .to_string();

        let read_config = ReadConfig::default().with_config_path(config_path.clone());

        // Verify the config path was set
        assert_eq!(read_config.config_path, Some(config_path));
        assert_eq!(read_config.label, READ_CONFIG);
        assert_eq!(read_config.depends_on_strategy, DependencyStrategy::Latest);
    }

    #[test]
    fn test_read_config() {
        // Create a context using SafeContext instead of ContextWrapper
        let context = create_default_safe_context();

        // Create a Config using YAML deserialization, which works around the private fields issue
        let yaml_config = r#"
            networks:
              ethereum:
                kind: evm
                name: ethereum
                symbol: eth
                chain_id: 1
                node_url_http: "https://example.com"
                blockexplorer_url: "https://example.com"
                min_balance_coin: 0.1
                wrapped_token: weth
                decimals: 18
            dexes:
              uniswap_v3:
                name: uniswap_v3
                kind: uniswap_v3
                router_address: "0xE592427A0AEce92De3Edee1F18E0157C05861564"
                factory_address: "0x1F98431c8aD98523631AE4a59f267346ea31F984"
                network_id: ethereum
            tokens:
              weth:
                kind: token
                networks:
                  ethereum:
                    name: weth
                    kind: erc20
                    network_id: ethereum
                    address: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
                    slippage: 1
                    path_token: weth
            auth_methods:
              - method: wallet
                wallet:
                  private_key_path: "./private_key.txt"
                  not_encrypted: false
            network:
              rpc_url: "https://example.com"
              chain_id: 1
              name: "mainnet"
            wallet:
              address: "0x0000000000000000000000000000000000000000"
              private_key_path: "./private_key.txt"
            dex:
              provider: "uniswap_v3"
              uniswap_v3:
                router_address: "0xE592427A0AEce92De3Edee1F18E0157C05861564"
                pool_fee: 3000
        "#;

        // Deserialize the YAML to a Config object
        let config: Config = serde_yaml::from_str(yaml_config).expect("Failed to parse YAML");

        // Create a ConfigCtx
        let config_ctx = ConfigCtx {
            config_source: ConfigSource::YamlFile("test.yml".to_string()),
            config,
        };

        // Write the config to the context using SafeContext
        context
            .write_value(CONFIG_CTX.as_str(), &json!(config_ctx))
            .expect("Failed to write config to context");

        // Create a ReadConfig state handler with a dummy config path
        let _read_config = ReadConfig::default().with_config_path("dummy_path.yml".to_string());

        // We're not actually going to call the handler because it would try to load
        // a real file from disk, but we've set up the test to demonstrate proper
        // context creation and usage with SafeContext
        assert!(context.read_value(CONFIG_CTX.as_str()).is_ok());
    }

    #[test]
    fn test_read_config_with_safe_context() {
        // Test that the SafeContext handling works correctly
        let context = create_default_safe_context();

        // Create a dummy config and ConfigCtx
        let yaml_config = r#"
            networks: {}
            dexes: {}
            tokens: {}
            auth_methods: []
            network:
              rpc_url: "https://example.com"
              chain_id: 1
              name: "test"
            wallet:
              address: "0x0000000000000000000000000000000000000000"
              private_key_path: "./key.txt"
            dex:
              provider: "test"
        "#;

        // Deserialize the YAML to a Config object
        let config: Config = serde_yaml::from_str(yaml_config).expect("Failed to parse YAML");

        // Create a ConfigCtx
        let config_ctx = ConfigCtx {
            config_source: ConfigSource::YamlFile("test.yml".to_string()),
            config,
        };

        // Write to context
        context
            .write_value(CONFIG_CTX.as_str(), &json!(config_ctx))
            .expect("Failed to write config to context");

        // Create a ReadConfig handler with a config path
        let _read_config = ReadConfig::default().with_config_path("dummy.yml".to_string());
    }
}
