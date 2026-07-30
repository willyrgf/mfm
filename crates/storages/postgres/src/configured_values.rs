use mfm_journal::ConfiguredValueBinding;
use mfm_store::{
    AsyncStoreFuture, ConfiguredValueBackend, ConfiguredValueResolveVerifier, StoreError,
    VerifiedConfiguredValue,
};
use sqlx::Row;

use crate::error::{database_error, PostgresStoreError, Result};
use crate::store::PostgresRunJournalBackend;

impl ConfiguredValueBackend for PostgresRunJournalBackend {
    fn backend_resolve_configured_value<'a>(
        &'a self,
        verifier: ConfiguredValueResolveVerifier,
    ) -> AsyncStoreFuture<'a, VerifiedConfiguredValue, Self::Error> {
        Box::pin(async move { resolve(self, verifier).await })
    }
}

async fn resolve(
    store: &PostgresRunJournalBackend,
    verifier: ConfiguredValueResolveVerifier,
) -> Result<VerifiedConfiguredValue> {
    let expected_key = verifier.key().fields()?;
    if &expected_key.store_scope_id != store.store_scope_id() {
        return Err(StoreError::AccessDenied {
            purpose: "resolve_configured_value",
        }
        .into());
    }
    let mut transaction = store
        .writer_pool()
        .begin()
        .await
        .map_err(|error| database_error("begin configured-value resolution", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set configured-value isolation", error))?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set configured-value read-only mode", error))?;
    store
        .pin_transaction_schema(&mut transaction)
        .await
        .map_err(|error| database_error("pin qualified configured-value schema", error))?;
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("assume configured-value application role", error))?;

    let row = sqlx::query(
        "SELECT configured.store_scope_id, configured.tenant_scope_id, \
                configured.entry_point_id, configured.target, configured.artifact_id, \
                configured.content_digest, configured.evidence_hash, \
                configured.canonical_value_ref AS configured_value_ref, \
                configured.canonical_binding, \
                admission.canonical_value_ref AS admitted_value_ref, blob.bytes \
           FROM configured_values AS configured \
           JOIN artifact_admissions AS admission \
             ON admission.canonical_value_ref = configured.canonical_value_ref \
            AND admission.artifact_id = configured.artifact_id \
            AND admission.evidence_hash = configured.evidence_hash \
            AND admission.content_digest = configured.content_digest \
           JOIN artifact_blobs AS blob \
             ON blob.content_digest = configured.content_digest \
          WHERE configured.store_scope_id = $1 \
            AND configured.tenant_scope_id = $2 \
            AND configured.entry_point_id = $3 \
            AND configured.target = $4",
    )
    .bind(expected_key.store_scope_id.as_str())
    .bind(expected_key.tenant_scope_id.as_str())
    .bind(expected_key.entry_point_id.as_str())
    .bind(expected_key.target.as_str())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| database_error("load immutable configured-value binding", error))?
    .ok_or(StoreError::ObjectNotReachable)?;

    let canonical_binding = required_bytes(&row, "canonical_binding")?;
    let binding = ConfiguredValueBinding::strict_decode(&canonical_binding)
        .map_err(|_| PostgresStoreError::Corruption("configured-value binding is invalid"))?;
    let binding_fields = binding.fields()?;
    let key = binding_fields.key.fields()?;
    let value = binding_fields.value_ref.fields()?;
    if key.store_scope_id.as_str() != required_string(&row, "store_scope_id")?
        || key.tenant_scope_id.as_str() != required_string(&row, "tenant_scope_id")?
        || key.entry_point_id.as_str() != required_string(&row, "entry_point_id")?
        || key.target.as_str() != required_string(&row, "target")?
        || value.artifact_id.as_str() != required_string(&row, "artifact_id")?
        || value.content_digest.as_str() != required_string(&row, "content_digest")?
        || value.evidence_hash.as_str() != required_string(&row, "evidence_hash")?
        || binding_fields.value_ref.as_bytes() != required_bytes(&row, "configured_value_ref")?
        || binding_fields.value_ref.as_bytes() != required_bytes(&row, "admitted_value_ref")?
    {
        return Err(PostgresStoreError::Corruption(
            "configured-value routing disagrees with canonical binding",
        ));
    }
    let verified = verifier.complete(binding, required_bytes(&row, "bytes")?)?;
    transaction
        .commit()
        .await
        .map_err(|error| database_error("complete configured-value resolution", error))?;
    Ok(verified)
}

fn required_string(row: &sqlx::postgres::PgRow, column: &str) -> Result<String> {
    row.try_get(column)
        .map_err(|_| PostgresStoreError::Corruption("configured-value text is invalid"))
}

fn required_bytes(row: &sqlx::postgres::PgRow, column: &str) -> Result<Vec<u8>> {
    row.try_get(column)
        .map_err(|_| PostgresStoreError::Corruption("configured-value bytes are invalid"))
}
