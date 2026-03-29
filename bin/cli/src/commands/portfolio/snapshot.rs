use std::future::Future;
use std::path::PathBuf;

use clap::Args;
use mfm_app::{
    AppError, FeatureExecutionResult, PortfolioSnapshotRequest, PortfolioSnapshotResponse,
};
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, parse_portfolio_snapshot_authored_config,
    parse_portfolio_snapshot_authored_config_with_hint, AuthoredConfigFormat,
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
    execute_request_with_starter(request, |request| async move {
        services.start_portfolio_snapshot(request).await
    })
    .await
}

#[cfg(test)]
async fn execute_with_starter<F, Fut>(
    args: &SnapshotArgs,
    start: F,
) -> CommandResult<FeatureExecutionResult>
where
    F: FnOnce(PortfolioSnapshotRequest) -> Fut,
    Fut: Future<Output = Result<PortfolioSnapshotResponse, AppError>>,
{
    let request = parse_request(args)?;
    execute_request_with_starter(request, start).await
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
            let authored =
                parse_portfolio_snapshot_authored_config(raw, AuthoredConfigFormat::Json)
                    .map_err(command_error_from_config_error)?;
            canonicalize_portfolio_snapshot_authored_config(authored)
                .map_err(command_error_from_config_error)
        }
        (None, Some(path)) => {
            let raw = std::fs::read_to_string(path).map_err(|_| {
                CommandError::new(
                    "InvalidRequestFile",
                    "Failed to read --request-file contents",
                )
            })?;
            let authored = parse_portfolio_snapshot_authored_config_with_hint(&raw, Some(path))
                .map_err(command_error_from_config_error)?;
            canonicalize_portfolio_snapshot_authored_config(authored)
                .map_err(command_error_from_config_error)
        }
    }
}

async fn execute_request_with_starter<F, Fut>(
    request: PortfolioSnapshotRequest,
    start: F,
) -> CommandResult<FeatureExecutionResult>
where
    F: FnOnce(PortfolioSnapshotRequest) -> Fut,
    Fut: Future<Output = Result<PortfolioSnapshotResponse, AppError>>,
{
    let response = start(request).await.map_err(command_error_from_app_error)?;
    let result = FeatureExecutionResult::from_serializable("portfolio.snapshot", response)
        .map_err(|_| CommandError::new("SerializationError", "Failed to serialize result"))?;
    Ok(CommandOutput::new(result))
}

fn command_error_from_config_error(
    err: mfm_portfolio_config::PortfolioSnapshotConfigError,
) -> CommandError {
    match err {
        mfm_portfolio_config::PortfolioSnapshotConfigError::InvalidJson { .. } => {
            CommandError::new("InvalidJson", "Failed to parse request body as JSON")
        }
        mfm_portfolio_config::PortfolioSnapshotConfigError::InvalidToml { .. } => {
            CommandError::new("InvalidToml", "Failed to parse request body as TOML")
        }
        mfm_portfolio_config::PortfolioSnapshotConfigError::InvalidBundle(_)
        | mfm_portfolio_config::PortfolioSnapshotConfigError::Serialize { .. }
        | mfm_portfolio_config::PortfolioSnapshotConfigError::CanonicalJson { .. } => {
            CommandError::new("InvalidRequest", err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_state_portfolio::model::{
        ExecutionAnchor, NetworkPin, PortfolioQuoteTotal, PortfolioReport, WalletReport,
    };
    use mfm_state_symbol::model::QuoteCode;
    use serde_json::json;

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

        let output = execute_with_starter(&args, |request| async move {
            assert_eq!(request.portfolio.portfolio_id, "portfolio_main");
            Ok(PortfolioSnapshotResponse {
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
