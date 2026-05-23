#![allow(clippy::disallowed_methods)]

use std::path::PathBuf;
use std::sync::Arc;

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_machine::engine::Stores;
use mfm_machine::stores::{ArtifactStore, StreamStore};
use mfm_stream_store_mem::MemStreamStore;

/// Builds the artifact store selected by CLI arguments and environment.
pub(crate) fn make_artifact_store(artifact_root: Option<PathBuf>) -> Arc<dyn ArtifactStore> {
    let artifact_root = artifact_root.unwrap_or_else(mfm_app_legacy::default_artifact_root);
    Arc::new(FsArtifactStore::new(artifact_root))
}

/// Builds in-memory stream storage with the standard artifact store for ephemeral commands.
pub(crate) fn make_ephemeral_stores(artifact_root: Option<PathBuf>) -> Stores {
    let artifacts = make_artifact_store(artifact_root);
    let streams: Arc<dyn StreamStore> = Arc::new(MemStreamStore::new());

    Stores { streams, artifacts }
}
