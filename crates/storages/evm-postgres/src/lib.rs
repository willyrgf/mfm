#![warn(missing_docs)]
//! Narrow EVM wallet nonce storage used by a qualified adapter.
//!
//! This crate owns only the external tenant/sender/nonce-domain identity and its durable
//! idempotency rows. It is not a run-history coordinator, Runtime lease, or application access
//! service.

use std::collections::BTreeMap;
use std::sync::Mutex;

use mfm_ids::{StableId, TenantScopeId};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};

const SCHEMA_CONTRACT: &str = "mfm.single-trust-evm-nonce-postgres.v1";

/// Redaction-safe wallet nonce storage error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WalletAuthorityError {
    /// The public wallet domain or operation key is invalid.
    #[error("wallet nonce domain is invalid")]
    Invalid,
    /// A nonce key was absent when completion was requested.
    #[error("wallet nonce operation is not actionable")]
    NotActionable,
    /// The database operation failed without exposing driver diagnostics.
    #[error("wallet nonce storage failed")]
    Storage,
    /// The database schema contract is not installed.
    #[error("wallet nonce schema contract is invalid")]
    Schema,
}

/// Result type for wallet nonce storage.
pub type Result<T> = std::result::Result<T, WalletAuthorityError>;

/// Public immutable wallet nonce-domain identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletNonceDomain {
    /// Fixed tenant partition.
    pub tenant: TenantScopeId,
    /// Public sender identity.
    pub sender: StableId,
    /// Public wallet nonce domain.
    pub nonce_domain: StableId,
}

impl WalletNonceDomain {
    /// Validates the public domain identity.
    pub fn validate(&self) -> Result<()> {
        if self.sender.as_str().is_empty() || self.nonce_domain.as_str().is_empty() {
            Err(WalletAuthorityError::Invalid)
        } else {
            Ok(())
        }
    }
}

/// Durable PostgreSQL wallet nonce authority for one exact domain.
pub struct PostgresWalletNonceStore {
    pool: PgPool,
    domain: WalletNonceDomain,
    first_nonce: u64,
}

impl PostgresWalletNonceStore {
    /// Connects to one database and checks the fresh nonce schema.
    pub async fn connect(
        database_url: &str,
        domain: WalletNonceDomain,
        first_nonce: u64,
    ) -> Result<Self> {
        domain.validate()?;
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(database_url)
            .await
            .map_err(|_| WalletAuthorityError::Storage)?;
        let store = Self::from_pool(pool, domain, first_nonce)?;
        store.check_ready().await?;
        Ok(store)
    }

    /// Wraps one opened pool without exposing driver diagnostics.
    pub fn from_pool(pool: PgPool, domain: WalletNonceDomain, first_nonce: u64) -> Result<Self> {
        domain.validate()?;
        Ok(Self {
            pool,
            domain,
            first_nonce,
        })
    }

    /// Installs the fresh nonce-domain schema in a caller-selected database.
    pub async fn migrate(pool: &PgPool) -> Result<()> {
        for statement in include_str!("../migrations/0001_nonce_domain.sql").split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                sqlx::query(statement)
                    .execute(pool)
                    .await
                    .map_err(|_| WalletAuthorityError::Storage)?;
            }
        }
        Ok(())
    }

    /// Checks only the installed schema contract.
    pub async fn check_ready(&self) -> Result<()> {
        let contract: Option<String> = sqlx::query_scalar(
            "SELECT schema_contract FROM mfm_evm_nonce_schema WHERE schema_contract = $1",
        )
        .bind(SCHEMA_CONTRACT)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| WalletAuthorityError::Schema)?;
        if contract.as_deref() == Some(SCHEMA_CONTRACT) {
            Ok(())
        } else {
            Err(WalletAuthorityError::Schema)
        }
    }

    /// Returns the fixed wallet nonce domain.
    pub const fn domain(&self) -> &WalletNonceDomain {
        &self.domain
    }

    /// Reserves one nonce for one non-secret idempotency key.
    pub async fn reserve(&self, operation_key: StableId) -> Result<u64> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| WalletAuthorityError::Storage)?;
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT nonce FROM mfm_evm_nonce_operations
             WHERE tenant_scope_id = $1 AND sender_id = $2 AND nonce_domain_id = $3
               AND operation_key = $4
             FOR UPDATE",
        )
        .bind(self.domain.tenant.as_str())
        .bind(self.domain.sender.as_str())
        .bind(self.domain.nonce_domain.as_str())
        .bind(operation_key.as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| WalletAuthorityError::Storage)?;
        if let Some(nonce) = existing {
            commit(transaction).await?;
            return u64::try_from(nonce).map_err(|_| WalletAuthorityError::Storage);
        }

        sqlx::query(
            "INSERT INTO mfm_evm_nonce_domains
             (tenant_scope_id, sender_id, nonce_domain_id, next_nonce)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (tenant_scope_id, sender_id, nonce_domain_id) DO NOTHING",
        )
        .bind(self.domain.tenant.as_str())
        .bind(self.domain.sender.as_str())
        .bind(self.domain.nonce_domain.as_str())
        .bind(i64::try_from(self.first_nonce).map_err(|_| WalletAuthorityError::Invalid)?)
        .execute(&mut *transaction)
        .await
        .map_err(|_| WalletAuthorityError::Storage)?;
        let next_nonce: i64 = sqlx::query_scalar(
            "SELECT next_nonce FROM mfm_evm_nonce_domains
             WHERE tenant_scope_id = $1 AND sender_id = $2 AND nonce_domain_id = $3
             FOR UPDATE",
        )
        .bind(self.domain.tenant.as_str())
        .bind(self.domain.sender.as_str())
        .bind(self.domain.nonce_domain.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| WalletAuthorityError::Storage)?;
        let next = u64::try_from(next_nonce).map_err(|_| WalletAuthorityError::Storage)?;
        let following = next.checked_add(1).ok_or(WalletAuthorityError::Invalid)?;
        sqlx::query(
            "UPDATE mfm_evm_nonce_domains SET next_nonce = $4
             WHERE tenant_scope_id = $1 AND sender_id = $2 AND nonce_domain_id = $3",
        )
        .bind(self.domain.tenant.as_str())
        .bind(self.domain.sender.as_str())
        .bind(self.domain.nonce_domain.as_str())
        .bind(i64::try_from(following).map_err(|_| WalletAuthorityError::Storage)?)
        .execute(&mut *transaction)
        .await
        .map_err(|_| WalletAuthorityError::Storage)?;
        sqlx::query(
            "INSERT INTO mfm_evm_nonce_operations
             (tenant_scope_id, sender_id, nonce_domain_id, operation_key, nonce, completed)
             VALUES ($1, $2, $3, $4, $5, FALSE)",
        )
        .bind(self.domain.tenant.as_str())
        .bind(self.domain.sender.as_str())
        .bind(self.domain.nonce_domain.as_str())
        .bind(operation_key.as_str())
        .bind(next_nonce)
        .execute(&mut *transaction)
        .await
        .map_err(|_| WalletAuthorityError::Storage)?;
        commit(transaction).await?;
        Ok(next)
    }

    /// Marks one operation complete without deleting its idempotency row.
    pub async fn complete(&self, operation_key: &StableId) -> Result<u64> {
        let nonce: Option<i64> = sqlx::query_scalar(
            "UPDATE mfm_evm_nonce_operations SET completed = TRUE
             WHERE tenant_scope_id = $1 AND sender_id = $2 AND nonce_domain_id = $3
               AND operation_key = $4
             RETURNING nonce",
        )
        .bind(self.domain.tenant.as_str())
        .bind(self.domain.sender.as_str())
        .bind(self.domain.nonce_domain.as_str())
        .bind(operation_key.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| WalletAuthorityError::Storage)?;
        nonce
            .map(|value| u64::try_from(value).map_err(|_| WalletAuthorityError::Storage))
            .unwrap_or(Err(WalletAuthorityError::NotActionable))
    }
}

