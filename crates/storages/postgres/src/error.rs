/// Error returned while qualifying a PostgreSQL structured-history store.
#[derive(Debug, thiserror::Error)]
pub enum PostgresStoreError {
    /// PostgreSQL could not be reached through the supplied session materials.
    #[error("postgres writer connection failed")]
    Connection,
    /// The compiled destructive baseline and the connected schema differ.
    #[error("postgres store schema authority mismatch")]
    SchemaAuthorityMismatch,
    /// The applied migration checksum differs from the compiled baseline.
    #[error("postgres store migration checksum mismatch")]
    MigrationChecksumMismatch,
    /// The connection is not the exact restricted session profile for this target.
    #[error("postgres connection is not the authoritative writer")]
    WriterRequired,
    /// The opaque deployment-issued target session bundle was rejected.
    #[error("postgres target session bundle rejected qualification")]
    TargetSessionRejected,
    /// A copied, expired, wrong-release, or stale-generation target permit was rejected.
    #[error("postgres target permit is not current")]
    TargetPermitRejected,
    /// The PostgreSQL commit outcome could not be classified after connection loss.
    #[error("postgres commit outcome is unknown")]
    OutcomeUnknown,
    /// A bounded PostgreSQL operation failed.
    #[error("postgres store operation failed: {0}")]
    Database(&'static str),
    /// Retained rows disagree with the frozen authority contract.
    #[error("postgres store retained authority is corrupt: {0}")]
    Corruption(&'static str),
}

/// Result returned by this crate.
pub type Result<T> = std::result::Result<T, PostgresStoreError>;
