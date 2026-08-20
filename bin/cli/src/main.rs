//! MFM command-line rendering of the typed Application client surface.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use mfm_app::{
    derive_run_id, provision_store, Application, BindingList, ConfigDocument, ConfigDocumentError,
    ConfigPageRequest, ConfigSelection, Deployment, EntryPointList, EnvironmentName, RequestError,
    RunPageRequest, RunRecovery, RunRequestError, SerializableRunView, StartRunResult,
};
use mfm_catalog::{
    ConfigCursor, ConfigDigest, ConfigName, PageLimit, RunCursor, MAX_CONFIG_DOCUMENT_BYTES,
};
use mfm_ids::RunId;
use mfm_runtime::{RunView, RunViewState};
use serde::Serialize;
use tokio::io::AsyncReadExt;

#[derive(Parser)]
#[command(name = "mfm", version, about = "MFM client CLI")]
struct Cli {
    /// Override the conventional XDG/HOME deployment bootstrap path.
    #[arg(long, global = true)]
    deployment: Option<PathBuf>,
    /// Select human-oriented text or the stable JSON result model.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    output: OutputFormat,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Discover compiled entry-point document tags.
    EntryPoint {
        #[command(subcommand)]
        command: EntryPointCommand,
    },
    /// Manage the durable persistence installation.
    Store {
        #[command(subcommand)]
        command: StoreCommand,
    },
    /// Discover public capability bindings.
    Binding {
        #[command(subcommand)]
        command: BindingCommand,
    },
    /// Manage named durable configs.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Start, progress, read, or enumerate runs.
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
}

#[derive(Subcommand)]
enum EntryPointCommand {
    /// Lists every accepted config-document entry-point tag.
    List,
}

#[derive(Subcommand)]
enum StoreCommand {
    /// Installs and verifies the run-history and config-catalog schemas.
    Init {
        /// Environment variable naming the checked administrative store locator.
        #[arg(long = "admin-store-locator-env")]
        admin_store_locator_env: String,
    },
}