async fn commit(transaction: Transaction<'_, Postgres>) -> Result<()> {
    transaction
        .commit()
        .await
        .map_err(|_| WalletAuthorityError::Storage)
}

struct MemoryNonceState {
    next_nonce: u64,
    operations: BTreeMap<StableId, (u64, bool)>,
}

/// In-memory nonce-domain implementation for local assembly and conformance tests.
pub struct PostgresWalletNonceAuthority {
    domain: WalletNonceDomain,
    state: Mutex<MemoryNonceState>,
}

impl PostgresWalletNonceAuthority {
    /// Creates one exact tenant/sender/nonce-domain test authority.
    pub fn new(domain: WalletNonceDomain, first_nonce: u64) -> Result<Self> {
        domain.validate()?;
        Ok(Self {
            domain,
            state: Mutex::new(MemoryNonceState {
                next_nonce: first_nonce,
                operations: BTreeMap::new(),
            }),
        })
    }

    /// Returns the public nonce domain.
    pub const fn domain(&self) -> &WalletNonceDomain {
        &self.domain
    }

    /// Reserves one nonce idempotently.
    pub fn reserve(&self, operation_key: StableId) -> Result<u64> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WalletAuthorityError::Invalid)?;
        if let Some((nonce, _)) = state.operations.get(&operation_key) {
            return Ok(*nonce);
        }
        let nonce = state.next_nonce;
        state.next_nonce = state
            .next_nonce
            .checked_add(1)
            .ok_or(WalletAuthorityError::Invalid)?;
        state.operations.insert(operation_key, (nonce, false));
        Ok(nonce)
    }

    /// Marks one operation complete idempotently.
    pub fn complete(&self, operation_key: &StableId) -> Result<u64> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WalletAuthorityError::Invalid)?;
        let (nonce, completed) = state
            .operations
            .get_mut(operation_key)
            .ok_or(WalletAuthorityError::NotActionable)?;
        *completed = true;
        Ok(*nonce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain() -> WalletNonceDomain {
        WalletNonceDomain {
            tenant: TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
            sender: StableId::new("mfm.sender.test").expect("sender"),
            nonce_domain: StableId::new("mfm.nonce-domain.test").expect("nonce domain"),
        }
    }

    #[test]
    fn memory_authority_returns_and_reuses_the_allocated_nonce() {
        let authority = PostgresWalletNonceAuthority::new(domain(), 7).expect("authority");
        let operation = StableId::new("mfm.operation.test").expect("operation");

        assert_eq!(authority.reserve(operation.clone()).expect("reserve"), 7);
        assert_eq!(authority.reserve(operation.clone()).expect("retry"), 7);
        assert_eq!(authority.complete(&operation).expect("complete"), 7);
        assert_eq!(authority.complete(&operation).expect("repeat complete"), 7);
    }

    #[test]
    fn memory_authority_rejects_nonce_overflow_before_inserting() {
        let authority = PostgresWalletNonceAuthority::new(domain(), u64::MAX).expect("authority");
        let operation = StableId::new("mfm.operation.test").expect("operation");

        assert_eq!(
            authority.reserve(operation.clone()),
            Err(WalletAuthorityError::Invalid)
        );
        assert_eq!(
            authority.complete(&operation),
            Err(WalletAuthorityError::NotActionable)
        );
    }
}
