use std::fmt;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use mfm_app::{
    RunLaunchConfigArtifact, RunLaunchSeedArtifact, TypedPublicOutputResponse, TypedRunPhase,
    TypedRunResponse,
};
use mfm_op_evm_deploy_configure_validate::{
    canonicalize_deploy_configure_validate_authored_config, compile_dcv_configure_program,
    compile_dcv_deploy_program, compile_dcv_program, compile_dcv_validate_program,
    parse_deploy_configure_validate_authored_config,
    parse_deploy_configure_validate_authored_config_with_hint, AuthoredConfigFormat,
    CompiledDcvProgram, ConfiguredContract, DcvConfigArtifact, DcvSeedArtifact,
    DeployConfigureValidateCanonicalConfig, DeployConfigureValidateConfigureConfig,
    DeployConfigureValidateDeployConfig, DeployConfigureValidateValidateConfig, DeployedContract,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    command_error_from_app_error, connect_run_services, drive_mode, TypedDriveArg,
    TypedRunStoresArgs,
};

/// Subcommands under `mfm evm dcv`.
#[derive(Subcommand)]
pub(crate) enum DcvCommand {
    /// Deploy a contract and emit a DeployedContract typestate value.
    Deploy {
        /// Parsed arguments for deploy.
        #[command(flatten)]
        args: DeployArgs,
    },
    /// Configure a deployed contract and emit a ConfiguredContract typestate value.
    Configure {
        /// Parsed arguments for configure.
        #[command(flatten)]
        args: ConfigureArgs,
    },
    /// Validate a configured contract.
    Validate {
        /// Parsed arguments for validate.
        #[command(flatten)]
        args: ValidateArgs,
    },
    /// Run deploy, configure, and validate as one composed workflow.
    DeployConfigureValidate {
        /// Parsed arguments for the composed workflow.
        #[command(flatten)]
        args: FullArgs,
    },
}

impl DcvCommand {
    /// Dispatches the selected EVM DCV subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            DcvCommand::Deploy { args } => execute_deploy(ctx, args).await,
            DcvCommand::Configure { args } => execute_configure(ctx, args).await,
            DcvCommand::Validate { args } => execute_validate(ctx, args).await,
            DcvCommand::DeployConfigureValidate { args } => execute_full(ctx, args).await,
        }
    }
}

/// Arguments shared by EVM DCV commands that parse a request object.
#[derive(Args)]
pub(crate) struct DcvRequestArgs {
    /// Request JSON payload.
    #[arg(long)]
    request_json: Option<String>,

    /// Path to a request JSON or TOML file.
    #[arg(long)]
    request_file: Option<PathBuf>,
}

/// Arguments for `mfm evm dcv deploy`.
#[derive(Args)]
pub(crate) struct DeployArgs {
    /// Request source.
    #[command(flatten)]
    request: DcvRequestArgs,

    /// Typed storage configuration.
    #[command(flatten)]
    stores: TypedRunStoresArgs,

    /// Framework version evidence recorded in RunStarted.
    #[arg(long, default_value = "mfm.cli.evm_dcv.typed.v1")]
    framework_version: String,

    /// Source revision evidence recorded in RunStarted.
    #[arg(long, env = "MFM_SOURCE_REVISION", default_value = "unknown")]
    source_revision: String,

    /// Scheduler drive policy after RunStarted.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    drive: TypedDriveArg,
}

/// Arguments for `mfm evm dcv configure`.
#[derive(Args)]
pub(crate) struct ConfigureArgs {
    /// Request source.
    #[command(flatten)]
    request: DcvRequestArgs,

    /// JSON file containing the DeployedContract typestate value.
    #[arg(long)]
    deployed_contract_file: PathBuf,

    /// Typed storage configuration.
    #[command(flatten)]
    stores: TypedRunStoresArgs,

    /// Framework version evidence recorded in RunStarted.
    #[arg(long, default_value = "mfm.cli.evm_dcv.typed.v1")]
    framework_version: String,

    /// Source revision evidence recorded in RunStarted.
    #[arg(long, env = "MFM_SOURCE_REVISION", default_value = "unknown")]
    source_revision: String,

    /// Scheduler drive policy after RunStarted.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    drive: TypedDriveArg,
}

