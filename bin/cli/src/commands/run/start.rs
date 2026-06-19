use std::path::PathBuf;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    command_error_from_app_error, connect_run_services, drive_mode, parse_typed_run_id,
    TypedDriveArg, TypedRunStoresArgs,
};
use clap::Args;
use mfm_app::{RunLaunchConfigArtifact, RunLaunchSeedArtifact, TypedRunResponse};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{SchemaId, SeedId};

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

    /// Config input as `schema:<name>:<version>:<algorithm>:<digest>=/path/to/config.json`.
    #[arg(long = "config", value_name = "SCHEMA_ID=PATH")]
    pub configs: Vec<ConfigInputArg>,

    /// Framework version evidence recorded in RunAdmitted.
    #[arg(long, default_value = "mfm.cli.typed.v1")]
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

#[derive(Clone, Debug)]
pub(crate) struct SeedInputArg {
    seed_id: SeedId,
    path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct ConfigInputArg {
    schema_id: SchemaId,
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

impl FromStr for ConfigInputArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (schema_id, path) = value
            .split_once('=')
            .ok_or_else(|| "config input must use SCHEMA_ID=PATH".to_owned())?;
        Ok(Self {
            schema_id: SchemaId::parse(schema_id)
                .map_err(|_| "schema id must use the typed schema identity format".to_owned())?,
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
    let bundle_bytes = tokio::fs::read(&args.bundle).await.map_err(|_| {
        CommandError::backend(
            "CertifiedBundleReadFailed",
            "Failed to read certified typed spec bundle file",
        )
    })?;
    let bundle = mfm_app::parse_certified_spec_bundle_json_bytes(&bundle_bytes)
        .map_err(command_error_from_app_error)?;
    let registry =
        mfm_app::production_certification_registry().map_err(command_error_from_app_error)?;
    let media_type = mfm_app::json_media_type().map_err(command_error_from_app_error)?;
    let mut config_inputs = Vec::with_capacity(args.configs.len());
    for config in &args.configs {
        let bytes = read_canonical_json_file(
            &config.path,
            "LaunchConfigReadFailed",
            "LaunchConfigInvalid",
        )
        .await?;
        config_inputs.push(RunLaunchConfigArtifact {
            schema_id: config.schema_id.clone(),
            bytes,
            media_type: media_type.clone(),
        });
    }
    let mut seed_inputs = Vec::with_capacity(args.seeds.len());
    for seed in &args.seeds {
        let bytes =
            read_canonical_json_file(&seed.path, "LaunchSeedReadFailed", "LaunchSeedInvalid")
                .await?;
        seed_inputs.push(RunLaunchSeedArtifact {
            seed_id: seed.seed_id.clone(),
            bytes,
            media_type: media_type.clone(),
        });
    }

    let request = mfm_app::prepare_verified_bundle_launch(
        mfm_app::UntrustedCertifiedBundleLaunchInput {
            spec_bytes: bundle.spec_bytes(),
            certificate_bytes: bundle.certificate_bytes(),
            registry: &registry,
            run_id,
            framework_version: &args.framework_version,
            source_revision: &args.source_revision,
            launched_at_unix_ms: launch_unix_ms()?,
            drive: drive_mode(args.drive),
        },
        config_inputs,
        seed_inputs,
    )
    .map_err(command_error_from_app_error)?;
    let services = connect_run_services(&args.stores).await?;
    let response = services
        .launch_run(request)
        .await
        .map_err(command_error_from_app_error)?;
    Ok(CommandOutput::new(response))
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

async fn read_canonical_json_file(
    path: &PathBuf,
    read_code: &'static str,
    parse_code: &'static str,
) -> Result<Vec<u8>, CommandError> {
    let read_message = match read_code {
        "LaunchConfigReadFailed" => "Failed to read launch config input file",
        "LaunchSeedReadFailed" => "Failed to read launch seed input file",
        _ => "Failed to read launch input file",
    };
    let parse_message = match parse_code {
        "LaunchConfigInvalid" => "Launch config input is not canonical JSON",
        "LaunchSeedInvalid" => "Launch seed input is not canonical JSON",
        _ => "Launch input is not canonical JSON",
    };
    let raw = tokio::fs::read_to_string(path)
        .await
        .map_err(|_| CommandError::backend(read_code, read_message))?;
    PlainCanonicalJsonBytes::from_json_str(&raw)
        .map(|canonical| canonical.to_vec())
        .map_err(|_| CommandError::backend(parse_code, parse_message))
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
            configs: Vec::new(),
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

        assert_eq!(err.code, "CertifiedBundleInvalid");
    }

    #[tokio::test]
    async fn start_rejects_missing_config_inputs_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (bundle, _configs) = proof_bundle_and_configs(tmp.path());

        let err = execute_internal(&StartArgs {
            bundle,
            run_id: None,
            seeds: Vec::new(),
            configs: Vec::new(),
            framework_version: "mfm.cli.test".to_owned(),
            source_revision: "test-source".to_owned(),
            drive: TypedDriveArg::AppendOnly,
            stores: TypedRunStoresArgs {
                typed_artifact_root: Some(tmp.path().join("artifacts")),
                database_url: None,
            },
        })
        .await
        .expect_err("missing config inputs reject before store construction");

        assert_eq!(err.code, "MissingLaunchConfigArtifact");
    }

    #[tokio::test]
    async fn start_accepts_config_inputs_before_store_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (bundle, configs) = proof_bundle_and_configs(tmp.path());

        let err = execute_internal(&StartArgs {
            bundle,
            run_id: None,
            seeds: Vec::new(),
            configs,
            framework_version: "mfm.cli.test".to_owned(),
            source_revision: "test-source".to_owned(),
            drive: TypedDriveArg::AppendOnly,
            stores: TypedRunStoresArgs {
                typed_artifact_root: Some(tmp.path().join("artifacts")),
                database_url: None,
            },
        })
        .await
        .expect_err("valid launch material proceeds to store construction");

        assert_eq!(err.code, "MissingDatabaseUrl");
    }

    fn proof_bundle_and_configs(root: &std::path::Path) -> (PathBuf, Vec<ConfigInputArg>) {
        let proof_config = mfm_op_proof::ProofWorkflowConfig::default();
        let draft = mfm_op_proof::proof_program_draft(proof_config.clone()).expect("proof draft");
        let certified = mfm_op_proof::certified_proof_spec(proof_config).expect("proof spec");
        let bundle = certified.bundle().expect("proof bundle");
        let bundle_json = serde_json::json!({
            "kind": "certified_typed_spec_bundle_v1",
            "spec": serde_json::from_slice::<serde_json::Value>(bundle.spec_bytes())
                .expect("spec JSON"),
            "certificate": serde_json::from_slice::<serde_json::Value>(bundle.certificate_bytes())
                .expect("certificate JSON"),
        });
        let bundle_path = root.join("bundle.json");
        std::fs::write(
            &bundle_path,
            serde_json::to_vec(&bundle_json).expect("bundle JSON"),
        )
        .expect("write bundle");

        let mut configs = Vec::new();
        let mut index = 0usize;
        for config in draft
            .state_nodes()
            .iter()
            .map(|node| &node.config)
            .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        {
            let path = root.join(format!("config-{index}.json"));
            index += 1;
            std::fs::write(&path, config.canonical_json.as_bytes()).expect("write config");
            configs.push(ConfigInputArg {
                schema_id: config.schema_id.clone(),
                path,
            });
        }
        for node in &certified.envelope().spec.nodes {
            let Some(framework) = &node.framework else {
                continue;
            };
            let bytes = mfm_spec::v1::framework_config_canonical_json(
                framework.config_kind(),
                &node.node_id,
            )
            .expect("framework config");
            let path = root.join(format!("config-{index}.json"));
            index += 1;
            std::fs::write(&path, bytes.as_bytes()).expect("write framework config");
            configs.push(ConfigInputArg {
                schema_id: node.config_ref.schema_id.clone(),
                path,
            });
        }

        (bundle_path, configs)
    }
}
