use std::fmt;
use std::path::PathBuf;

use clap::Args;
use mfm_app::{
    RunLaunchConfigArtifact, TypedPublicOutputResponse, TypedRunPhase, TypedRunResponse,
};
use mfm_op_portfolio_tracker::{
    compile_portfolio_snapshot_program, PortfolioConfigArtifact, PortfolioWorkflowConfig,
};
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, parse_portfolio_snapshot_authored_config,
    parse_portfolio_snapshot_authored_config_with_hint, AuthoredConfigFormat,
    PortfolioSnapshotCanonicalConfig, PortfolioSnapshotConfigError,
};
use serde::Serialize;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    command_error_from_app_error, connect_run_services, drive_mode, TypedDriveArg,
    TypedRunStoresArgs,
};

/// Arguments for `mfm portfolio snapshot`.
#[derive(Args)]
pub(crate) struct SnapshotArgs {
    /// Canonical portfolio snapshot request JSON payload
    #[arg(long)]
    pub request_json: Option<String>,

    /// Path to a portfolio snapshot request JSON or TOML file
    #[arg(long)]
    pub request_file: Option<PathBuf>,

    /// Typed storage configuration reserved for the typed portfolio port.
    #[command(flatten)]
    pub stores: TypedRunStoresArgs,

    /// Framework version evidence recorded in RunStarted.
    #[arg(long, default_value = "mfm.cli.portfolio.typed.v1")]
    pub framework_version: String,

    /// Source revision evidence recorded in RunStarted.
    #[arg(long, env = "MFM_SOURCE_REVISION", default_value = "unknown")]
    pub source_revision: String,

    /// Scheduler drive policy after the typed RunStarted event is committed.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    pub drive: TypedDriveArg,
}

#[derive(Debug, Serialize)]
struct PortfolioSnapshotResponse {
    run: TypedRunResponse,
    public_output: Option<TypedPublicOutputResponse>,
}

impl fmt::Display for PortfolioSnapshotResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.public_output {
            Some(public_output) => write!(f, "{public_output}"),
            None => write!(f, "{}", self.run),
        }
    }
}

/// Executes the portfolio snapshot command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &SnapshotArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &SnapshotArgs) -> CommandResult<PortfolioSnapshotResponse> {
    let canonical = parse_request(args)?;
    let workflow_config = PortfolioWorkflowConfig::from(canonical);
    let compiled = compile_portfolio_snapshot_program(workflow_config)
        .map_err(|error| CommandError::new("PortfolioCompileInvalid", error.to_string()))?;
    let public_schema_id = compiled.public_schema_id.clone();

    let services = connect_run_services(&args.stores).await?;
    let config_inputs = run_launch_config_artifacts(compiled.config_artifacts);

    let run_id = mfm_app::new_run_id();
    let request = mfm_app::prepare_certified_run_launch(
        mfm_app::CertifiedRunLaunchInput {
            certified_spec: compiled.certified_spec,
            registry: services.certification_registry(),
            run_id: run_id.clone(),
            framework_version: &args.framework_version,
            source_revision: &args.source_revision,
            drive: drive_mode(args.drive),
        },
        config_inputs,
        Vec::new(),
    )
    .map_err(command_error_from_app_error)?;
    let run = services
        .launch_run(request)
        .await
        .map_err(command_error_from_app_error)?;
    let public_output = if run.phase == TypedRunPhase::Completed {
        Some(
            services
                .typed_public_output(&run_id, &public_schema_id)
                .await
                .map_err(command_error_from_app_error)?,
        )
    } else {
        None
    };
    Ok(CommandOutput::new(PortfolioSnapshotResponse {
        run,
        public_output,
    }))
}

fn parse_request(args: &SnapshotArgs) -> Result<PortfolioSnapshotCanonicalConfig, CommandError> {
    match (&args.request_json, &args.request_file) {
        (Some(_), Some(_)) => Err(CommandError::new(
            "InvalidArguments",
            "Pass only one of --request-json or --request-file",
        )),
        (None, None) => Err(CommandError::new(
            "MissingArgument",
            "Pass one of --request-json or --request-file",
        )),
        (Some(raw), None) => parse_and_canonicalize_json(raw),
        (None, Some(path)) => {
            let raw = std::fs::read_to_string(path).map_err(|_| {
                CommandError::new(
                    "InvalidRequestFile",
                    "Failed to read --request-file contents",
                )
            })?;
            let authored = parse_portfolio_snapshot_authored_config_with_hint(&raw, Some(path))
                .map_err(command_error_from_portfolio_snapshot_config_error)?;
            canonicalize_portfolio_snapshot_authored_config(authored)
                .map_err(command_error_from_portfolio_snapshot_config_error)
        }
    }
}

