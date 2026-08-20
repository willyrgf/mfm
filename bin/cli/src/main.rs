//! MFM Portfolio snapshot CLI.
//!
//! Three flat commands over one JSON configuration file: `init` provisions the run-history
//! schema, `snapshot` plans and progresses one run under a caller-supplied RunId, and `show`
//! reads that retained run back. The binary owns parsing and rendering only.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use mfm_app::{Application, ApplicationError, BoundCapabilitySet, ComposeError, ComposedRuntime};
use mfm_evm::{EvmEndpoint, EvmPhysicalTarget};
use mfm_evm_live::JsonRpcEvmProvider;
use mfm_ids::RunId;
use mfm_portfolio::{PortfolioConfig, PortfolioSnapshotSelector};
use mfm_runtime::{RunView, RunViewState, RuntimeError};
use mfm_storage_postgres::{
    provision_schemas, AdminPostgresLocator, PostgresLocatorError, PostgresStore, ProvisionError,
    RuntimePostgresLocator, StoreOpenError,
};

/// Standalone MFM Portfolio snapshot surface.
#[derive(Parser)]
#[command(name = "mfm", version, about = "MFM Portfolio snapshot CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Installs the run-history schema when absent and verifies an existing installation.
    Init {
        /// Path to the JSON configuration file.
        #[arg(long)]
        config: PathBuf,
        /// Environment variable naming the checked administrative store locator.
        #[arg(long = "admin-store-locator-env")]
        admin_store_locator_env: String,
    },
    /// Plans, admits, and progresses one Portfolio snapshot under the supplied RunId.
    Snapshot {
        /// Path to the JSON configuration file.
        #[arg(long)]
        config: PathBuf,
        /// Full `run:sha256-jcs-v1:<64 hex>` identity; the CLI never derives one.
        #[arg(long = "run-id")]
        run_id: String,
    },
    /// Reads one retained run without progressing it.
    Show {
        /// Path to the JSON configuration file.
        #[arg(long)]
        config: PathBuf,
        /// Full `run:sha256-jcs-v1:<64 hex>` identity of the retained run.
        #[arg(long = "run-id")]
        run_id: String,
    },
}

