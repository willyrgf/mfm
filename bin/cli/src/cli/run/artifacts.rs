use crate::cli::command_result::{CommandOutput, CommandResult};
use crate::cli::utils::output::handle_command_result;
use crate::cli::utils::run_stores::{make_artifact_store, RunStoresArgs};
use crate::cli::CommandContext;
use clap::{Args, Subcommand};
use mfm_machine::ids::ArtifactId;
use serde::Serialize;
use std::fmt;

use super::engine_bundle::command_error_from_storage_error;

#[derive(Subcommand)]
pub enum ArtifactsCommand {
    /// Fetch an artifact by id
    Get {
        #[command(flatten)]
        args: GetArgs,
    },
}

impl ArtifactsCommand {
    pub async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            ArtifactsCommand::Get { args } => execute_get(ctx, args).await,
        }
    }
}

#[derive(Args)]
pub struct GetArgs {
    /// Artifact id (SHA-256 lowercase hex, 64 chars)
    pub artifact_id: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactGetResponse {
    pub artifact_id: String,
    #[serde(flatten)]
    pub body: ArtifactBody,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "encoding", rename_all = "lowercase")]
pub enum ArtifactBody {
    Json { value: serde_json::Value },
    Hex { hex: String },
}

impl fmt::Display for ArtifactGetResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.body {
            ArtifactBody::Json { value } => {
                let s =
                    serde_json::to_string_pretty(value).unwrap_or_else(|_| "<invalid json>".into());
                write!(f, "{s}")
            }
            ArtifactBody::Hex { hex } => write!(f, "{hex}"),
        }
    }
}

async fn execute_get(ctx: &CommandContext, args: &GetArgs) -> ! {
    let result = execute_get_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_get_internal(args: &GetArgs) -> CommandResult<ArtifactGetResponse> {
    let store = make_artifact_store(args.stores.artifact_root.clone());
    let id = ArtifactId(args.artifact_id.clone());

    let bytes = store
        .get(&id)
        .await
        .map_err(command_error_from_storage_error)?;

    let body = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => ArtifactBody::Json { value },
        Err(_) => ArtifactBody::Hex {
            hex: hex::encode(bytes),
        },
    };

    Ok(CommandOutput::new(ArtifactGetResponse {
        artifact_id: id.0,
        body,
    }))
}
