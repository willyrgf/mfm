use std::path::PathBuf;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::{connect_run_services, RunStoresArgs};
use clap::{Args, ValueEnum};
use mfm_app::{
    DistinctRunKey, EntryPointRunLaunchInput, PublicOpName, PublicOutputResponse,
    RunLaunchOutcomeStatus, RunResponse,
};
use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};
use serde::Serialize;

/// Arguments for `mfm run start`.
#[derive(Args)]
pub(crate) struct StartArgs {
    /// Public entry-point operation name.
    #[arg(long, value_name = "NAME")]
    pub op: String,

    /// Authored operation config file.
    #[arg(long, value_name = "PATH")]
    pub config: PathBuf,

    /// Optional public operation version. Defaults to the latest registered version.
    #[arg(long, value_name = "VERSION")]
    pub op_version: Option<u32>,

    /// Authored config format.
    #[arg(long, value_enum, default_value_t = ConfigFormatArg::Toml)]
    pub config_format: ConfigFormatArg,

    /// Caller-supplied key that forces a distinct run of otherwise identical certified work.
    #[arg(long, value_name = "KEY")]
    pub distinct_run_key: Option<String>,

    /// Runtime configuration file for live capabilities (default: $MFM_RUNTIME_CONFIG_FILE).
    #[arg(long, value_name = "PATH")]
    pub runtime_config: Option<PathBuf>,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// CLI spelling for authored config formats.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ConfigFormatArg {
    /// TOML authored config.
    Toml,
    /// JSON authored config.
    Json,
}

impl From<ConfigFormatArg> for AuthoredConfigFormat {
    fn from(value: ConfigFormatArg) -> Self {
        match value {
            ConfigFormatArg::Toml => Self::Toml,
            ConfigFormatArg::Json => Self::Json,
        }
    }
}

impl std::fmt::Display for ConfigFormatArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toml => f.write_str("toml"),
            Self::Json => f.write_str("json"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct StartOutput {
    outcome: RunLaunchOutcomeStatus,
    run: RunResponse,
    public_output: Option<PublicOutputResponse>,
}

impl std::fmt::Display for StartOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.public_output {
            Some(public_output) => write!(f, "launch_outcome={} {public_output}", self.outcome),
            None => write!(f, "launch_outcome={} {}", self.outcome, self.run),
        }
    }
}