#[derive(Subcommand)]
enum BindingCommand {
    /// Lists public bindings derived from the live composition.
    List,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Imports one complete config under an absent name.
    Import {
        /// Durable catalog name.
        name: String,
        /// Config JSON path, or `-` for stdin.
        #[arg(long)]
        from: PathBuf,
    },
    /// Lists one ascending keyset page.
    List {
        /// Exclusive config cursor.
        #[arg(long)]
        cursor: Option<String>,
        /// Page size from 1 through 200.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Shows one config summary and exact canonical document.
    Show {
        /// Durable catalog name.
        name: String,
    },
    /// Deletes one exact named revision.
    Delete {
        /// Durable catalog name.
        name: String,
        /// Required canonical-document digest.
        #[arg(long)]
        digest: String,
    },
}

#[derive(Subcommand)]
enum RunCommand {
    /// Starts and progresses one run from a stored config.
    Start {
        /// Durable config name.
        #[arg(long)]
        config: String,
        /// Optional exact config revision assertion.
        #[arg(long = "config-digest")]
        config_digest: Option<String>,
        /// Optional explicit identity; omission uses OS cryptographic entropy.
        #[arg(long = "run-id")]
        run_id: Option<String>,
    },
    /// Progresses one retained run.
    Progress {
        /// Exact retained run identity.
        #[arg(long = "run-id")]
        run_id: String,
    },
    /// Reads one retained run without progression.
    Show {
        /// Exact retained run identity.
        #[arg(long = "run-id")]
        run_id: String,
    },
    /// Lists one ascending mechanical run-head page.
    List {
        /// Exclusive run cursor.
        #[arg(long)]
        cursor: Option<String>,
        /// Page size from 1 through 200.
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[derive(Debug)]
enum CliError {
    Composition(mfm_app::ComposeError),
    ConfigName,
    ConfigDigest,
    RunId,
    Cursor,
    PageLimit,
    ConfigDocument(ConfigDocumentError),
    ConfigInput,
    Request(RequestError),
    RunRequest(Box<RunRequestError>),
    RunIdGeneration,
    Output,
    Usage,
}

impl CliError {
    const fn code(&self) -> &'static str {
        match self {
            Self::Composition(_) => "composition_failed",
            Self::ConfigName => "invalid_config_name",
            Self::ConfigDigest => "invalid_config_digest",
            Self::RunId => "invalid_run_id",
            Self::Cursor => "invalid_cursor",
            Self::PageLimit => "invalid_page_limit",
            Self::ConfigDocument(error) => error.code(),
            Self::ConfigInput => "config_input_unavailable",
            Self::Request(error) => error.code(),
            Self::RunRequest(error) => error.code(),
            Self::RunIdGeneration => "run_id_generation_failed",
            Self::Output => "output_failed",
            Self::Usage => "invalid_usage",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Composition(error) => error.to_string(),
            Self::ConfigName => "config name is invalid".to_owned(),
            Self::ConfigDigest => "config digest is invalid".to_owned(),
            Self::RunId => "run id is invalid".to_owned(),
            Self::Cursor => "cursor is invalid".to_owned(),
            Self::PageLimit => "page limit is invalid".to_owned(),
            Self::ConfigDocument(error) => error.to_string(),
            Self::ConfigInput => "config input is unavailable".to_owned(),
            Self::Request(error) => error.to_string(),
            Self::RunRequest(error) => error.to_string(),
            Self::RunIdGeneration => "run id generation failed".to_owned(),
            Self::Output => "output could not be written".to_owned(),
            Self::Usage => "command usage is invalid".to_owned(),
        }
    }

    const fn recovery(&self) -> Option<&RunRecovery> {
        match self {
            Self::RunRequest(error) => error.recovery(),
            _ => None,
        }
    }
}

impl From<mfm_app::ComposeError> for CliError {
    fn from(error: mfm_app::ComposeError) -> Self {
        Self::Composition(error)
    }
}

impl From<RequestError> for CliError {
    fn from(error: RequestError) -> Self {
        Self::Request(error)
    }
}

impl From<RunRequestError> for CliError {
    fn from(error: RunRequestError) -> Self {
        Self::RunRequest(Box::new(error))
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let output = cli.output;
    match run(cli).await {
        Ok(code) => code,
        Err(error) => {
            if emit_error(output, &error).is_err() {
                return ExitCode::from(2);
            }
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, CliError> {
    if matches!(cli.command, Command::EntryPoint { .. }) && cli.deployment.is_some() {
        return Err(CliError::Usage);
    }
    match cli.command {
        Command::EntryPoint {
            command: EntryPointCommand::List,
        } => emit_entry_points(cli.output),
        Command::Store {
            command: StoreCommand::Init {
                admin_store_locator_env,
            },
        } => {
            let deployment = Deployment::load(cli.deployment.as_deref()).await?;
            let admin = EnvironmentName::new(admin_store_locator_env)
                .map_err(|_| CliError::Composition(mfm_app::ComposeError::Deployment))?;
            provision_store(&deployment, &admin).await?;
            emit_empty(cli.output)
        }
        Command::Binding {
            command: BindingCommand::List,
        } => {
            let application = open(cli.deployment.as_deref()).await?;
            emit_bindings(cli.output, &application)
        }
        Command::Config { command } => {
            run_config(cli.output, cli.deployment.as_deref(), command).await
        }
        Command::Run { command } => run_run(cli.output, cli.deployment.as_deref(), command).await,
    }
}

async fn open(path: Option<&Path>) -> Result<Application, CliError> {
    let deployment = Deployment::load(path).await?;
    Application::open(&deployment).await.map_err(Into::into)
}

async fn run_config(
    output: OutputFormat,
    deployment: Option<&Path>,
    command: ConfigCommand,
) -> Result<ExitCode, CliError> {
    match command {
        ConfigCommand::Import { name, from } => {
            let name = ConfigName::new(name).map_err(|_| CliError::ConfigName)?;
            let bytes = read_config_input(&from).await?;
            let document = ConfigDocument::new(bytes)
                .await
                .map_err(CliError::ConfigDocument)?;
            let application = open(deployment).await?;
            let result = application.import_config(name, document).await?;
            emit_serializable(output, &result, || render_config_summary(result.config()))
        }
        ConfigCommand::List { cursor, limit } => {
            let cursor = cursor
                .map(ConfigCursor::parse)
                .transpose()
                .map_err(|_| CliError::Cursor)?;
            let limit = page_limit(limit)?;
            let application = open(deployment).await?;
            let result = application
                .list_configs(&ConfigPageRequest::new(cursor, limit))
                .await?;
            emit_serializable(output, &result, || {
                let mut text = String::new();
                for item in result.items() {
                    text.push_str(&render_config_summary(item));
                }
                if let Some(cursor) = result.next_cursor() {
                    text.push_str(&format!("next_cursor={}\n", cursor.as_str()));
                }
                text
            })
        }
        ConfigCommand::Show { name } => {
            let name = ConfigName::new(name).map_err(|_| CliError::ConfigName)?;
            let application = open(deployment).await?;
            let result = application.read_config(&name).await?;
            emit_serializable(output, &result, || {
                let mut text = render_config_summary(result.config());
                text.push_str("document=");
                text.push_str(std::str::from_utf8(result.canonical_bytes()).unwrap_or("{}"));
                text.push('\n');
                text
            })
        }
        ConfigCommand::Delete { name, digest } => {
            let name = ConfigName::new(name).map_err(|_| CliError::ConfigName)?;
            let digest = ConfigDigest::parse(digest).map_err(|_| CliError::ConfigDigest)?;
            let application = open(deployment).await?;
            application.delete_config(&name, &digest).await?;
            emit_empty(output)
        }
    }
}

async fn run_run(
    output: OutputFormat,
    deployment: Option<&Path>,
    command: RunCommand,
) -> Result<ExitCode, CliError> {
    match command {
        RunCommand::Start {
            config,
            config_digest,
            run_id,
        } => {
            let name = ConfigName::new(config).map_err(|_| CliError::ConfigName)?;
            let selection = match config_digest {
                Some(digest) => ConfigSelection::Exact {
                    name,
                    digest: ConfigDigest::parse(digest).map_err(|_| CliError::ConfigDigest)?,
                },
                None => ConfigSelection::Current { name },
            };
            let run_id = match run_id {
                Some(value) => RunId::parse(value).map_err(|_| CliError::RunId)?,
                None => generate_run_id()?,
            };
            let application = open(deployment).await?;
            let result = application.start_run(run_id, &selection).await?;
            emit_start_result(output, &result)
        }
        RunCommand::Progress { run_id } => {
            let run_id = RunId::parse(run_id).map_err(|_| CliError::RunId)?;
            let application = open(deployment).await?;
            let view = application.progress_run(&run_id).await?;
            emit_run_view(output, &view)
        }
        RunCommand::Show { run_id } => {
            let run_id = RunId::parse(run_id).map_err(|_| CliError::RunId)?;
            let application = open(deployment).await?;
            let view = application.read_run(&run_id).await?;
            emit_run_view(output, &view)
        }
        RunCommand::List { cursor, limit } => {
            let cursor = cursor
                .map(RunCursor::parse)
                .transpose()
                .map_err(|_| CliError::Cursor)?;
            let limit = page_limit(limit)?;
            let application = open(deployment).await?;
            let page = application
                .list_runs(&RunPageRequest::new(cursor, limit))
                .await?;
            emit_serializable(output, &page, || {
                let mut text = String::new();
                for item in page.items() {
                    text.push_str(&format!(
                        "run_id={}\nhead_sequence={}\nhead_digest={}\ntotal_bytes={}\n",
                        item.run_id(),
                        item.head_sequence(),
                        item.head_digest(),
                        item.total_bytes()
                    ));
                }
                if let Some(cursor) = page.next_cursor() {
                    text.push_str(&format!("next_cursor={}\n", cursor.as_str()));
                }
                text
            })
        }
    }
}

fn page_limit(value: Option<usize>) -> Result<PageLimit, CliError> {
    value
        .map(PageLimit::new)
        .transpose()
        .map(Option::unwrap_or_default)
        .map_err(|_| CliError::PageLimit)
}

async fn read_config_input(path: &Path) -> Result<Vec<u8>, CliError> {
    let mut bytes = Vec::new();
    if path == Path::new("-") {
        tokio::io::stdin()
            .take((MAX_CONFIG_DOCUMENT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| CliError::ConfigInput)?;
    } else {
        tokio::fs::File::open(path)
            .await
            .map_err(|_| CliError::ConfigInput)?
            .take((MAX_CONFIG_DOCUMENT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| CliError::ConfigInput)?;
    }
    Ok(bytes)
}

fn generate_run_id() -> Result<RunId, CliError> {
    generate_run_id_with(getrandom::fill)
}

fn generate_run_id_with<E>(
    fill: impl FnOnce(&mut [u8]) -> Result<(), E>,
) -> Result<RunId, CliError> {
    let mut entropy = [0; 32];
    fill(&mut entropy).map_err(|_| CliError::RunIdGeneration)?;
    Ok(derive_run_id(entropy))
}

fn emit_entry_points(output: OutputFormat) -> Result<ExitCode, CliError> {
    let items = Application::entry_points();
    emit_serializable(output, &EntryPointList::new(items), || {
        items
            .iter()
            .map(|item| format!("entry_point={}\n", item.entry_point()))
            .collect()
    })
}

fn emit_bindings(output: OutputFormat, application: &Application) -> Result<ExitCode, CliError> {
    emit_serializable(output, &BindingList::new(application.bindings()), || {
        application
            .bindings()
            .iter()
            .map(|binding| match binding {
                mfm_app::PublicBindingView::Evm {
                    chain_id,
                    endpoint_id,
                    binding_ref,
                } => format!(
                    "kind=evm\nchain_id={chain_id}\nendpoint_id={endpoint_id}\nbinding_ref={}\n",
                    serde_json::to_string(binding_ref).unwrap_or_else(|_| "{}".to_owned())
                ),
            })
            .collect()
    })
}

fn emit_empty(output: OutputFormat) -> Result<ExitCode, CliError> {
    match output {
        OutputFormat::Text => Ok(ExitCode::SUCCESS),
        OutputFormat::Json => emit_json(&serde_json::json!({})),
    }
}

fn emit_start_result(output: OutputFormat, result: &StartRunResult) -> Result<ExitCode, CliError> {
    match output {
        OutputFormat::Json => emit_json_with_run_status(result, result.run()),
        OutputFormat::Text => {
            let mut text = render_config_summary(result.config());
            text.push_str(&render_run_view(result.run()));
            write_stdout(text.as_bytes())?;
            Ok(run_exit(result.run()))
        }
    }
}

fn emit_run_view(output: OutputFormat, view: &RunView) -> Result<ExitCode, CliError> {
    match output {
        OutputFormat::Json => emit_json_with_run_status(&SerializableRunView::new(view), view),
        OutputFormat::Text => {
            write_stdout(render_run_view(view).as_bytes())?;
            Ok(run_exit(view))
        }
    }
}

fn emit_json_with_run_status(value: &impl Serialize, view: &RunView) -> Result<ExitCode, CliError> {
    write_json_stdout(value)?;
    Ok(run_exit(view))
}

fn emit_serializable(
    output: OutputFormat,
    value: &impl Serialize,
    text: impl FnOnce() -> String,
) -> Result<ExitCode, CliError> {
    match output {
        OutputFormat::Text => {
            write_stdout(text().as_bytes())?;
            Ok(ExitCode::SUCCESS)
        }
        OutputFormat::Json => emit_json(value),
    }
}

fn emit_json(value: &impl Serialize) -> Result<ExitCode, CliError> {
    write_json_stdout(value)?;
    Ok(ExitCode::SUCCESS)
}

fn write_json_stdout(value: &impl Serialize) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer(&mut stdout, value).map_err(|_| CliError::Output)?;
    stdout.write_all(b"\n").map_err(|_| CliError::Output)?;
    stdout.flush().map_err(|_| CliError::Output)
}

fn write_stdout(bytes: &[u8]) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(bytes).map_err(|_| CliError::Output)?;
    stdout.flush().map_err(|_| CliError::Output)
}

fn emit_error(output: OutputFormat, error: &CliError) -> Result<(), ()> {
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    match output {
        OutputFormat::Text => {
            writeln!(stderr, "error: {}", error.message()).map_err(|_| ())?;
            if let Some(recovery) = error.recovery() {
                render_recovery_text(&mut stderr, recovery)?;
            }
        }
        OutputFormat::Json => {
            let value = ErrorView { error };
            serde_json::to_writer(&mut stderr, &value).map_err(|_| ())?;
            stderr.write_all(b"\n").map_err(|_| ())?;
        }
    }
    stderr.flush().map_err(|_| ())
}

struct ErrorView<'a> {
    error: &'a CliError,
}

impl Serialize for ErrorView<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let recovery = self.error.recovery();
        let mut state =
            serializer.serialize_struct("Error", 2 + usize::from(recovery.is_some()))?;
        state.serialize_field("code", self.error.code())?;
        state.serialize_field("message", &self.error.message())?;
        if let Some(recovery) = recovery {
            state.serialize_field("recovery", recovery)?;
        }
        state.end()
    }
}

fn render_recovery_text(writer: &mut impl Write, recovery: &RunRecovery) -> Result<(), ()> {
    match recovery {
        RunRecovery::Start { run_id, config } => {
            writeln!(writer, "recovery.kind=start").map_err(|_| ())?;
            writeln!(writer, "recovery.run_id={run_id}").map_err(|_| ())?;
            writeln!(writer, "recovery.config_name={}", config.name()).map_err(|_| ())?;
            writeln!(writer, "recovery.config_digest={}", config.digest()).map_err(|_| ())?;
            writeln!(writer, "recovery.entry_point={}", config.entry_point()).map_err(|_| ())
        }
        RunRecovery::Progress { run_id } => {
            writeln!(writer, "recovery.kind=progress").map_err(|_| ())?;
            writeln!(writer, "recovery.run_id={run_id}").map_err(|_| ())
        }
    }
}

fn render_config_summary(config: &mfm_app::ConfigSummary) -> String {
    format!(
        "config_name={}\nconfig_digest={}\nentry_point={}\n",
        config.name(),
        config.digest(),
        config.entry_point()
    )
}

fn render_run_view(view: &RunView) -> String {
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
    );
    if let RunViewState::Succeeded(value) | RunViewState::Failed(value) = view.state() {
        rendered.push_str(&format!(
            "contract_ref={}\nvalue_ref={}\nvalue=",
            serde_json::to_string(value.contract_ref()).unwrap_or_else(|_| "{}".to_owned()),
            serde_json::to_string(value.value_ref()).unwrap_or_else(|_| "{}".to_owned())
        ));
        rendered.push_str(std::str::from_utf8(value.canonical_bytes()).unwrap_or("{}"));
        rendered.push('\n');
    }
    rendered
}

fn run_exit(view: &RunView) -> ExitCode {
    match view.state() {
        RunViewState::Succeeded(_) => ExitCode::SUCCESS,
        RunViewState::Runnable | RunViewState::Failed(_) => ExitCode::from(1),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mfm_app::{BoundCapabilitySet, ComposedRuntime};
    use mfm_catalog::MemoryCatalog;
    use mfm_store::MemoryStore;

    use super::*;

    const DOCUMENT: &[u8] = br#"{
      "input":{"portfolio":{"quotes":["usd"],"portfolio_id":"portfolio-example","collections":[{"request":{"sources":[{"token":null,"source_id":"wallet-0.native","chain_id":1,"address":"0x1111111111111111111111111111111111111111"}],"decimals":18},"correlation":"native-0"}]},"selector":{"quote":"usd","target":"portfolio-example"},"routes":[{"endpoint_id":"alpha","chain_id":1}]},
      "entry_point":"mfm.portfolio/snapshot@1"}"#;

    #[test]
    fn page_limit_and_checked_scalar_errors_are_stable() {
        assert!(page_limit(None).is_ok());
        assert!(matches!(page_limit(Some(0)), Err(CliError::PageLimit)));
        assert!(matches!(page_limit(Some(201)), Err(CliError::PageLimit)));
        assert_eq!(CliError::ConfigName.code(), "invalid_config_name");
        assert_eq!(CliError::RunId.code(), "invalid_run_id");
        assert_eq!(CliError::RunIdGeneration.code(), "run_id_generation_failed");
    }

    #[test]
    fn run_id_generation_consumes_exactly_32_bytes_and_fails_closed() {
        let generated = generate_run_id_with(|entropy| {
            assert_eq!(entropy.len(), 32);
            entropy.copy_from_slice(&[7; 32]);
            Ok::<(), ()>(())
        })
        .expect("generated id");
        assert_eq!(generated, derive_run_id([7; 32]));
        assert!(matches!(
            generate_run_id_with(|entropy| {
                assert_eq!(entropy.len(), 32);
                Err::<(), ()>(())
            }),
            Err(CliError::RunIdGeneration)
        ));
    }

    #[tokio::test]
    async fn recovery_json_matches_the_cross_transport_fixtures() {
        let store = Arc::new(MemoryStore::new());
        let bindings = BoundCapabilitySet::new(Vec::new()).expect("bindings");
        let composed = ComposedRuntime::compose(store, bindings).expect("composition");
        let application =
            Application::from_parts(composed, Arc::new(MemoryCatalog::new())).expect("application");
        let document = ConfigDocument::new(DOCUMENT.to_vec())
            .await
            .expect("document");
        let outcome = application
            .import_config(ConfigName::new("daily").expect("name"), document)
            .await
            .expect("import");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .expect("run id");
        let start = CliError::from(RunRequestError::AppendIndeterminate {
            recovery: RunRecovery::Start {
                run_id: run_id.clone(),
                config: outcome.config().clone(),
            },
        });
        assert_fixture(
            &ErrorView { error: &start },
            include_str!("../../../docs/contracts/client-surface/run-recovery-start.json"),
        );
        let progress = CliError::from(RunRequestError::AppendIndeterminate {
            recovery: RunRecovery::Progress { run_id },
        });
        assert_fixture(
            &ErrorView { error: &progress },
            include_str!("../../../docs/contracts/client-surface/run-recovery-progress.json"),
        );
    }

    fn assert_fixture(actual: &impl Serialize, expected: &str) {
        assert_eq!(
            serde_json::to_value(actual).expect("actual JSON"),
            serde_json::from_str::<serde_json::Value>(expected).expect("fixture JSON")
        );
    }
}
