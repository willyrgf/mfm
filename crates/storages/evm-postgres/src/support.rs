//! Private canonical encoding, pool, and SQL conversion helpers.

use std::str::FromStr;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::EvmWalletReference;
use mfm_ids::{ContentRef, DigestAlgorithm, SchemaId};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgConnection, PgPool};

use crate::error::{PostgresEvmWalletError, Result};
use crate::schema::validate_wallet_schema;

pub(crate) fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let json =
        serde_json::to_string(value).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.as_str().to_owned())
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
}

pub(crate) fn decode_canonical<T: DeserializeOwned>(json: &str) -> Result<T> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(json.as_bytes())
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    serde_json::from_str(canonical.as_str()).map_err(|_| PostgresEvmWalletError::InvalidAuthority)
}

pub(crate) fn reference_text(reference: &EvmWalletReference) -> Result<String> {
    canonical_json(reference)
}

pub(crate) fn parse_reference_text(value: &str) -> Result<EvmWalletReference> {
    decode_canonical(value)
}

pub(crate) fn evidence_reference<T: Serialize>(
    domain: &str,
    value: &T,
) -> Result<EvmWalletReference> {
    let digest = mfm_journal::structured::domain_content_digest(domain, value)
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let schema = SchemaId::new(
        "mfm.evm.wallet-storage-evidence",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.structured-schema.v1:mfm.evm.wallet-storage-evidence:1"),
    )
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let reference =
        ContentRef::new(schema, digest).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    Ok(EvmWalletReference::from_content_ref(reference))
}

pub(crate) async fn open_role_pool(
    database_url: &str,
    expected_schema: &str,
    role: &'static str,
) -> Result<PgPool> {
    validate_identifier(expected_schema)?;
    let schema = expected_schema.to_owned();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _metadata| {
            let schema = schema.clone();
            Box::pin(async move {
                sqlx::query("RESET ROLE").execute(&mut *connection).await?;
                sqlx::query("SELECT pg_catalog.set_config('role', $1, FALSE)")
                    .bind(role)
                    .execute(&mut *connection)
                    .await?;
                sqlx::query(
                    "SELECT pg_catalog.set_config( \
                         'search_path', pg_catalog.format('%I, pg_catalog', $1), FALSE \
                     )",
                )
                .bind(&schema)
                .execute(&mut *connection)
                .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    validate_pool(&pool, expected_schema, role).await?;
    Ok(pool)
}

async fn validate_pool(pool: &PgPool, expected_schema: &str, role: &str) -> Result<()> {
    validate_wallet_schema(pool, expected_schema, role).await
}

pub(crate) async fn session_identity(connection: &mut PgConnection) -> Result<(u32, i32)> {
    let (database_oid, backend_pid) = sqlx::query_as::<_, (i64, i32)>(
        "SELECT (SELECT oid::bigint FROM pg_catalog.pg_database \
                 WHERE datname = current_database()), pg_backend_pid()",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    let database_oid =
        u32::try_from(database_oid).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    Ok((database_oid, backend_pid))
}

pub(crate) async fn transaction_id(connection: &mut PgConnection) -> Result<u32> {
    let value = sqlx::query_scalar::<_, String>("SELECT pg_current_xact_id()::xid::text")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    u32::from_str(&value).map_err(|_| PostgresEvmWalletError::InvalidAuthority)
}

pub(crate) fn validate_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 63
        || !value.chars().enumerate().all(|(index, character)| {
            character == '_'
                || (character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit()))
        })
    {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}
