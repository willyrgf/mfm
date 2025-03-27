use crate::config::Config;
use crate::contexts::{ConfigCtx, ConfigSource, CONFIG_CTX};
use mfm_machine::state::safe_context::{create_default_safe_context, SafeContext};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Path to the common test configuration file
/// This should be used by all tests that need a configuration
pub fn test_config_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("resources")
        .join("test_config.yml")
}

/// Load the standard test configuration
pub fn load_test_config() -> Config {
    let config_path = test_config_path();
    Config::load(config_path.to_str().unwrap()).expect("Failed to load test_config.yml")
}

/// Create a context with the test configuration loaded
pub fn create_test_context_with_config() -> SafeContext {
    let config_path = test_config_path();
    let config = load_test_config();

    let config_ctx = ConfigCtx {
        config_source: ConfigSource::YamlFile(config_path.to_string_lossy().to_string()),
        config,
    };

    // Set up context
    let context = create_default_safe_context();
    context
        .write_value(CONFIG_CTX.as_str(), &json!(config_ctx))
        .expect("Failed to write to context");

    context
}
