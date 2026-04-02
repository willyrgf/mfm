use clap::{Args, Subcommand};
use mfm_app::{
    DeployConfigureValidateSpec, PipelineStartRequest, RunStartResponse, RunsStartRequest,
};
use mfm_evm_deploy_configure_validate_config::{
    canonicalize_deploy_configure_validate_authored_config,
    parse_deploy_configure_validate_authored_config,
    parse_deploy_configure_validate_authored_config_with_hint, AuthoredConfigFormat,
    DeployConfigureValidateConfigError,
};
use mfm_sdk::pipeline::Pipeline;
use std::path::PathBuf;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services_from_args};
use crate::support::run_stores::RunStoresArgs;

/// Subcommands under `mfm run pipeline`.
#[derive(Subcommand)]
pub(crate) enum PipelineCommand {
    /// Start a run from an explicit pipeline JSON payload
    Start {
        /// Parsed arguments for the explicit pipeline start command.
        #[command(flatten)]
        args: PipelineStartArgs,
    },

    /// Start the standard deploy->configure->validate pipeline from a compact spec
    DeployConfigureValidate {
        /// Parsed arguments for the deploy-configure-validate helper command.
        #[command(flatten)]
        args: DeployConfigureValidateArgs,
    },
}

impl PipelineCommand {
    /// Dispatches the selected pipeline subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            PipelineCommand::Start { args } => {
                handle_command_result(execute_start_internal(args).await, &ctx.output_format)
            }
            PipelineCommand::DeployConfigureValidate { args } => {
                handle_command_result(execute_dcv_internal(args).await, &ctx.output_format)
            }
        }
    }
}

/// Arguments for `mfm run pipeline start`.
#[derive(Args)]
pub(crate) struct PipelineStartArgs {
    /// Pipeline JSON payload
    #[arg(long)]
    pub pipeline_json: String,

    /// Optional pipeline input JSON (default: {})
    #[arg(long, default_value = "{}")]
    pub input_json: String,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Arguments for `mfm run pipeline deploy-configure-validate`.
#[derive(Args)]
pub(crate) struct DeployConfigureValidateArgs {
    /// Spec JSON payload describing deploy/configure/validate op configs.
    #[arg(long)]
    pub spec_json: Option<String>,

