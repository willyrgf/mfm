use std::future::Future;
use std::path::PathBuf;

use clap::Args;
use mfm_app::{
    canonicalize_portfolio_snapshot_input, AppError, AuthoredConfigInput, FeatureCatalog,
    FeatureExecutionResult, FeatureRequest, PortfolioSnapshotRequest,
};

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services_from_args};
use crate::support::run_stores::RunStoresArgs;

/// Arguments for `mfm portfolio snapshot`.
#[derive(Args)]
pub(crate) struct SnapshotArgs {
    /// Canonical portfolio snapshot request JSON payload
    #[arg(long)]
    pub request_json: Option<String>,

    /// Path to a portfolio snapshot request JSON or TOML file
    #[arg(long)]
    pub request_file: Option<PathBuf>,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Executes the portfolio snapshot command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &SnapshotArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &SnapshotArgs) -> CommandResult<FeatureExecutionResult> {
    let request = parse_request(args)?;
    let services = make_app_services_from_args(&args.stores).await?;
    let catalog = FeatureCatalog::with_builtins();
    execute_request_with_executor(request, |feature_request| async move {
        catalog.execute(&services, feature_request).await
    })
    .await
}

#[cfg(test)]
async fn execute_with_executor<F, Fut>(
    args: &SnapshotArgs,
    execute: F,
) -> CommandResult<FeatureExecutionResult>
where
    F: FnOnce(FeatureRequest) -> Fut,
    Fut: Future<Output = Result<FeatureExecutionResult, AppError>>,
{
    let request = parse_request(args)?;
    execute_request_with_executor(request, execute).await
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

async fn execute_request_with_executor<F, Fut>(
    request: PortfolioSnapshotRequest,
    execute: F,
) -> CommandResult<FeatureExecutionResult>
where
    F: FnOnce(FeatureRequest) -> Fut,
    Fut: Future<Output = Result<FeatureExecutionResult, AppError>>,
{
    let feature_request = FeatureRequest {
        feature_id: "portfolio.snapshot".to_string(),
        payload: serde_json::to_value(&request)
            .map_err(|_| CommandError::new("SerializationError", "Failed to serialize request"))?,
    };
    let result = execute(feature_request)
        .await
        .map_err(command_error_from_app_error)?;
    Ok(CommandOutput::new(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_app::{ErrorClass, PortfolioSnapshotResponse};
    use mfm_state_portfolio::model::{
        ExecutionAnchor, NetworkPin, PortfolioQuoteTotal, PortfolioReport, WalletReport,
    };
    use mfm_state_symbol::model::QuoteCode;
    use serde_json::json;
    use tempfile::Builder;

    #[tokio::test]
    async fn execute_with_starter_returns_success_payload_with_report_totals() {
        let args = SnapshotArgs {
            request_json: Some(canonical_request_json().to_string()),
            request_file: None,
            stores: RunStoresArgs {
                artifact_root: None,
                database_url: None,
            },
        };

        let output = execute_with_executor(&args, |feature_request| async move {
            assert_eq!(feature_request.feature_id, "portfolio.snapshot");
            let request: PortfolioSnapshotRequest =
                serde_json::from_value(feature_request.payload).expect("typed feature payload");
            assert_eq!(request.portfolio.portfolio_id, "portfolio_main");
            FeatureExecutionResult::from_serializable(
                "portfolio.snapshot",
                PortfolioSnapshotResponse {
                    run_id: "run_123".to_string(),
                    phase: "completed".to_string(),
                    final_snapshot_id: Some("snapshot_ctx_123".to_string()),
                    snapshot_artifact_id: Some("artifact_123".to_string()),
                    report: Some(PortfolioReport {
                        schema_version: 2,
                        portfolio_id: request.portfolio.portfolio_id,
                        generated_at_ms: 1234,
                        network_pins: vec![NetworkPin {
                            network_id: "ethereum-mainnet".to_string(),
                            anchor: ExecutionAnchor::Evm {
                                chain_id: 1,
                                block_number: 100,
                            },
                        }],
                        wallet_summaries: vec![WalletReport {
                            wallet_id: "wallet_main".to_string(),
                            network_id: "ethereum-mainnet".to_string(),
                            totals_by_quote: vec![PortfolioQuoteTotal {
                                quote: QuoteCode::Usd,
                                assets_value_dec: "12.50".to_string(),
                                collateral_value_dec: "0".to_string(),
                                debt_value_dec: "0".to_string(),
                                staked_value_dec: "0".to_string(),
                                net_value_dec: "12.50".to_string(),
                            }],
                        }],
                        totals_by_quote: vec![PortfolioQuoteTotal {
                            quote: QuoteCode::Usd,
                            assets_value_dec: "12.50".to_string(),
                            collateral_value_dec: "0".to_string(),
                            debt_value_dec: "0".to_string(),
                            staked_value_dec: "0".to_string(),
                            net_value_dec: "12.50".to_string(),
                        }],
                        error_count: 0,
                    }),
                },
            )
            .map_err(|err| {
                AppError::new(ErrorClass::Internal, "SerializationError", err.to_string())
            })
        })
        .await
        .expect("successful command output");

        assert_eq!(output.data.feature_id, "portfolio.snapshot");
        assert_eq!(output.data.result["phase"], "completed");
        assert_eq!(output.data.result["snapshot_artifact_id"], "artifact_123");
        assert_eq!(
            output.data.result["report"]["wallet_summaries"][0]["totals_by_quote"][0]["quote"],
            "USD"
        );
        assert_eq!(
            output.data.result["report"]["wallet_summaries"][0]["totals_by_quote"][0]
                ["assets_value_dec"],
            "12.50"
        );
        assert_eq!(
            output.data.result["report"]["totals_by_quote"][0]["net_value_dec"],
            "12.50"
        );
    }

    #[tokio::test]
    async fn execute_with_starter_preserves_json_parse_error_contract() {
        let args = SnapshotArgs {
            request_json: Some("{".to_string()),
            request_file: None,
            stores: RunStoresArgs {
                artifact_root: None,
                database_url: None,
            },
        };

        let err = execute_with_executor(&args, |_| async move {
            panic!("starter should not run when request parsing fails")
        })
        .await
        .expect_err("invalid json should fail");

        assert_eq!(err.code, "InvalidJson");
        assert_eq!(err.message, "Failed to parse request body as JSON");
    }

    #[tokio::test]
    async fn execute_with_starter_preserves_toml_parse_error_contract() {
        let request_file = Builder::new().suffix(".toml").tempfile().expect("tempfile");
        std::fs::write(request_file.path(), "portfolio = [").expect("write invalid request file");

        let args = SnapshotArgs {
            request_json: None,
            request_file: Some(request_file.path().to_path_buf()),
            stores: RunStoresArgs {
                artifact_root: None,
                database_url: None,
            },
        };

        let err = execute_with_executor(&args, |_| async move {
            panic!("starter should not run when request parsing fails")
        })
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
                        "chain_id": 1,
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
