use mfm_store::v1::{StoreError, StoreErrorInspection};

/// Error returned by the qualified PostgreSQL journal store.
#[derive(Debug, thiserror::Error)]
pub enum PostgresStoreError {
    /// PostgreSQL could not be reached through the supplied writer pool.
    #[error("postgres writer connection failed")]
    Connection,
    /// The compiled destructive baseline and the connected schema differ.
    #[error("postgres store schema authority mismatch")]
    SchemaAuthorityMismatch,
    /// The applied migration checksum differs from the compiled baseline.
    #[error("postgres store migration checksum mismatch")]
    MigrationChecksumMismatch,
    /// The connection is read-only, in recovery, or not authorized as the application role.
    #[error("postgres connection is not the authoritative writer")]
    WriterRequired,
    /// The deployment-owned non-rollback writer fence rejected qualification.
    #[error("postgres authoritative-writer fence rejected qualification")]
    WriterFenceRejected,
    /// The PostgreSQL commit outcome could not be classified after connection loss.
    #[error("postgres commit outcome is unknown")]
    OutcomeUnknown,
    /// The shared journal/store contract rejected the operation.
    #[error("{0}")]
    Store(#[source] Box<StoreError>),
    /// A bounded PostgreSQL operation failed.
    #[error("postgres store operation failed: {0}")]
    Database(&'static str),
    /// Retained rows disagree with the frozen authority contract.
    #[error("postgres store retained authority is corrupt: {0}")]
    Corruption(&'static str),
}

/// Result returned by this crate.
pub type Result<T> = std::result::Result<T, PostgresStoreError>;

pub(crate) fn database_error(context: &'static str, _error: sqlx::Error) -> PostgresStoreError {
    PostgresStoreError::Database(context)
}

pub(crate) fn ambiguous_commit_error(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::Protocol(_)
            | sqlx::Error::WorkerCrashed
    )
}

impl From<StoreError> for PostgresStoreError {
    fn from(error: StoreError) -> Self {
        Self::Store(Box::new(error))
    }
}

impl StoreErrorInspection for PostgresStoreError {
    fn as_store_error(&self) -> Option<&StoreError> {
        match self {
            Self::Store(error) => Some(error.as_ref()),
            Self::Connection
            | Self::SchemaAuthorityMismatch
            | Self::MigrationChecksumMismatch
            | Self::WriterRequired
            | Self::WriterFenceRejected
            | Self::OutcomeUnknown
            | Self::Database(_)
            | Self::Corruption(_) => None,
        }
    }
}

impl From<mfm_journal::v1::JournalError> for PostgresStoreError {
    fn from(error: mfm_journal::v1::JournalError) -> Self {
        Self::from(StoreError::from(error))
    }
}
