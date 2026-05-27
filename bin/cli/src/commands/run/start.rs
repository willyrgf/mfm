use std::path::PathBuf;
use std::str::FromStr;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    command_error_from_typed_app_error, drive_mode, make_typed_app_services, parse_typed_run_id,
    TypedDriveArg, TypedRunStoresArgs,
};
use clap::Args;
use mfm_app::{TypedRunResponse, TypedSeedInput};
use mfm_ids::SeedId;

/// Arguments for `mfm run start`.
#[derive(Args)]
pub(crate) struct StartArgs {
    /// Certified typed spec bundle JSON file.
    #[arg(long, value_name = "PATH")]
    pub bundle: PathBuf,

    /// Optional typed run id (`run:<algorithm>:<digest>`). Defaults to a generated typed id.
    #[arg(long)]
    pub run_id: Option<String>,

    /// Seed input as `seed:<algorithm>:<digest>=/path/to/canonical-seed.json`.
    #[arg(long = "seed", value_name = "SEED_ID=PATH")]
    pub seeds: Vec<SeedInputArg>,

    /// Framework version evidence recorded in RunStarted.
    #[arg(long, default_value = "mfm.cli.typed.v1")]
    pub framework_version: String,

    /// Source revision evidence recorded in RunStarted.
    #[arg(long, env = "MFM_SOURCE_REVISION", default_value = "unknown")]
    pub source_revision: String,

    /// Scheduler drive policy after the typed RunStarted event is committed.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    pub drive: TypedDriveArg,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: TypedRunStoresArgs,
}

#[derive(Clone, Debug)]
pub(crate) struct SeedInputArg {
    seed_id: SeedId,
    path: PathBuf,
}

impl FromStr for SeedInputArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (seed_id, path) = value
            .split_once('=')
            .ok_or_else(|| "seed input must use SEED_ID=PATH".to_owned())?;
        Ok(Self {
            seed_id: SeedId::parse(seed_id)
                .map_err(|_| "seed id must use the typed seed identity format".to_owned())?,
            path: PathBuf::from(path),
        })
    }
}

/// Executes the start command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &StartArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StartArgs) -> CommandResult<TypedRunResponse> {
    let run_id = match &args.run_id {
        Some(run_id) => parse_typed_run_id(run_id)?,
        None => mfm_app::new_run_id(),
    };
    let bundle_bytes = tokio::fs::read(&args.bundle).await.map_err(|error| {
        CommandError::new(
            "TypedBundleReadFailed",
            format!(
                "failed to read certified typed spec bundle file {}: {error}",
                args.bundle.display()
            ),
        )
    })?;
    let bundle = mfm_app::parse_certified_spec_bundle_json_bytes(&bundle_bytes)
        .map_err(command_error_from_typed_app_error)?;
    let seed_media_type = mfm_app::json_media_type().map_err(command_error_from_typed_app_error)?;
    let mut seed_inputs = Vec::with_capacity(args.seeds.len());
    for seed in &args.seeds {
        let bytes = tokio::fs::read(&seed.path).await.map_err(|error| {
            CommandError::new(
                "TypedSeedReadFailed",
                format!(
                    "failed to read seed input {} for {}: {error}",
                    seed.path.display(),
                    seed.seed_id
                ),
            )
        })?;
        seed_inputs.push(TypedSeedInput {
            seed_id: seed.seed_id.clone(),
            bytes,
            media_type: seed_media_type.clone(),
        });
    }

    let services = make_typed_app_services(&args.stores).await?;
    let request = mfm_app::build_typed_run_start_request(
        services.artifacts(),
        mfm_app::CertifiedBundleRunStartInput {
            spec_bytes: bundle.spec_bytes(),
            certificate_bytes: bundle.certificate_bytes(),
            registry: services.certification_registry(),
            run_id,
            framework_version: &args.framework_version,
            source_revision: &args.source_revision,
            drive: drive_mode(args.drive),
        },
        seed_inputs,
    )
    .await
    .map_err(command_error_from_typed_app_error)?;
    let response = services
        .start_certified_run(request)
        .await
        .map_err(command_error_from_typed_app_error)?;
    Ok(CommandOutput::new(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn start_rejects_invalid_bundle_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bundle = tmp.path().join("bundle.json");
        std::fs::write(&bundle, "{}").expect("write invalid bundle");

        let err = execute_internal(&StartArgs {
            bundle,
            run_id: None,
            seeds: Vec::new(),
            framework_version: "mfm.cli.test".to_owned(),
            source_revision: "test-source".to_owned(),
            drive: TypedDriveArg::AppendOnly,
            stores: TypedRunStoresArgs {
                typed_artifact_root: Some(tmp.path().join("artifacts")),
                database_url: None,
            },
        })
        .await
        .expect_err("invalid bundle rejects before store construction");

        assert_eq!(err.code, "TypedBundleInvalid");
    }
}
