/// Result returned by the PostgreSQL executor backend.
pub type Result<T> = std::result::Result<T, PostgresExecutorStoreError>;

/// Closed, redaction-safe PostgreSQL executor-storage errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PostgresExecutorStoreError {
    /// The supplied connection is not the dedicated writable executor authority.
    #[error("executor PostgreSQL writer is not qualified")]
    WriterRequired,
    /// The dedicated executor schema, roles, or privileges do not match the compiled contract.
    #[error("executor PostgreSQL schema authority mismatch")]
    SchemaAuthorityMismatch,
    /// The compiled migration history does not match the database.
    #[error("executor PostgreSQL migration checksum mismatch")]
    MigrationChecksumMismatch,
    /// The immutable executor binding does not equal the requested store identity.
    #[error("executor PostgreSQL binding mismatch")]
    BindingMismatch,
    /// The independent deployment writer-generation fence rejected this writer.
    #[error("executor PostgreSQL writer-generation fence rejected")]
    WriterFenceRejected,
    /// Persisted immutable ledger rows fail strict reconstruction or refold.
    #[error("executor PostgreSQL ledger is corrupt")]
    CorruptLedger,
    /// A database operation failed before a commit could become ambiguous.
    #[error("executor PostgreSQL operation failed")]
    Database,
}
