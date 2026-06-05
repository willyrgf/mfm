use std::fmt;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use mfm_app::{
    RunLaunchConfigArtifact, RunLaunchSeedArtifact, TypedPublicOutputResponse, TypedRunPhase,
    TypedRunResponse,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm_contract_config::{ConfigurePhaseConfig, DeployPhaseConfig, ValidatePhaseConfig};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
use mfm_op_evm_contract_lifecycle::{
    compile_contract_configure_program, compile_contract_deploy_program,
    compile_contract_lifecycle_program, compile_contract_validate_program,
    CompiledContractLifecycleProgram, ContractLifecycleCompileError, ContractLifecycleConfig,
    ContractLifecycleConfigArtifact,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    command_error_from_app_error, connect_run_services, drive_mode, TypedDriveArg,
    TypedRunStoresArgs,
};

/// Subcommands under `mfm evm contracts`.
#[derive(Subcommand)]
pub(crate) enum ContractsCommand {
    /// Deploy a contract and publish the deployed-contract output
    Deploy {
        /// Parsed arguments for contract deployment.
        #[command(flatten)]
        args: DeployArgs,
    },
    /// Configure an already deployed contract
    Configure {
        /// Parsed arguments for contract configuration.
        #[command(flatten)]
        args: ConfigureArgs,
    },
    /// Validate an already configured contract
    Validate {
        /// Parsed arguments for contract validation.
        #[command(flatten)]
        args: ValidateArgs,
    },
    /// Run deploy, configure, and validate as one lifecycle
    Lifecycle {
        /// Parsed arguments for a full contract lifecycle.
        #[command(flatten)]
        args: LifecycleArgs,
    },
}

impl ContractsCommand {
    /// Dispatches the selected contract lifecycle subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            ContractsCommand::Deploy { args } => execute_deploy(ctx, args).await,
            ContractsCommand::Configure { args } => execute_configure(ctx, args).await,
            ContractsCommand::Validate { args } => execute_validate(ctx, args).await,
            ContractsCommand::Lifecycle { args } => execute_lifecycle(ctx, args).await,
        }
    }
}

/// Arguments for `mfm evm contracts deploy`.
#[derive(Args)]
pub(crate) struct DeployArgs {
    /// Deploy phase config JSON file.
    #[arg(long = "config-file", value_name = "PATH")]
    pub config_file: PathBuf,

    /// Shared typed launch arguments.
    #[command(flatten)]
    pub launch: ContractLaunchArgs,
}

/// Arguments for `mfm evm contracts configure`.
#[derive(Args)]
pub(crate) struct ConfigureArgs {
    /// Configure phase config JSON file.
    #[arg(long = "config-file", value_name = "PATH")]
    pub config_file: PathBuf,

    /// Deployed-contract JSON value to use as the launch seed.
    #[arg(long = "deployed-file", value_name = "PATH")]
    pub deployed_file: PathBuf,

    /// Shared typed launch arguments.
    #[command(flatten)]
    pub launch: ContractLaunchArgs,
}

/// Arguments for `mfm evm contracts validate`.
#[derive(Args)]
pub(crate) struct ValidateArgs {
    /// Validate phase config JSON file.
    #[arg(long = "config-file", value_name = "PATH")]
    pub config_file: PathBuf,

    /// Configured-contract JSON value to use as the launch seed.
    #[arg(long = "configured-file", value_name = "PATH")]
    pub configured_file: PathBuf,

    /// Shared typed launch arguments.
    #[command(flatten)]
    pub launch: ContractLaunchArgs,
}

/// Arguments for `mfm evm contracts lifecycle`.
#[derive(Args)]
pub(crate) struct LifecycleArgs {
    /// Full contract lifecycle config JSON file.
    #[arg(long = "config-file", value_name = "PATH")]
    pub config_file: PathBuf,

    /// Shared typed launch arguments.
    #[command(flatten)]
    pub launch: ContractLaunchArgs,
}