/// Arguments for `mfm evm dcv validate`.
#[derive(Args)]
pub(crate) struct ValidateArgs {
    /// Request source.
    #[command(flatten)]
    request: DcvRequestArgs,

    /// JSON file containing the ConfiguredContract typestate value.
    #[arg(long)]
    configured_contract_file: PathBuf,

    /// Typed storage configuration.
    #[command(flatten)]
    stores: TypedRunStoresArgs,

    /// Framework version evidence recorded in RunStarted.
    #[arg(long, default_value = "mfm.cli.evm_dcv.typed.v1")]
    framework_version: String,

    /// Source revision evidence recorded in RunStarted.
    #[arg(long, env = "MFM_SOURCE_REVISION", default_value = "unknown")]
    source_revision: String,

    /// Scheduler drive policy after RunStarted.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    drive: TypedDriveArg,
}

/// Arguments for `mfm evm dcv deploy-configure-validate`.
#[derive(Args)]
pub(crate) struct FullArgs {
    /// Request source.
    #[command(flatten)]
    request: DcvRequestArgs,

    /// Typed storage configuration.
    #[command(flatten)]
    stores: TypedRunStoresArgs,

    /// Framework version evidence recorded in RunStarted.
    #[arg(long, default_value = "mfm.cli.evm_dcv.typed.v1")]
    framework_version: String,

    /// Source revision evidence recorded in RunStarted.
    #[arg(long, env = "MFM_SOURCE_REVISION", default_value = "unknown")]
    source_revision: String,

    /// Scheduler drive policy after RunStarted.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    drive: TypedDriveArg,
}

#[derive(Debug, Serialize)]
struct EvmDcvResponse {
    run: TypedRunResponse,
    public_schema_id: String,
    public_output: Option<TypedPublicOutputResponse>,
}

impl fmt::Display for EvmDcvResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.public_output {
            Some(public_output) => write!(f, "{public_output}"),
            None => write!(f, "{}", self.run),
        }
    }
}