fn parse_and_canonicalize_json(
    raw: &str,
) -> Result<PortfolioSnapshotCanonicalConfig, CommandError> {
    let authored = parse_portfolio_snapshot_authored_config(raw, AuthoredConfigFormat::Json)
        .map_err(command_error_from_portfolio_snapshot_config_error)?;
    canonicalize_portfolio_snapshot_authored_config(authored)
        .map_err(command_error_from_portfolio_snapshot_config_error)
}

fn run_launch_config_artifacts(
    configs: Vec<PortfolioConfigArtifact>,
) -> Vec<RunLaunchConfigArtifact> {
    configs
        .into_iter()
        .map(|config| RunLaunchConfigArtifact {
            schema_id: config.schema_id,
            bytes: config.bytes,
            media_type: config.media_type,
        })
        .collect()
}

fn command_error_from_portfolio_snapshot_config_error(
    err: PortfolioSnapshotConfigError,
) -> CommandError {
    match err {
        PortfolioSnapshotConfigError::InvalidJson { .. } => {
            CommandError::new("InvalidJson", "Failed to parse request body as JSON")
        }
        PortfolioSnapshotConfigError::InvalidToml { .. } => {
            CommandError::new("InvalidToml", "Failed to parse request body as TOML")
        }
        PortfolioSnapshotConfigError::InvalidBundle(_)
        | PortfolioSnapshotConfigError::Decode { .. }
        | PortfolioSnapshotConfigError::Serialize { .. }
        | PortfolioSnapshotConfigError::CanonicalJson { .. } => {
            CommandError::new("InvalidRequest", err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::Builder;

    #[test]
    fn parse_request_accepts_valid_canonical_request() {
        let args = SnapshotArgs {
            request_json: Some(canonical_request_json().to_string()),
            request_file: None,
            stores: TypedRunStoresArgs {
                typed_artifact_root: None,
                database_url: None,
            },
            framework_version: "mfm.cli.portfolio.typed.v1".to_owned(),
            source_revision: "unknown".to_owned(),
            drive: TypedDriveArg::UntilBlocked,
        };

        let parsed = parse_request(&args).expect("valid request");

        assert_eq!(parsed.portfolio.portfolio_id, "portfolio_main");
    }

    #[test]
    fn parse_request_preserves_json_parse_error_contract() {
        let args = SnapshotArgs {
            request_json: Some("{".to_string()),
            request_file: None,
            stores: TypedRunStoresArgs {
                typed_artifact_root: None,
                database_url: None,
            },
            framework_version: "mfm.cli.portfolio.typed.v1".to_owned(),
            source_revision: "unknown".to_owned(),
            drive: TypedDriveArg::UntilBlocked,
        };

        let err = parse_request(&args).expect_err("invalid json should fail");

        assert_eq!(err.code, "InvalidJson");
        assert_eq!(err.message, "Failed to parse request body as JSON");
    }

    #[test]
    fn parse_request_preserves_toml_parse_error_contract() {
        let request_file = Builder::new().suffix(".toml").tempfile().expect("tempfile");
        std::fs::write(request_file.path(), "portfolio = [").expect("write invalid request file");

        let args = SnapshotArgs {
            request_json: None,
            request_file: Some(request_file.path().to_path_buf()),
            stores: TypedRunStoresArgs {
                typed_artifact_root: None,
                database_url: None,
            },
            framework_version: "mfm.cli.portfolio.typed.v1".to_owned(),
            source_revision: "unknown".to_owned(),
            drive: TypedDriveArg::UntilBlocked,
        };

        let err = parse_request(&args).expect_err("invalid toml should fail");

        assert_eq!(err.code, "InvalidToml");
        assert_eq!(err.message, "Failed to parse request body as TOML");
    }

    fn canonical_request_json() -> serde_json::Value {
        json!({
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
                        "address": "0x000000000000000000000000000000000000dead",
                        "implementation": {
                            "kind": "address_only"
                        },
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
                        "balance_reader": {
                            "kind": "native_balance"
                        },
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
            "valuation_source_registry": {
                "sources": []
            }
        })
    }
}
