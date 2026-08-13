use clap::{Parser, Subcommand};
use mfm_app::{
    application_catalog, parse_admission_json, AdmitRunRequest, Application, MAX_ADMISSION_BYTES,
};
use mfm_evm::EvmConfig;
use mfm_ids::{AppendRequestId, RunId, StableId, StoreEpoch, StoreScopeId, TenantScopeId};
use mfm_portfolio::{PortfolioConfig, PortfolioId, QuoteCode};
use mfm_store::{
    ConfigurationCommitOutcome, StoreWorkLimits, StructuredStore, StructuredStoreIdentity,
};
use mfm_values::ValidatedConfig;

#[derive(Parser)]
#[command(name = "mfm", version, about = "fixed-tenant MFM run facade")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Admit {
        #[arg(long)]
        entry_point: String,
        #[arg(long)]
        input: String,
    },
    Drive {
        run_id: String,
    },
    Read {
        run_id: String,
    },
    Replay {
        run_id: String,
    },
    Trace {
        run_id: String,
    },
    Audit {
        run_id: String,
    },
    Export {
        run_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let tenant = TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")?;
    let store = StructuredStore::open_memory(
        StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")?,
            StoreEpoch::new(1),
            tenant,
        ),
        application_catalog()?,
        StoreWorkLimits::default(),
    )?;
    let (history, reader, configuration, audit) = store.split().into_parts();
    let portfolio = configuration
        .initial_write_session::<PortfolioConfig>()
        .prepare_local(
            AppendRequestId::new("cli-portfolio-configuration-0001")?,
            ValidatedConfig::new(PortfolioConfig {
                portfolio_id: PortfolioId {
                    value: "demo-portfolio".to_owned(),
                },
                quotes: vec![QuoteCode::Usd],
            })?,
        )?;
    let portfolio = match configuration.commit(portfolio).await? {
        ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
        _ => return Err("failed to persist Portfolio configuration".into()),
    };
    let portfolio_head = portfolio.head().clone();
    let evm = configuration
        .write_session::<EvmConfig>(&portfolio_head)?
        .prepare_local(
            AppendRequestId::new("cli-evm-configuration-000000001")?,
            ValidatedConfig::new(EvmConfig {})?,
        )?;
    let evm = match configuration.commit(evm).await? {
        ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
        _ => return Err("failed to persist EVM configuration".into()),
    };
    let app = Application::new(
        history,
        reader,
        configuration,
        audit,
        portfolio_head,
        evm.into_head(),
        Vec::new(),
    )?;
    let output = match cli.command {
        Command::Admit { entry_point, input } => {
            if input.len() > MAX_ADMISSION_BYTES {
                return Err("input exceeds the admission bound".into());
            }
            let entry = StableId::new(entry_point)?;
            let value = parse_admission_json(&input).map_err(|_| "invalid input")?;
            serde_json::to_string(&app.admit_run(AdmitRunRequest::new(entry, value)?).await?)?
        }
        Command::Drive { run_id } => {
            serde_json::to_string(&app.drive(RunId::parse(run_id)?).await?)?
        }
        Command::Read { run_id } => {
            serde_json::to_string(&app.read_public_run(RunId::parse(run_id)?).await?)?
        }
        Command::Replay { run_id } => {
            serde_json::to_string(&app.replay_run(RunId::parse(run_id)?).await?)?
        }
        Command::Trace { run_id } => {
            serde_json::to_string(&app.trace_run(RunId::parse(run_id)?).await?)?
        }
        Command::Audit { run_id } => {
            serde_json::to_string(&app.audit_access(RunId::parse(run_id)?).await?)?
        }
        Command::Export { run_id } => {
            let export = app.export_run(RunId::parse(run_id)?).await?;
            std::str::from_utf8(export.bytes())?.to_owned()
        }
    };
    println!("{output}");
    Ok(())
}
