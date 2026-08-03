//! Redaction-safe PostgreSQL wallet-authority errors.

/// Result type for PostgreSQL EVM wallet storage.
pub type Result<T> = std::result::Result<T, PostgresEvmWalletError>;

/// Redaction-safe failure at the storage construction or maintenance boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PostgresEvmWalletError {
    /// The database was unavailable.
    #[error("EVM wallet PostgreSQL database is unavailable")]
    Unavailable,
    /// The installed schema, role, or retained authority was not exact.
    #[error("EVM wallet PostgreSQL authority contract is invalid")]
    InvalidAuthority,
    /// An immutable registry or semantic operation key conflicted.
    #[error("EVM wallet PostgreSQL permanent identity conflicts")]
    PermanentConflict,
    /// The deployment-owned target fence did not qualify the requested target.
    #[error("EVM wallet PostgreSQL target fence rejected the target")]
    FenceRejected,
}
