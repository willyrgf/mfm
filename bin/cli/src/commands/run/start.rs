use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    command_error_from_app_error, connect_run_services, drive_mode, parse_typed_run_id,
    TypedDriveArg, TypedRunStoresArgs,
};
use clap::{Args, ValueEnum};
use mfm_app::{
    AuthoredConfig, ConfigFormat, EntryPointRunLaunchInput, PublicOpName,
    TypedPublicOutputResponse, TypedRunMode, TypedRunResponse,
};
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

    /// Optional typed run id (`run:<algorithm>:<digest>`). Defaults to a generated typed id.
    #[arg(long)]
    pub run_id: Option<String>,

    /// Framework version evidence recorded in RunAdmitted.
    #[arg(long, default_value = "mfm.cli.entry_point.v1")]
    pub framework_version: String,

    /// Source revision evidence recorded in RunAdmitted.
    #[arg(long, env = "MFM_SOURCE_REVISION", default_value = "unknown")]
    pub source_revision: String,

    /// Scheduler drive policy after the typed RunAdmitted event is committed.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    pub drive: TypedDriveArg,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: TypedRunStoresArgs,
}

/// CLI spelling for authored config formats.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ConfigFormatArg {
    /// TOML authored config.
    Toml,
    /// JSON authored config.
    Json,
}

impl From<ConfigFormatArg> for ConfigFormat {
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
    run: TypedRunResponse,
    public_output: Option<TypedPublicOutputResponse>,
}

impl std::fmt::Display for StartOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.public_output {
            Some(public_output) => write!(f, "{public_output}"),
            None => write!(f, "{}", self.run),
        }
    }
}

/// Executes the start command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &StartArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StartArgs) -> CommandResult<StartOutput> {
    let run_id = match &args.run_id {
        Some(run_id) => parse_typed_run_id(run_id)?,
        None => mfm_app::new_run_id(),
    };
    let public_op_name = PublicOpName::new(&args.op)?;
    let op_version = args.op_version.map(mfm_app::OpVersion::new).transpose()?;
    let config_bytes = tokio::fs::read(&args.config)
        .await
        .map_err(|_| CommandError::backend("AuthoredConfigReadFailed", "Failed to read config"))?;
    let authored_config = AuthoredConfig::new(args.config_format.into(), config_bytes)?;
    let entry_point_registry =
        mfm_app::production_entry_point_op_registry().map_err(command_error_from_app_error)?;
    let certification_registry =
        mfm_app::production_certification_registry().map_err(command_error_from_app_error)?;
    let prepared = mfm_app::prepare_entry_point_run_launch(EntryPointRunLaunchInput {
        entry_point_registry: &entry_point_registry,
        public_op_name,
        op_version,
        authored_config,
        certification_registry: &certification_registry,
        run_id: run_id.clone(),
        framework_version: &args.framework_version,
        source_revision: &args.source_revision,
        launched_at_unix_ms: launch_unix_ms()?,
        drive: drive_mode(args.drive),
    })
    .map_err(command_error_from_app_error)?;
    let public_output_schema_id = prepared.public_output_schema_id.clone();
    let services = connect_run_services(&args.stores).await?;
    let run = services
        .launch_run(prepared.request)
        .await
        .map_err(command_error_from_app_error)?;
    let public_output = if run.run_mode == TypedRunMode::Completed {
        match public_output_schema_id {
            Some(schema_id) => Some(
                services
                    .typed_public_output(&run_id, &schema_id)
                    .await
                    .map_err(command_error_from_app_error)?,
            ),
            None => None,
        }
    } else {
        None
    };
    Ok(CommandOutput::new(StartOutput { run, public_output }))
}

