use mfm_catalog_model::CatalogRef;
use mfm_storage_postgres::{CatalogValueKey, PostgresStore};
use mfm_values::{MfmConfig, ValidatedConfig};

use crate::{AppError, ErrorClass};

/// Resolves and validates one exact catalog reference into its ordinary typed config.
pub async fn resolve_catalog_value<T: MfmConfig>(
    store: &PostgresStore,
    reference: &CatalogRef<T>,
) -> Result<T, AppError> {
    let schema_id = T::schema_id().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "CatalogSchemaInvalid",
            "The catalog value schema is invalid",
        )
    })?;
    let key = CatalogValueKey::new(
        reference.name().as_str(),
        schema_id,
        reference.digest().clone(),
    )
    .map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "CatalogReferenceInvalid",
            "The catalog reference is invalid",
        )
    })?;
    let row = store
        .load_catalog_value(&key)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| {
            AppError::not_found(
                "CatalogValueNotFound",
                "The exact catalog value was not found",
            )
        })?;
    let config: T = serde_json::from_slice(&row.canonical_json).map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "CatalogValueTypeInvalid",
            "The catalog value does not match the requested type",
        )
    })?;
    let validated = ValidatedConfig::new(config).map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "CatalogValueValidationFailed",
            "The catalog value failed semantic validation",
        )
    })?;
    let canonical = validated.canonical_json().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "CatalogValueCanonicalizationFailed",
            "The catalog value could not be canonicalized",
        )
    })?;
    if canonical.as_bytes() != row.canonical_json.as_slice()
        || canonical.content_digest() != *reference.digest()
    {
        return Err(AppError::backend(
            ErrorClass::Internal,
            "CatalogValueCanonicalMismatch",
            "The catalog value failed canonical integrity verification",
        ));
    }
    Ok(validated.into_inner())
}