/// One reviewed redaction-safe failure class per stderr line.
///
/// Environment variable NAMES may appear; a URL value never does.
#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("configuration is invalid")]
    Configuration,
    #[error("environment variable {0} is not set")]
    Environment(String),
    #[error("{0}")]
    Locator(#[source] PostgresLocatorError),
    #[error("{0}")]
    Provision(#[source] ProvisionError),
    #[error("{0}")]
    Composition(#[source] ComposeError),
    #[error("run id is invalid")]
    RunId,
    #[error("{0}")]
    Store(#[source] StoreOpenError),
    #[error("evm provider transport could not be constructed")]
    Provider,
    #[error("configuration cannot be planned for the configured evm route")]
    Plan,
    #[error("runtime operation failed: {0}")]
    Runtime(#[source] RuntimeError),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CliConfig {
    portfolio: PortfolioConfig,
    selector: PortfolioSnapshotSelector,
    evm: EvmRouteConfig,
    store: StoreConfig,
}

/// The one EVM route this wave admits per configuration file.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EvmRouteConfig {
    chain_id: u64,
    endpoint_id: String,
    rpc_url_env: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreConfig {
    runtime_locator_env: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<ExitCode, CliError> {
    match Cli::parse().command {
        Command::Init {
            config,
            admin_store_locator_env,
        } => {
            let config = load_config(&config)?;
            let runtime =
                RuntimePostgresLocator::parse(environment(&config.store.runtime_locator_env)?)
                    .map_err(CliError::Locator)?;
            let admin = AdminPostgresLocator::parse(environment(&admin_store_locator_env)?)
                .map_err(CliError::Locator)?;
            provision_schemas(&admin, &runtime)
                .await
                .map_err(CliError::Provision)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Snapshot { config, run_id } => {
            let run_id = RunId::parse(run_id).map_err(|_| CliError::RunId)?;
            let config = load_config(&config)?;
            let (application, target) = compose(&config).await?;
            let view = application
                .start_portfolio(
                    run_id,
                    config.selector,
                    &config.portfolio,
                    std::slice::from_ref(&target),
                )
                .await
                .map_err(map_application_error)?;
            Ok(emit(&view))
        }
        Command::Show { config, run_id } => {
            let run_id = RunId::parse(run_id).map_err(|_| CliError::RunId)?;
            let config = load_config(&config)?;
            // A cold read still needs the fully bound assembly: association pre-resolves
            // every implementation and callback before the fold reads one frame.
            let (application, _) = compose(&config).await?;
            let view = application
                .read(&run_id)
                .await
                .map_err(map_application_error)?;
            Ok(emit(&view))
        }
    }
}

fn load_config(path: &Path) -> Result<CliConfig, CliError> {
    let text = std::fs::read_to_string(path).map_err(|_| CliError::Configuration)?;
    serde_json::from_str(&text).map_err(|_| CliError::Configuration)
}

fn environment(name: &str) -> Result<String, CliError> {
    if !environment_name_is_valid(name) {
        return Err(CliError::Configuration);
    }
    std::env::var(name).map_err(|_| CliError::Environment(name.to_owned()))
}

fn environment_name_is_valid(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && matches!(bytes[0], b'A'..=b'Z' | b'_')
        && !bytes[1..]
            .iter()
            .any(|byte| !matches!(byte, b'A'..=b'Z' | b'0'..=b'9' | b'_'))
}

/// Builds the one live composition both `snapshot` and `show` use.
async fn compose(config: &CliConfig) -> Result<(Application, EvmPhysicalTarget), CliError> {
    let store_locator =
        RuntimePostgresLocator::parse(environment(&config.store.runtime_locator_env)?)
            .map_err(CliError::Locator)?;
    let rpc_url = environment(&config.evm.rpc_url_env)?;
    let endpoint =
        EvmEndpoint::new(config.evm.endpoint_id.as_str()).map_err(|_| CliError::Configuration)?;
    let endpoint_ref = endpoint
        .endpoint_ref()
        .map_err(|_| CliError::Configuration)?;
    let target = EvmPhysicalTarget::new(config.evm.chain_id, endpoint_ref)
        .map_err(|_| CliError::Configuration)?;
    let store = PostgresStore::connect(&store_locator)
        .await
        .map_err(CliError::Store)?;
    let provider: Arc<dyn mfm_evm_live::EvmProvider> =
        Arc::new(JsonRpcEvmProvider::new(rpc_url).map_err(|_| CliError::Provider)?);
    let bindings = BoundCapabilitySet::new(vec![(config.evm.chain_id, endpoint, provider)])
        .map_err(CliError::Composition)?;
    let composed =
        ComposedRuntime::compose(Arc::new(store), bindings).map_err(CliError::Composition)?;
    Ok((Application::new(composed), target))
}

const fn map_application_error(error: ApplicationError) -> CliError {
    match error {
        ApplicationError::InvalidRequest => CliError::Configuration,
        ApplicationError::Internal => CliError::Plan,
        ApplicationError::Runtime(error) => CliError::Runtime(error),
    }
}

/// Writes the one shared view rendering and returns its exit code.
fn emit(view: &RunView) -> ExitCode {
    use std::io::Write;

    let mut rendered = format!(
        "run_id={}\nhead_sequence={}\nhead_digest={}\nstate={}\n",
        view.run_id(),
        view.head_sequence(),
        view.head_digest(),
        match view.state() {
            RunViewState::Runnable => "runnable",
            RunViewState::Succeeded(_) => "succeeded",
            RunViewState::Failed(_) => "failed",
        }
    )
    .into_bytes();
    if let RunViewState::Succeeded(value) | RunViewState::Failed(value) = view.state() {
        rendered.extend_from_slice(value.canonical_bytes());
        rendered.push(b'\n');
    }
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&rendered)
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return ExitCode::from(2);
    }
    match view.state() {
        RunViewState::Succeeded(_) => ExitCode::SUCCESS,
        RunViewState::Runnable | RunViewState::Failed(_) => ExitCode::from(1),
    }
}

#[cfg(test)]
mod tests {
    use super::environment_name_is_valid;

    #[test]
    fn environment_names_are_bounded_and_safe_to_render() {
        for accepted in ["A", "_", "MFM_STORE_1", &"A".repeat(64)] {
            assert!(environment_name_is_valid(accepted));
        }
        for rejected in [
            "",
            "lowercase",
            "1MFM",
            "MFM-STORE",
            "MFM STORE",
            "MFM_STORE=value",
            &"A".repeat(65),
        ] {
            assert!(!environment_name_is_valid(rejected));
        }
    }
}