fn launch_unix_ms() -> Result<u64, CommandError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            CommandError::backend("LaunchClockUnavailable", "System clock is unavailable")
        })?
        .as_millis();
    u64::try_from(millis).map_err(|_| {
        CommandError::new(
            "LaunchClockOverflow",
            "current Unix timestamp in milliseconds does not fit in u64",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn start_rejects_unknown_op_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.json");
        std::fs::write(&config, "{}").expect("write config");

        let mut args = start_args(
            config,
            TypedRunStoresArgs {
                typed_artifact_root: Some(tmp.path().join("artifacts")),
                database_url: None,
            },
        );
        args.op = "unknown_op".to_owned();

        let err = execute_internal(&args)
            .await
            .expect_err("unknown op rejects before store construction");

        assert_eq!(err.code, "EntryPointOpNotFound");
    }

    #[tokio::test]
    async fn start_rejects_invalid_config_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.json");
        std::fs::write(&config, r#"{"portfolio":{"portfolio_id":1}}"#).expect("write config");

        let err = execute_internal(&start_args(
            config,
            TypedRunStoresArgs {
                typed_artifact_root: Some(tmp.path().join("artifacts")),
                database_url: None,
            },
        ))
        .await
        .expect_err("invalid config rejects before store construction");

        assert_eq!(err.code, "AuthoredConfigDecodeFailed");
    }

    #[tokio::test]
    async fn start_accepts_entry_point_material_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.json");
        std::fs::write(&config, sample_portfolio_config_json()).expect("write config");

        let err = execute_internal(&start_args(
            config,
            TypedRunStoresArgs {
                typed_artifact_root: Some(tmp.path().join("artifacts")),
                database_url: None,
            },
        ))
        .await
        .expect_err("valid entry-point launch proceeds to store construction");

        assert_eq!(err.code, "MissingDatabaseUrl");
    }

    #[tokio::test]
    async fn start_defaults_config_format_to_toml_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("portfolio.toml");
        std::fs::write(&config, sample_portfolio_config_toml()).expect("write config");

        let mut args = start_args(
            config,
            TypedRunStoresArgs {
                typed_artifact_root: Some(tmp.path().join("artifacts")),
                database_url: None,
            },
        );
        args.config_format = ConfigFormatArg::Toml;

        let err = execute_internal(&args)
            .await
            .expect_err("valid TOML entry-point launch proceeds to store construction");

        assert_eq!(err.code, "MissingDatabaseUrl");
    }

    #[tokio::test]
    async fn start_accepts_evm_entry_points_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");

        for (index, (op, config)) in evm_entry_point_configs().into_iter().enumerate() {
            let config_path = tmp.path().join(format!("{op}-{index}.json"));
            std::fs::write(&config_path, config.to_string()).expect("write config");
            let mut args = start_args(
                config_path,
                TypedRunStoresArgs {
                    typed_artifact_root: Some(tmp.path().join(format!("artifacts-{index}"))),
                    database_url: None,
                },
            );
            args.op = op.to_owned();
            args.op_version = Some(1);
            args.config_format = ConfigFormatArg::Json;

            let err = execute_internal(&args)
                .await
                .expect_err("valid EVM entry-point launch proceeds to store construction");

            assert_eq!(err.code, "MissingDatabaseUrl", "{op}");
        }
    }

    fn start_args(config: PathBuf, stores: TypedRunStoresArgs) -> StartArgs {
        StartArgs {
            op: "portfolio_snapshot".to_owned(),
            config,
            op_version: None,
            config_format: ConfigFormatArg::Json,
            run_id: None,
            framework_version: "mfm.cli.test".to_owned(),
            source_revision: "test-source".to_owned(),
            drive: TypedDriveArg::AppendOnly,
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

    fn network_json() -> serde_json::Value {
        serde_json::json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
        })
    }

    fn signer_json() -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn deploy_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
            "signer": signer_json(),
        })
    }

    fn configure_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
            "signer": signer_json(),
            "calls": [],
        })
    }

    fn validate_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
        })
    }

    fn lifecycle_config_json() -> serde_json::Value {
        serde_json::json!({
            "deploy": deploy_config_json(),
            "configure": configure_config_json(),
            "validate": validate_config_json(),
        })
    }

    fn deployed_contract_json() -> serde_json::Value {
        serde_json::json!({
            "lifecycle_version": 1,
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "contract_address": "0x000000000000000000000000000000000000dead",
            "deploy_tx_hash": "0x01",
            "deploy_receipt_evidence": null,
            "deployed_block_number": 1,
        })
    }

    fn configured_contract_json() -> serde_json::Value {
        serde_json::json!({
            "lifecycle_version": 1,
            "deployed": deployed_contract_json(),
            "configure_calls": [],
            "confirmation_read_assertions": [],
            "confirmation_event_assertions": [],
            "configure_tx_hashes": [],
            "configure_receipt_evidence": [],
            "configured_block_number": 2,
        })
    }

    fn configure_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "config": configure_config_json(),
            "deployed": deployed_contract_json(),
        })
    }

    fn validate_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "config": validate_config_json(),
            "configured": configured_contract_json(),
        })
    }
}