    /// Path to a spec JSON or TOML file describing deploy/configure/validate op configs.
    #[arg(long)]
    pub spec_file: Option<PathBuf>,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

fn parse_pipeline_json(s: &str) -> Result<Pipeline, CommandError> {
    serde_json::from_str::<Pipeline>(s).map_err(|_| {
        CommandError::new(
            "InvalidJson",
            "Failed to parse --pipeline-json as a pipeline JSON payload",
        )
    })
}

fn parse_input_json(s: &str) -> Result<serde_json::Value, CommandError> {
    serde_json::from_str::<serde_json::Value>(s)
        .map_err(|_| CommandError::new("InvalidJson", "Failed to parse --input-json as JSON"))
}

async fn execute_start_internal(args: &PipelineStartArgs) -> CommandResult<RunStartResponse> {
    let pipeline = parse_pipeline_json(&args.pipeline_json)?;
    let input = parse_input_json(&args.input_json)?;

    let services = make_app_services_from_args(&args.stores).await?;

    let response = services
        .start_run(RunsStartRequest::Pipeline(PipelineStartRequest {
            pipeline,
            input,
            run_config: Some(mfm_app::default_run_config()),
        }))
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}

async fn execute_dcv_internal(
    args: &DeployConfigureValidateArgs,
) -> CommandResult<RunStartResponse> {
    let spec = parse_dcv_spec(args)?;
    let services = make_app_services_from_args(&args.stores).await?;
    let response = services
        .start_deploy_configure_validate(spec)
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}

fn parse_dcv_spec(
    args: &DeployConfigureValidateArgs,
) -> Result<DeployConfigureValidateSpec, CommandError> {
    match (&args.spec_json, &args.spec_file) {
        (Some(_), Some(_)) => Err(CommandError::new(
            "InvalidArguments",
            "Pass only one of --spec-json or --spec-file",
        )),
        (None, None) => Err(CommandError::new(
            "MissingArgument",
            "Pass one of --spec-json or --spec-file",
        )),
        (Some(raw), None) => {
            let authored =
                parse_deploy_configure_validate_authored_config(raw, AuthoredConfigFormat::Json)
                    .map_err(|err| command_error_from_dcv_config_error(err, Some("JSON")))?;
            canonicalize_deploy_configure_validate_authored_config(authored)
                .map_err(|err| command_error_from_dcv_config_error(err, None))
        }
        (None, Some(path)) => {
            let raw = std::fs::read_to_string(path).map_err(|_| {
                CommandError::new("InvalidSpecFile", "Failed to read --spec-file contents")
            })?;
            let authored =
                parse_deploy_configure_validate_authored_config_with_hint(&raw, Some(path))
                    .map_err(|err| command_error_from_dcv_config_error(err, None))?;
            canonicalize_deploy_configure_validate_authored_config(authored)
                .map_err(|err| command_error_from_dcv_config_error(err, None))
        }
    }
}

fn command_error_from_dcv_config_error(
    err: DeployConfigureValidateConfigError,
    format_name: Option<&str>,
) -> CommandError {
    match err {
        DeployConfigureValidateConfigError::InvalidJson { .. } => {
            if let Some(format_name) = format_name {
                CommandError::new(
                    "InvalidJson",
                    format!("Failed to parse deploy/configure/validate {format_name}"),
                )
            } else {
                CommandError::new("InvalidJson", "Failed to parse --spec-json as JSON")
            }
        }
        DeployConfigureValidateConfigError::InvalidToml { .. } => CommandError::new(
            "InvalidToml",
            "Failed to parse deploy/configure/validate TOML",
        ),
        DeployConfigureValidateConfigError::Serialize { .. }
        | DeployConfigureValidateConfigError::CanonicalJson { .. } => {
            CommandError::new("DeployConfigureValidateConfigError", err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::Builder;

    async fn execute_dcv_with_starter<F, Fut>(
        args: &DeployConfigureValidateArgs,
        start: F,
    ) -> CommandResult<RunStartResponse>
    where
        F: FnOnce(DeployConfigureValidateSpec) -> Fut,
        Fut: std::future::Future<Output = Result<RunStartResponse, mfm_app::AppError>>,
    {
        let spec = parse_dcv_spec(args)?;
        let response = start(spec).await.map_err(command_error_from_app_error)?;
        Ok(CommandOutput::new(response))
    }

    #[tokio::test]
    async fn execute_dcv_with_starter_parses_json_and_starts_root_op() {
        let args = DeployConfigureValidateArgs {
            spec_json: Some(
                serde_json::json!({
                    "deploy": {
                        "network_id": "ethereum-mainnet",
                        "from": "0x000000000000000000000000000000000000dead"
                    },
                    "configure": {
                        "network_id": "ethereum-mainnet",
                        "from": "0x000000000000000000000000000000000000dead",
                        "calls": []
                    },
                    "validate": {
                        "network_id": "ethereum-mainnet",
                        "expected_chain_id": 1
                    }
                })
                .to_string(),
            ),
            spec_file: None,
            stores: RunStoresArgs {
                artifact_root: None,
                database_url: None,
            },
        };

        let output = execute_dcv_with_starter(&args, |spec| async move {
            assert_eq!(spec.machine_id, "evm_deploy_configure_validate");
            assert_eq!(spec.pipeline_version, "v1");
            assert_eq!(spec.deploy.network_id, "ethereum-mainnet");
            Ok(RunStartResponse {
                run_id: "run_123".to_string(),
                phase: "completed".to_string(),
                final_snapshot_id: Some("snapshot_ctx_123".to_string()),
            })
        })
        .await
        .expect("successful dcv start");

        assert_eq!(output.data.run_id, "run_123");
        assert_eq!(output.data.phase, "completed");
        assert_eq!(
            output.data.final_snapshot_id.as_deref(),
            Some("snapshot_ctx_123")
        );
    }

    #[tokio::test]
    async fn execute_dcv_with_starter_preserves_json_parse_error_contract() {
        let args = DeployConfigureValidateArgs {
            spec_json: Some("{".to_string()),
            spec_file: None,
            stores: RunStoresArgs {
                artifact_root: None,
                database_url: None,
            },
        };

        let err = execute_dcv_with_starter(&args, |_| async move {
            panic!("starter should not run when spec parsing fails")
        })
        .await
        .expect_err("invalid json should fail");

        assert_eq!(err.code, "InvalidJson");
        assert_eq!(
            err.message,
            "Failed to parse deploy/configure/validate JSON"
        );
    }

    #[tokio::test]
    async fn execute_dcv_with_starter_preserves_toml_parse_error_contract() {
        let spec_file = Builder::new().suffix(".toml").tempfile().expect("tempfile");
        std::fs::write(spec_file.path(), "deploy = [").expect("write invalid spec file");

        let args = DeployConfigureValidateArgs {
            spec_json: None,
            spec_file: Some(spec_file.path().to_path_buf()),
            stores: RunStoresArgs {
                artifact_root: None,
                database_url: None,
            },
        };

        let err = execute_dcv_with_starter(&args, |_| async move {
            panic!("starter should not run when spec parsing fails")
        })
        .await
        .expect_err("invalid toml should fail");

        assert_eq!(err.code, "InvalidToml");
        assert_eq!(
            err.message,
            "Failed to parse deploy/configure/validate TOML"
        );
    }
}
