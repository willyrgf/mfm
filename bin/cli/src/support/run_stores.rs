#![allow(clippy::disallowed_methods)]

use std::path::PathBuf;
use std::sync::Arc;

use crate::commands::result::CommandError;
use clap::Args;
use mfm_artifact_store_fs::FsArtifactStore;
use mfm_machine::engine::Stores;
use mfm_machine::stores::{ArtifactStore, StreamStore};
use mfm_stream_store_mem::MemStreamStore;
use mfm_stream_store_postgres::PostgresStreamStore;

/// Shared store selection arguments for commands that access persisted runs.
#[derive(Args, Debug, Clone)]
pub(crate) struct RunStoresArgs {
    /// Root directory for run artifacts (default: $MFM_ARTIFACT_ROOT or ~/.mfm/run_artifacts)
    #[arg(long)]
    pub(crate) artifact_root: Option<PathBuf>,

    /// PostgreSQL connection string for the stream store (default: $DATABASE_URL)
    #[arg(long)]
    pub(crate) database_url: Option<String>,
}

/// Builds the artifact store selected by CLI arguments and environment.
pub(crate) fn make_artifact_store(artifact_root: Option<PathBuf>) -> Arc<dyn ArtifactStore> {
    let artifact_root = artifact_root.unwrap_or_else(mfm_app::default_artifact_root);
    Arc::new(FsArtifactStore::new(artifact_root))
}

async fn make_stream_store(
    database_url: Option<String>,
) -> Result<Arc<dyn StreamStore>, CommandError> {
    let database_url = match database_url.or_else(|| std::env::var("DATABASE_URL").ok()) {
        Some(s) => s,
        None => {
            return Err(CommandError::new(
                "MissingDatabaseUrl",
                "Missing DATABASE_URL (or pass --database-url)",
            ))
        }
    };

    let store = PostgresStreamStore::connect(&database_url)
        .await
        .map_err(|e| match e {
            mfm_machine::errors::StorageError::Concurrency(info)
            | mfm_machine::errors::StorageError::NotFound(info)
            | mfm_machine::errors::StorageError::Corruption(info)
            | mfm_machine::errors::StorageError::Other(info) => {
                CommandError::new(info.code.as_str(), info.message)
            }
        })?;

    Ok(Arc::new(store))
}

/// Builds the persistent stream and artifact stores selected by CLI arguments.
pub(crate) async fn make_stores(
    artifact_root: Option<PathBuf>,
    database_url: Option<String>,
) -> Result<Stores, CommandError> {
    let artifacts =
        mfm_app::wrap_protected_artifact_store_if_configured(make_artifact_store(artifact_root))
            .map_err(|err| CommandError::new(err.code, err.message))?;
    let streams = make_stream_store(database_url).await?;

    Ok(Stores { streams, artifacts })
}

/// Builds in-memory stream storage with the standard artifact store for ephemeral commands.
pub(crate) fn make_ephemeral_stores(artifact_root: Option<PathBuf>) -> Stores {
    let artifacts = make_artifact_store(artifact_root);
    let streams: Arc<dyn StreamStore> = Arc::new(MemStreamStore::new());

    Stores { streams, artifacts }
}