/// Shared launch arguments for typed contract lifecycle commands.
#[derive(Args)]
pub(crate) struct ContractLaunchArgs {
    /// Typed storage configuration for certified run events and artifacts.
    #[command(flatten)]
    pub stores: TypedRunStoresArgs,

    /// Framework version evidence recorded in RunStarted.
    #[arg(long, default_value = "mfm.cli.evm_contracts.typed.v1")]
    pub framework_version: String,

    /// Source revision evidence recorded in RunStarted.
    #[arg(long, env = "MFM_SOURCE_REVISION", default_value = "unknown")]
    pub source_revision: String,

    /// Scheduler drive policy after the typed RunStarted event is committed.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    pub drive: TypedDriveArg,
}

#[derive(Debug, Serialize)]
struct EvmContractCommandResponse {
    run: TypedRunResponse,
    public_output: Option<TypedPublicOutputResponse>,
}

impl fmt::Display for EvmContractCommandResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.public_output {
            Some(public_output) => write!(f, "{public_output}"),
            None => write!(f, "{}", self.run),
        }
    }
}

async fn execute_deploy(ctx: &CommandContext, args: &DeployArgs) -> ! {
    let result = execute_deploy_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_configure(ctx: &CommandContext, args: &ConfigureArgs) -> ! {
    let result = execute_configure_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_validate(ctx: &CommandContext, args: &ValidateArgs) -> ! {
    let result = execute_validate_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_lifecycle(ctx: &CommandContext, args: &LifecycleArgs) -> ! {
    let result = execute_lifecycle_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_deploy_internal(args: &DeployArgs) -> CommandResult<EvmContractCommandResponse> {
    let config = read_json_file::<DeployPhaseConfig>(&args.config_file)?;
    let compiled = compile_contract_deploy_program(config).map_err(command_error_from_compile)?;
    launch_compiled_contract_program(compiled, Vec::new(), &args.launch).await
}

async fn execute_configure_internal(
    args: &ConfigureArgs,
) -> CommandResult<EvmContractCommandResponse> {
    let config = read_json_file::<ConfigurePhaseConfig>(&args.config_file)?;
    let deployed = read_json_file::<DeployedContract>(&args.deployed_file)?;
    let seed_bytes = canonical_value_bytes(&deployed)?;
    let compiled =
        compile_contract_configure_program(config, deployed).map_err(command_error_from_compile)?;
    let seed = seed_artifact_for_single_seed(&compiled, seed_bytes)?;
    launch_compiled_contract_program(compiled, vec![seed], &args.launch).await
}

async fn execute_validate_internal(
    args: &ValidateArgs,
) -> CommandResult<EvmContractCommandResponse> {
    let config = read_json_file::<ValidatePhaseConfig>(&args.config_file)?;
    let configured = read_json_file::<ConfiguredContract>(&args.configured_file)?;
    let seed_bytes = canonical_value_bytes(&configured)?;
    let compiled = compile_contract_validate_program(config, configured)
        .map_err(command_error_from_compile)?;
    let seed = seed_artifact_for_single_seed(&compiled, seed_bytes)?;
    launch_compiled_contract_program(compiled, vec![seed], &args.launch).await
}

async fn execute_lifecycle_internal(
    args: &LifecycleArgs,
) -> CommandResult<EvmContractCommandResponse> {
    let config = read_json_file::<ContractLifecycleConfig>(&args.config_file)?;
    let compiled =
        compile_contract_lifecycle_program(config).map_err(command_error_from_compile)?;
    launch_compiled_contract_program(compiled, Vec::new(), &args.launch).await
}

async fn launch_compiled_contract_program(
    compiled: CompiledContractLifecycleProgram,
    seed_inputs: Vec<RunLaunchSeedArtifact>,
    launch: &ContractLaunchArgs,
) -> CommandResult<EvmContractCommandResponse> {
    let public_schema_id = compiled.public_schema_id.clone();
    let services = connect_run_services(&launch.stores).await?;
    let run_id = mfm_app::new_run_id();
    let request = mfm_app::prepare_certified_run_launch(
        mfm_app::CertifiedRunLaunchInput {
            certified_spec: compiled.certified_spec,
            registry: services.certification_registry(),
            run_id: run_id.clone(),
            framework_version: &launch.framework_version,
            source_revision: &launch.source_revision,
            drive: drive_mode(launch.drive),
        },
        run_launch_config_artifacts(compiled.config_artifacts),
        seed_inputs,
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
    Ok(CommandOutput::new(EvmContractCommandResponse {
        run,
        public_output,
    }))
}

fn run_launch_config_artifacts(
    configs: Vec<ContractLifecycleConfigArtifact>,
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

fn seed_artifact_for_single_seed(
    compiled: &CompiledContractLifecycleProgram,
    bytes: Vec<u8>,
) -> Result<RunLaunchSeedArtifact, CommandError> {
    let seeds = &compiled.certified_spec.envelope().spec.seeds;
    let [seed] = seeds.as_slice() else {
        return Err(CommandError::new(
            "EvmContractCompileInvalid",
            format!(
                "expected exactly one contract launch seed, found {}",
                seeds.len()
            ),
        ));
    };
    Ok(RunLaunchSeedArtifact {
        seed_id: seed.seed_id.clone(),
        bytes,
        media_type: mfm_app::json_media_type().map_err(command_error_from_app_error)?,
    })
}

fn read_json_file<T>(path: &Path) -> Result<T, CommandError>
where
    T: DeserializeOwned,
{
    let raw = std::fs::read_to_string(path).map_err(|error| {
        CommandError::new(
            "InvalidEvmContractRequest",
            format!("failed to read {}: {error}", path.display()),
        )
    })?;
    serde_json::from_str(&raw).map_err(|error| {
        CommandError::new(
            "InvalidEvmContractRequest",
            format!("failed to parse {} as JSON: {error}", path.display()),
        )
    })
}

fn canonical_value_bytes<T>(value: &T) -> Result<Vec<u8>, CommandError>
where
    T: Serialize,
{
    let json = serde_json::to_string(value).map_err(|error| {
        CommandError::new(
            "InvalidEvmContractRequest",
            format!("failed to serialize contract launch seed: {error}"),
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.to_vec())
        .map_err(|error| {
            CommandError::new(
                "InvalidEvmContractRequest",
                format!("failed to canonicalize contract launch seed: {error}"),
            )
        })
}

fn command_error_from_compile(err: ContractLifecycleCompileError) -> CommandError {
    CommandError::new("EvmContractCompileInvalid", err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_seed_artifact_matches_certified_seed_digest() {
        let deployed = deployed_contract();
        let seed_bytes = canonical_value_bytes(&deployed).expect("canonical seed");
        let compiled =
            compile_contract_configure_program(configure_config(), deployed).expect("compiled");
        let seed =
            seed_artifact_for_single_seed(&compiled, seed_bytes).expect("launch seed artifact");
        let seed_spec = compiled
            .certified_spec
            .envelope()
            .spec
            .seeds
            .first()
            .expect("seed spec");
        let canonical = PlainCanonicalJsonBytes::from_json_str(
            std::str::from_utf8(&seed.bytes).expect("seed utf8"),
        )
        .expect("seed canonical");

        assert_eq!(seed.seed_id, seed_spec.seed_id);
        assert_eq!(
            Some(canonical.content_digest()),
            seed_spec.required_digest.clone()
        );
    }

    fn configure_config() -> ConfigurePhaseConfig {
        serde_json::from_value(serde_json::json!({
            "network": network_json(),
            "signer": signer_json(),
            "calls": [],
        }))
        .expect("configure config")
    }

    fn deployed_contract() -> DeployedContract {
        DeployedContract {
            lifecycle_version: 1,
            network_id: "ethereum-mainnet".to_owned(),
            expected_chain_id: 1,
            contract_address: "0x000000000000000000000000000000000000c0de".to_owned(),
            deploy_tx_hash: "0xabc123".to_owned(),
            deploy_receipt_evidence: None,
            deployed_block_number: Some(100),
        }
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
}
