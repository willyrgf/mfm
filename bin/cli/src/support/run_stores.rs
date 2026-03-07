use std::path::PathBuf;
use std::sync::Arc;

use crate::commands::result::CommandError;
use clap::Args;
use mfm_artifact_store_fs::FsArtifactStore;
use mfm_event_store_mem::MemEventStore;
use mfm_event_store_postgres::PostgresEventStore;
use mfm_machine::engine::Stores;
use mfm_machine::stores::{ArtifactStore, EventStore};

/// Shared store selection arguments for commands that access persisted runs.
#[derive(Args, Debug, Clone)]
pub struct RunStoresArgs {
    /// Root directory for run artifacts (default: $MFM_ARTIFACT_ROOT or ~/.mfm/run_artifacts)
    #[arg(long)]
    pub artifact_root: Option<PathBuf>,

    /// PostgreSQL connection string for the event store (default: $DATABASE_URL)
    #[arg(long)]
    pub database_url: Option<String>,
}

fn default_artifact_root() -> PathBuf {
    std::env::var("MFM_ARTIFACT_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mfm").join("run_artifacts")
        })
}

/// Builds the artifact store selected by CLI arguments and environment.
pub fn make_artifact_store(artifact_root: Option<PathBuf>) -> Arc<dyn ArtifactStore> {
    let artifact_root = artifact_root.unwrap_or_else(default_artifact_root);
    Arc::new(FsArtifactStore::new(artifact_root))
}

async fn make_event_store(
    database_url: Option<String>,
) -> Result<Arc<dyn EventStore>, CommandError> {
    let database_url = match database_url.or_else(|| std::env::var("DATABASE_URL").ok()) {
        Some(s) => s,
        None => {
            return Err(CommandError::new(
                "MissingDatabaseUrl",
                "Missing DATABASE_URL (or pass --database-url)",
            ))
        }
    };

    let store = PostgresEventStore::connect(&database_url)
        .await
        .map_err(|e| match e {
            mfm_machine::errors::StorageError::Concurrency(info)
            | mfm_machine::errors::StorageError::NotFound(info)
            | mfm_machine::errors::StorageError::Corruption(info)
            | mfm_machine::errors::StorageError::Other(info) => {
                CommandError::new(info.code.0, info.message)
            }
        })?;

    Ok(Arc::new(store))
}

/// Builds the persistent event and artifact stores selected by CLI arguments.
pub async fn make_stores(
    artifact_root: Option<PathBuf>,
    database_url: Option<String>,
) -> Result<Stores, CommandError> {
    let artifacts = make_artifact_store(artifact_root);
    let events = make_event_store(database_url).await?;

    Ok(Stores { events, artifacts })
}

/// Builds in-memory event storage with the standard artifact store for ephemeral commands.
pub fn make_ephemeral_stores(artifact_root: Option<PathBuf>) -> Stores {
    let artifacts = make_artifact_store(artifact_root);
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());

    Stores { events, artifacts }
}