/// Executes the start command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &StartArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StartArgs) -> CommandResult<StartOutput> {
    let public_op_name = PublicOpName::new(&args.op)?;
    let op_version = args.op_version.map(mfm_app::OpVersion::new).transpose()?;
    let distinct_run_key = args
        .distinct_run_key
        .as_deref()
        .map(DistinctRunKey::new)
        .transpose()?;
    let config_bytes = tokio::fs::read(&args.config)
        .await
        .map_err(|_| CommandError::backend("AuthoredConfigReadFailed", "Failed to read config"))?;
    let authored_config = AuthoredConfig::new(args.config_format.into(), config_bytes)?;
    let entry_point_registry = mfm_app::production_entry_point_op_registry()?;
    let certification_registry = mfm_app::production_certification_registry()?;
    let services = connect_run_services(&args.stores, args.runtime_config.as_deref()).await?;
    let trust_scope_id = services.load_trust_scope_id().await?;
    let prepared = mfm_app::prepare_entry_point_run_launch(EntryPointRunLaunchInput {
        entry_point_registry: &entry_point_registry,
        public_op_name,
        op_version,
        authored_config,
        certification_registry: &certification_registry,
        trust_scope_id,
        distinct_run_key,
    })?;
    let report = services.launch_prepared_entry_point_run(prepared).await?;
    Ok(CommandOutput::new(StartOutput {
        outcome: report.outcome,
        run: report.run,
        public_output: report.public_output,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn start_rejects_empty_distinct_run_key_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.json");
        std::fs::write(&config, "{}").expect("write config");

        let mut args = start_args(config, RunStoresArgs { database_url: None });
        args.distinct_run_key = Some(String::new());

        let err = execute_internal(&args)
            .await
            .expect_err("empty distinct run key rejects before store construction");

        assert_eq!(err.code, "DistinctRunKeyInvalid");
    }

    #[tokio::test]
    async fn start_requires_store_trust_scope_before_entry_point_resolution() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.json");
        std::fs::write(&config, "{}").expect("write config");

        let mut args = start_args(config, RunStoresArgs { database_url: None });
        args.op = "unknown_op".to_owned();

        let err = execute_internal(&args)
            .await
            .expect_err("start requires store trust scope");

        assert_eq!(err.code, "MissingDatabaseUrl");
    }

    #[tokio::test]
    async fn start_requires_store_trust_scope_before_op_config_decode() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.json");
        std::fs::write(&config, r#"{"portfolio":{"portfolio_id":1}}"#).expect("write config");

        let err = execute_internal(&start_args(config, RunStoresArgs { database_url: None }))
            .await
            .expect_err("start requires store trust scope");

        assert_eq!(err.code, "MissingDatabaseUrl");
    }

    #[tokio::test]
    async fn start_accepts_entry_point_material_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.json");
        std::fs::write(&config, sample_portfolio_config_json()).expect("write config");

        let err = execute_internal(&start_args(config, RunStoresArgs { database_url: None }))
            .await
            .expect_err("valid entry-point launch proceeds to store construction");

        assert_eq!(err.code, "MissingDatabaseUrl");
    }

    #[tokio::test]
    async fn start_defaults_config_format_to_toml_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.toml");
        std::fs::write(&config, sample_portfolio_config_toml()).expect("write config");

        let mut args = start_args(config, RunStoresArgs { database_url: None });
        args.config_format = ConfigFormatArg::Toml;

        let err = execute_internal(&args)
            .await
            .expect_err("valid TOML entry-point launch proceeds to store construction");

        assert_eq!(err.code, "MissingDatabaseUrl");
    }

    #[tokio::test]
    async fn start_accepts_evm_entry_points_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let registry = mfm_app::production_entry_point_op_registry().expect("registry");

        for (index, (op, config)) in evm_entry_point_configs().into_iter().enumerate() {
            let public_op_name = PublicOpName::new(op).expect("public op name");
            let current_version = registry
                .resolve_latest(&public_op_name)
                .expect("EVM entry-point")
                .version();
            assert_eq!(current_version.get(), 2, "{op}");
            registry
                .resolve_version(&public_op_name, current_version)
                .expect("explicit EVM entry-point version");

            let config_path = tmp.path().join(format!("{op}-{index}.json"));
            std::fs::write(&config_path, config.to_string()).expect("write config");
            let mut args = start_args(config_path, RunStoresArgs { database_url: None });
            args.op = op.to_owned();
            args.op_version = Some(current_version.get());
            args.config_format = ConfigFormatArg::Json;

            let err = execute_internal(&args)
                .await
                .expect_err("valid EVM entry-point launch proceeds to store construction");

            assert_eq!(err.code, "MissingDatabaseUrl", "{op}");
        }
    }

    #[test]
    fn start_rejects_old_evm_configure_validate_envelopes_at_public_schema_boundary() {
        for (op, config) in [
            ("evm_contract_configure", old_configure_entry_config_json()),
            ("evm_contract_validate", old_validate_entry_config_json()),
        ] {
            let Err(err) = prepare_cli_entry_point_for_test(op, config) else {
                panic!("{op} old envelope must not prepare a launch");
            };

            assert_eq!(err.code, "AuthoredConfigDecodeFailed", "{op}");
        }
    }

    fn prepare_cli_entry_point_for_test(
        op: &str,
        config: serde_json::Value,
    ) -> Result<mfm_app::PreparedEntryPointRunLaunch, CommandError> {
        let entry_point_registry = mfm_app::production_entry_point_op_registry()?;
        let certification_registry = mfm_app::production_certification_registry()?;
        let authored_config = AuthoredConfig::new(
            AuthoredConfigFormat::Json,
            serde_json::to_vec(&config).expect("config json"),
        )?;
        let trust_scope_id =
            mfm_ids::TrustScopeId::new("mfm.trust_scope.v1:43434343434343434343434343434343")
                .expect("trust scope");

        mfm_app::prepare_entry_point_run_launch(EntryPointRunLaunchInput {
            entry_point_registry: &entry_point_registry,
            public_op_name: PublicOpName::new(op)?,
            op_version: Some(mfm_app::OpVersion::new(2)?),
            authored_config,
            certification_registry: &certification_registry,
            trust_scope_id,
            distinct_run_key: None,
        })
        .map_err(CommandError::from)
    }

    fn start_args(config: PathBuf, stores: RunStoresArgs) -> StartArgs {
        StartArgs {
            op: "portfolio_snapshot".to_owned(),
            config,
            op_version: None,
            config_format: ConfigFormatArg::Json,
            distinct_run_key: None,
            runtime_config: None,
            stores,
        }
    }

    fn sample_portfolio_config_json() -> String {
        serde_json::json!({
            "portfolio": {
                "portfolio_id": "portfolio_main",
                "quote_codes": ["USD"],
                "networks": [
                    {
                        "network_id": "ethereum-mainnet",
                        "family": "evm",
                        "chain_id": 1,
                        "control_scope": "shared",
                        "metadata": {}
                    }
                ],
                "wallets": [
                    {
                        "wallet_id": "wallet_main",
                        "subject": {
                            "kind": "evm_address",
                            "address": "0x000000000000000000000000000000000000dead"
                        },
                        "implementation": { "kind": "address_only" },
                        "network_id": "ethereum-mainnet",
                        "symbol_ids": ["eth.native.ethereum-mainnet"],
                        "metadata": {}
                    }
                ],
                "symbol_configs": [
                    {
                        "symbol_id": "eth.native.ethereum-mainnet",
                        "display_symbol": "ETH",
                        "kind": "native_balance",
                        "role": "native",
                        "network_id": "ethereum-mainnet",
                        "protocol": null,
                        "balance_reader": { "kind": "native_balance" },
                        "valuation": {
                            "quotes": [
                                {
                                    "quote": "USD",
                                    "priced_symbol_id": "eth.native.ethereum-mainnet",
                                    "reader": {
                                        "kind": "fixed_unit_price",
                                        "unit_price_dec": "1800.00"
                                    }
                                }
                            ]
                        },
                        "decimals": 18,
                        "underlying_symbol_id": null,
                        "metadata": {}
                    }
                ],
                "metadata": {}
            },
            "valuation_source_registry": { "sources": [] }
        })
        .to_string()
    }

    fn sample_portfolio_config_toml() -> String {
        r#"[portfolio]
portfolio_id = "portfolio_main"
quote_codes = ["USD"]

[portfolio.metadata]

[[portfolio.networks]]
network_id = "ethereum-mainnet"
family = "evm"
chain_id = 1
control_scope = "shared"

[portfolio.networks.metadata]

[[portfolio.wallets]]
wallet_id = "wallet_main"
network_id = "ethereum-mainnet"
symbol_ids = ["eth.native.ethereum-mainnet"]

[portfolio.wallets.subject]
kind = "evm_address"
address = "0x000000000000000000000000000000000000dead"

[portfolio.wallets.implementation]
kind = "address_only"

[portfolio.wallets.metadata]

[[portfolio.symbol_configs]]
symbol_id = "eth.native.ethereum-mainnet"
display_symbol = "ETH"
kind = "native_balance"
role = "native"
network_id = "ethereum-mainnet"
decimals = 18

[portfolio.symbol_configs.balance_reader]
kind = "native_balance"

[portfolio.symbol_configs.valuation]

[[portfolio.symbol_configs.valuation.quotes]]
quote = "USD"
priced_symbol_id = "eth.native.ethereum-mainnet"

[portfolio.symbol_configs.valuation.quotes.reader]
kind = "fixed_unit_price"
unit_price_dec = "1800.00"

[portfolio.symbol_configs.metadata]

[valuation_source_registry]
sources = []
"#
        .to_owned()
    }

    fn evm_entry_point_configs() -> [(&'static str, serde_json::Value); 4] {
        [
            ("evm_contract_deploy", deploy_config_json()),
            ("evm_contract_configure", configure_entry_config_json()),
            ("evm_contract_validate", validate_entry_config_json()),
            ("evm_contract_lifecycle", lifecycle_config_json()),
        ]
    }

    fn content_digest_str(byte: u8) -> String {
        format!("content:sha256-jcs-v1:{}", format!("{byte:02x}").repeat(32))
    }

    fn context_json() -> serde_json::Value {
        serde_json::json!({
            "lifecycle_key": "cli-test-lifecycle",
            "network": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1,
                "chain_fingerprint": null,
                "finality_or_observation_policy": null,
            },
            "contract_profile": {
                "profile_id": "cli-test-contract",
                "artifact_digest": content_digest_str(0x20),
                "interface_digest": content_digest_str(0x21),
                "creation_bytecode_digest": null,
                "deployed_code_hash": null,
                "selector_event_policy_digest": null,
            },
        })
    }

    fn signer_json() -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn deploy_action_json() -> serde_json::Value {
        serde_json::json!({
            "signer": signer_json(),
        })
    }

    fn configure_action_json() -> serde_json::Value {
        serde_json::json!({
            "signer": signer_json(),
            "calls": [],
        })
    }

    fn validate_action_json() -> serde_json::Value {
        serde_json::json!({})
    }

    fn deploy_config_json() -> serde_json::Value {
        serde_json::json!({
            "context": context_json(),
            "deploy": deploy_action_json(),
        })
    }

    fn lifecycle_config_json() -> serde_json::Value {
        serde_json::json!({
            "context": context_json(),
            "deploy": deploy_action_json(),
            "configure": configure_action_json(),
            "validate": validate_action_json(),
        })
    }

    fn import_deployed_json() -> serde_json::Value {
        serde_json::json!({
            "kind": "adopt_external_address",
            "adoption": {
                "address": "0x000000000000000000000000000000000000dead",
                "provenance_label": "cli-test-external",
                "evidence_policy": {
                    "require_code": false
                }
            },
        })
    }

    fn import_configured_json() -> serde_json::Value {
        serde_json::json!({
            "kind": "adopt_external_address",
            "adoption": {
                "address": "0x000000000000000000000000000000000000dead",
                "provenance_label": "cli-test-external",
                "evidence_policy": {
                    "require_code": false,
                    "allow_external_claimed_configured": true
                }
            },
        })
    }

    fn configure_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "context": context_json(),
            "import_deployed": import_deployed_json(),
            "configure": configure_action_json(),
        })
    }

    fn validate_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "context": context_json(),
            "import_configured": import_configured_json(),
            "validate": validate_action_json(),
        })
    }

    fn old_network_json() -> serde_json::Value {
        serde_json::json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
        })
    }

    fn old_deployed_contract_json() -> serde_json::Value {
        serde_json::json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "contract_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn old_configured_contract_json() -> serde_json::Value {
        serde_json::json!({
            "deployed": old_deployed_contract_json(),
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "contract_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn old_configure_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "config": {
                "network": old_network_json(),
                "signer": signer_json(),
                "calls": [],
            },
            "deployed": old_deployed_contract_json(),
        })
    }

    fn old_validate_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "config": {
                "network": old_network_json(),
                "read_assertions": [],
                "event_assertions": [],
            },
            "configured": old_configured_contract_json(),
        })
    }
}
