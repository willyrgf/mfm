use std::path::PathBuf;

use thiserror::Error;

/// Top-level error type for the Phase-1 publish-docs tool.
#[derive(Debug, Error)]
pub enum PublishDocsError {
    /// The workspace root could not be located from the current directory.
    #[error("unable to locate workspace root from {start_dir}")]
    WorkspaceRootNotFound {
        /// Directory from which root discovery began.
        start_dir: PathBuf,
    },

    /// The requested package selection could not be resolved from the wave catalog.
    #[error("package selection matched no packages")]
    EmptySelection,

    /// A requested package name was not present in the wave catalog.
    #[error("unknown package in selection: {package}")]
    UnknownPackage {
        /// Package name requested by the caller.
        package: String,
    },

    /// A package referenced by the wave catalog was missing from cargo metadata.
    #[error("package missing from cargo metadata: {package}")]
    PackageMissingFromMetadata {
        /// Package name missing from `cargo metadata`.
        package: String,
    },

    /// The local working tree is dirty and `--allow-dirty` was not supplied.
    #[error("working tree has uncommitted changes; commit first or pass --allow-dirty")]
    DirtyWorkingTree,

    /// The remote registry reported a version newer than the local package version.
    #[error(
        "remote version is newer than local package={package} local_version={local_version} remote_version={remote_version}"
    )]
    RemoteNewerThanLocal {
        /// Package name.
        package: String,
        /// Local version string.
        local_version: String,
        /// Remote version string.
        remote_version: String,
    },

    /// A JSON payload could not be rendered.
    #[error("failed to render json output: {0}")]
    Json(#[from] serde_json::Error),

    /// A network request failed.
    #[error(transparent)]
    Http(#[from] reqwest::Error),

    /// A semantic version could not be parsed.
    #[error(transparent)]
    Semver(#[from] semver::Error),

    /// A generic I/O error occurred.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// An external command failed.
    #[error("{message}")]
    CommandFailed {
        /// High-level sanitized message.
        message: String,
    },

    /// A generic catch-all error bubbled up from orchestration glue.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
