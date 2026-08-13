use clap::{Parser, Subcommand};
use mfm_app::{parse_admission_json, AdmitRunRequest, Application, MAX_ADMISSION_BYTES};
use mfm_ids::{RunId, StableId, StoreEpoch, StoreScopeId, TenantScopeId};

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
    let app = Application::for_tenant(
        TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")?,
        StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")?,
        StoreEpoch::new(1),
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
