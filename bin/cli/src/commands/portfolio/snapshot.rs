use std::fmt;
use std::path::PathBuf;

use clap::Args;
use mfm_app_legacy::{
    canonicalize_portfolio_snapshot_input, AuthoredConfigInput, PortfolioSnapshotRequest,
};
use serde::Serialize;

use crate::commands::result::{CommandError, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::command_error_from_app_error;
use crate::support::typed_run::TypedRunStoresArgs;

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
}

#[derive(Debug, Serialize)]
struct PortfolioSnapshotDisabledResponse {
    feature_id: String,
}

impl fmt::Display for PortfolioSnapshotDisabledResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is disabled until the typed portfolio port lands",
            self.feature_id
        )
    }
}

/// Executes the portfolio snapshot command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &SnapshotArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &SnapshotArgs) -> CommandResult<PortfolioSnapshotDisabledResponse> {
    let _request = parse_request(args)?;
    Err(CommandError::new(
        "TypedPortfolioPortPending",
        "portfolio snapshot is temporarily disabled until the typed portfolio workflow port replaces the old dynamic run path",
    ))
}

fn parse_request(args: &SnapshotArgs) -> Result<PortfolioSnapshotRequest, CommandError> {
    match (&args.request_json, &args.request_file) {
        (Some(_), Some(_)) => Err(CommandError::new(
            "InvalidArguments",
            "Pass only one of --request-json or --request-file",
        )),
        (None, None) => Err(CommandError::new(
            "MissingArgument",
            "Pass one of --request-json or --request-file",
        )),
        (Some(raw), None) => {
            canonicalize_portfolio_snapshot_input(AuthoredConfigInput::json(raw.clone()))
                .map_err(command_error_from_app_error)
        }
        (None, Some(path)) => {
            let raw = std::fs::read_to_string(path).map_err(|_| {
                CommandError::new(
                    "InvalidRequestFile",
                    "Failed to read --request-file contents",
                )
            })?;
            canonicalize_portfolio_snapshot_input(AuthoredConfigInput::with_path_hint(raw, path))
                .map_err(command_error_from_app_error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::Builder;

    #[tokio::test]
    async fn execute_internal_rejects_dynamic_portfolio_execution_after_parsing() {
        let args = SnapshotArgs {
            request_json: Some(canonical_request_json().to_string()),
            request_file: None,
            stores: TypedRunStoresArgs {
                typed_artifact_root: None,
                database_url: None,
            },
        };

        let err = execute_internal(&args)
            .await
            .expect_err("dynamic portfolio execution must be disabled");

        assert_eq!(err.code, "TypedPortfolioPortPending");
    }

    #[tokio::test]
    async fn execute_internal_preserves_json_parse_error_contract() {
        let args = SnapshotArgs {
            request_json: Some("{".to_string()),
            request_file: None,
            stores: TypedRunStoresArgs {
                typed_artifact_root: None,
                database_url: None,
            },
        };

        let err = execute_internal(&args)
            .await
            .expect_err("invalid json should fail");

        assert_eq!(err.code, "InvalidJson");
        assert_eq!(err.message, "Failed to parse request body as JSON");
    }

    #[tokio::test]
    async fn execute_internal_preserves_toml_parse_error_contract() {
        let request_file = Builder::new().suffix(".toml").tempfile().expect("tempfile");
        std::fs::write(request_file.path(), "portfolio = [").expect("write invalid request file");

        let args = SnapshotArgs {
            request_json: None,
            request_file: Some(request_file.path().to_path_buf()),
            stores: TypedRunStoresArgs {
                typed_artifact_root: None,
                database_url: None,
            },
        };

        let err = execute_internal(&args)
            .await
            .expect_err("invalid toml should fail");

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