async fn execute_deploy(ctx: &CommandContext, args: &DeployArgs) -> ! {
    let result = async {
        let config = parse_phase_request::<DeployConfigureValidateDeployConfig>(&args.request)?;
        let compiled = compile_dcv_deploy_program(config)
            .map_err(|error| CommandError::new("EvmDcvCompileInvalid", error.to_string()))?;
        launch_compiled(
            compiled,
            &args.stores,
            &args.framework_version,
            &args.source_revision,
            args.drive,
        )
        .await
    }
    .await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_configure(ctx: &CommandContext, args: &ConfigureArgs) -> ! {
    let result = async {
        let config = parse_phase_request::<DeployConfigureValidateConfigureConfig>(&args.request)?;
        let deployed = read_seed_file::<DeployedContract>(&args.deployed_contract_file)?;
        let compiled = compile_dcv_configure_program(config, deployed)
            .map_err(|error| CommandError::new("EvmDcvCompileInvalid", error.to_string()))?;
        launch_compiled(
            compiled,
            &args.stores,
            &args.framework_version,
            &args.source_revision,
            args.drive,
        )
        .await
    }
    .await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_validate(ctx: &CommandContext, args: &ValidateArgs) -> ! {
    let result = async {
        let config = parse_phase_request::<DeployConfigureValidateValidateConfig>(&args.request)?;
        let configured = read_seed_file::<ConfiguredContract>(&args.configured_contract_file)?;
        let compiled = compile_dcv_validate_program(config, configured)
            .map_err(|error| CommandError::new("EvmDcvCompileInvalid", error.to_string()))?;
        launch_compiled(
            compiled,
            &args.stores,
            &args.framework_version,
            &args.source_revision,
            args.drive,
        )
        .await
    }
    .await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_full(ctx: &CommandContext, args: &FullArgs) -> ! {
    let result = async {
        let config = parse_full_request(&args.request)?;
        let compiled = compile_dcv_program(config)
            .map_err(|error| CommandError::new("EvmDcvCompileInvalid", error.to_string()))?;
        launch_compiled(
            compiled,
            &args.stores,
            &args.framework_version,
            &args.source_revision,
            args.drive,
        )
        .await
    }
    .await;
    handle_command_result(result, &ctx.output_format);
}

async fn launch_compiled(
    compiled: CompiledDcvProgram,
    stores: &TypedRunStoresArgs,
    framework_version: &str,
    source_revision: &str,
    drive: TypedDriveArg,
) -> CommandResult<EvmDcvResponse> {
    let public_schema_id = compiled.public_schema_id.clone();
    let services = connect_run_services(stores).await?;
    let run_id = mfm_app::new_run_id();
    let request = mfm_app::prepare_certified_run_launch(
        mfm_app::CertifiedRunLaunchInput {
            certified_spec: compiled.certified_spec,
            registry: services.certification_registry(),
            run_id: run_id.clone(),
            framework_version,
            source_revision,
            drive: drive_mode(drive),
        },
        run_launch_config_artifacts(compiled.config_artifacts),
        run_launch_seed_artifacts(compiled.seed_artifacts),
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
    Ok(CommandOutput::new(EvmDcvResponse {
        run,
        public_schema_id: public_schema_id.as_str().to_owned(),
        public_output,
    }))
}

fn parse_phase_request<T>(args: &DcvRequestArgs) -> Result<T, CommandError>
where
    T: DeserializeOwned,
{
    let raw = read_request_source(args)?;
    if let Some(path) = &args.request_file {
        if path.extension().and_then(|value| value.to_str()) == Some("toml") {
            return toml::from_str(&raw)
                .map_err(|error| CommandError::new("InvalidToml", error.to_string()));
        }
    }
    serde_json::from_str(&raw).map_err(|error| CommandError::new("InvalidJson", error.to_string()))
}

fn parse_full_request(
    args: &DcvRequestArgs,
) -> Result<DeployConfigureValidateCanonicalConfig, CommandError> {
    let raw = read_request_source(args)?;
    let authored = match &args.request_file {
        Some(path) => parse_deploy_configure_validate_authored_config_with_hint(&raw, Some(path)),
        None => parse_deploy_configure_validate_authored_config(&raw, AuthoredConfigFormat::Json),
    }
    .map_err(|error| CommandError::new("InvalidEvmDcvRequest", error.to_string()))?;
    canonicalize_deploy_configure_validate_authored_config(authored)
        .map_err(|error| CommandError::new("InvalidEvmDcvRequest", error.to_string()))
}

fn read_request_source(args: &DcvRequestArgs) -> Result<String, CommandError> {
    match (&args.request_json, &args.request_file) {
        (Some(_), Some(_)) => Err(CommandError::new(
            "InvalidArguments",
            "Pass only one of --request-json or --request-file",
        )),
        (None, None) => Err(CommandError::new(
            "MissingArgument",
            "Pass one of --request-json or --request-file",
        )),
        (Some(raw), None) => Ok(raw.clone()),
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|error| {
            CommandError::new(
                "InvalidRequestFile",
                format!("Failed to read {}: {error}", path.display()),
            )
        }),
    }
}

fn read_seed_file<T>(path: &PathBuf) -> Result<T, CommandError>
where
    T: DeserializeOwned,
{
    let raw = std::fs::read_to_string(path).map_err(|error| {
        CommandError::new(
            "InvalidSeedFile",
            format!("Failed to read {}: {error}", path.display()),
        )
    })?;
    serde_json::from_str(&raw).map_err(|error| {
        CommandError::new(
            "InvalidSeedFile",
            format!("Failed to decode {}: {error}", path.display()),
        )
    })
}

fn run_launch_config_artifacts(configs: Vec<DcvConfigArtifact>) -> Vec<RunLaunchConfigArtifact> {
    configs
        .into_iter()
        .map(|config| RunLaunchConfigArtifact {
            schema_id: config.schema_id,
            bytes: config.bytes,
            media_type: config.media_type,
        })
        .collect()
}

fn run_launch_seed_artifacts(seeds: Vec<DcvSeedArtifact>) -> Vec<RunLaunchSeedArtifact> {
    seeds
        .into_iter()
        .map(|seed| RunLaunchSeedArtifact {
            seed_id: seed.seed_id,
            bytes: seed.bytes,
            media_type: seed.media_type,
        })
        .collect()
}
