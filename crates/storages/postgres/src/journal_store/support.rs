use std::collections::BTreeMap;

use mfm_canonical::{sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue};
use mfm_ids::{FieldPath, SemanticTypeId};
use mfm_journal::v1::ValueRef;
use mfm_store::v1::{
    AdmittedSupportGraph, PreparedSupportMember, StoreError, SupportGraphAdmissionVerifier,
};
use sqlx::{Postgres, Row, Transaction};

use crate::error::{ambiguous_commit_error, database_error, PostgresStoreError, Result};
use crate::store::QualifiedPostgresStore;

const SUPPORT_LOCK_DOMAIN: &str = "mfm.postgres.qualified-support-lock.v1";

pub(super) async fn admit_graph(
    store: &QualifiedPostgresStore,
    verifier: SupportGraphAdmissionVerifier,
) -> Result<AdmittedSupportGraph> {
    let prepared = verifier.prepared();
    if prepared.store_identity() != store.store_authority_context().store_identity() {
        return Err(StoreError::AccessDenied {
            purpose: "admit_support_graph",
        }
        .into());
    }
    let mut transaction = store
        .writer_pool()
        .begin()
        .await
        .map_err(|error| database_error("begin qualified support admission", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set qualified support isolation", error))?;
    sqlx::query("SET TRANSACTION READ WRITE")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set qualified support read-write mode", error))?;
    store
        .pin_transaction_schema(&mut transaction)
        .await
        .map_err(|error| database_error("pin qualified support schema", error))?;
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("assume qualified support application role", error))?;
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
        .bind(support_lock_key(prepared.qualification_scope_id())?)
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("acquire qualified support lock", error))?;

    let retained =
        load_support_members(&mut transaction, prepared.qualification_scope_id()).await?;
    if retained.is_empty() {
        for member in prepared.members().values() {
            admit_support_member(&mut transaction, prepared.qualification_scope_id(), member)
                .await?;
        }
    } else {
        verify_complete_support_graph(prepared.members(), retained)?;
    }

    match transaction.commit().await {
        Ok(()) => Ok(verifier.complete()),
        Err(error) if ambiguous_commit_error(&error) => Err(PostgresStoreError::OutcomeUnknown),
        Err(error) => Err(database_error("commit qualified support admission", error)),
    }
}

fn support_lock_key(qualification_scope_id: &SemanticTypeId) -> Result<i64> {
    let value = CanonicalValue::object([
        (
            "domain",
            CanonicalValue::String(SUPPORT_LOCK_DOMAIN.to_owned()),
        ),
        (
            "qualification_scope_id",
            CanonicalValue::String(qualification_scope_id.as_str().to_owned()),
        ),
    ])
    .map_err(|_| PostgresStoreError::Corruption("qualified support lock key is invalid"))?;
    let digest = sha256_digest_bytes(CanonicalJsonBytes::from_value(&value).as_bytes());
    let mut key = [0_u8; 8];
    key.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(i64::from_be_bytes(key))
}

struct RetainedSupportMember {
    field_path: FieldPath,
    value_ref: ValueRef,
    bytes: Vec<u8>,
}

async fn load_support_members(
    transaction: &mut Transaction<'_, Postgres>,
    qualification_scope_id: &SemanticTypeId,
) -> Result<Vec<RetainedSupportMember>> {
    let rows = sqlx::query(
        "SELECT support.field_path, support.artifact_id, support.content_digest, \
                support.evidence_hash, \
                support.canonical_value_ref AS routed_value_ref, \
                admission.canonical_value_ref AS admitted_value_ref, blob.bytes, \
                blob.byte_length::text AS blob_byte_length \
           FROM qualified_support_members AS support \
           JOIN artifact_admissions AS admission \
             ON admission.canonical_value_ref = support.canonical_value_ref \
            AND admission.artifact_id = support.artifact_id \
            AND admission.evidence_hash = support.evidence_hash \
            AND admission.content_digest = support.content_digest \
           JOIN artifact_blobs AS blob \
             ON blob.content_digest = support.content_digest \
          WHERE support.qualification_scope_id = $1 \
          ORDER BY support.field_path",
    )
    .bind(qualification_scope_id.as_str())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| database_error("load qualified support graph", error))?;
    rows.into_iter()
        .map(|row| {
            let field_path = row
                .try_get::<String, _>("field_path")
                .map_err(|_| {
                    PostgresStoreError::Corruption("qualified support field path is invalid")
                })?
                .parse::<FieldPath>()
                .map_err(|_| {
                    PostgresStoreError::Corruption("qualified support field path is invalid")
                })?;
            let canonical = row.try_get::<Vec<u8>, _>("routed_value_ref").map_err(|_| {
                PostgresStoreError::Corruption("qualified support value reference is invalid")
            })?;
            let evidence = row
                .try_get::<Vec<u8>, _>("admitted_value_ref")
                .map_err(|_| {
                    PostgresStoreError::Corruption("qualified support object evidence is invalid")
                })?;
            if canonical != evidence {
                return Err(PostgresStoreError::Corruption(
                    "qualified support routing disagrees with object authority",
                ));
            }
            let value_ref = ValueRef::strict_decode(&canonical).map_err(|_| {
                PostgresStoreError::Corruption("qualified support value reference is invalid")
            })?;
            let fields = value_ref.fields()?;
            let bytes = row.try_get::<Vec<u8>, _>("bytes").map_err(|_| {
                PostgresStoreError::Corruption("qualified support bytes are invalid")
            })?;
            let byte_length = row
                .try_get::<String, _>("blob_byte_length")
                .map_err(|_| {
                    PostgresStoreError::Corruption("qualified support byte length is invalid")
                })?
                .parse::<u64>()
                .map_err(|_| {
                    PostgresStoreError::Corruption("qualified support byte length is invalid")
                })?;
            if fields.artifact_id.as_str()
                != row.try_get::<String, _>("artifact_id").map_err(|_| {
                    PostgresStoreError::Corruption("qualified support artifact id is invalid")
                })?
                || fields.content_digest.as_str()
                    != row.try_get::<String, _>("content_digest").map_err(|_| {
                        PostgresStoreError::Corruption(
                            "qualified support content digest is invalid",
                        )
                    })?
                || fields.evidence_hash.as_str()
                    != row.try_get::<String, _>("evidence_hash").map_err(|_| {
                        PostgresStoreError::Corruption("qualified support evidence hash is invalid")
                    })?
                || fields.byte_length != byte_length
                || fields.byte_length
                    != u64::try_from(bytes.len()).map_err(|_| {
                        PostgresStoreError::Corruption("qualified support byte length is invalid")
                    })?
            {
                return Err(PostgresStoreError::Corruption(
                    "qualified support routing is inconsistent",
                ));
            }
            Ok(RetainedSupportMember {
                field_path,
                value_ref,
                bytes,
            })
        })
        .collect()
}

fn verify_complete_support_graph(
    prepared: &BTreeMap<FieldPath, PreparedSupportMember>,
    retained: Vec<RetainedSupportMember>,
) -> Result<()> {
    if prepared.len() != retained.len() {
        return Err(StoreError::InvalidObjectAuthority {
            message: "qualified support graph member set differs from retained authority",
        }
        .into());
    }
    for retained in retained {
        let member =
            prepared
                .get(&retained.field_path)
                .ok_or(StoreError::InvalidObjectAuthority {
                    message: "qualified support graph contains a missing or extra field path",
                })?;
        if member.value_ref().as_bytes() != retained.value_ref.as_bytes()
            || member.bytes() != retained.bytes
        {
            return Err(StoreError::ObjectAuthorityConflict {
                artifact_id: member.value_ref().fields()?.artifact_id,
            }
            .into());
        }
    }
    Ok(())
}

async fn admit_support_member(
    transaction: &mut Transaction<'_, Postgres>,
    qualification_scope_id: &SemanticTypeId,
    member: &PreparedSupportMember,
) -> Result<()> {
    let fields = member.value_ref().fields()?;
    sqlx::query(
        "INSERT INTO artifact_blobs (content_digest, byte_length, bytes) \
         VALUES ($1, $2::numeric, $3) \
         ON CONFLICT (content_digest) DO NOTHING",
    )
    .bind(fields.content_digest.as_str())
    .bind(fields.byte_length.to_string())
    .bind(member.bytes())
    .execute(&mut **transaction)
    .await
    .map_err(|error| database_error("insert qualified support blob", error))?;
    let blob = sqlx::query(
        "SELECT byte_length::text AS byte_length, bytes \
           FROM artifact_blobs WHERE content_digest = $1",
    )
    .bind(fields.content_digest.as_str())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| database_error("verify qualified support blob", error))?;
    let blob_length = blob
        .try_get::<String, _>("byte_length")
        .map_err(|_| PostgresStoreError::Corruption("qualified support byte length is invalid"))?
        .parse::<u64>()
        .map_err(|_| PostgresStoreError::Corruption("qualified support byte length is invalid"))?;
    let blob_bytes = blob
        .try_get::<Vec<u8>, _>("bytes")
        .map_err(|_| PostgresStoreError::Corruption("qualified support bytes are invalid"))?;
    if blob_length != fields.byte_length || blob_bytes != member.bytes() {
        return Err(StoreError::ObjectContentMismatch {
            artifact_id: fields.artifact_id,
        }
        .into());
    }

    sqlx::query(
        "INSERT INTO artifact_admissions \
            (artifact_id, evidence_hash, content_digest, schema_id, semantic_type_id, role, \
             byte_length, media_type, evidence_contract_schema_id, \
             evidence_contract_content_digest, canonical_value_ref) \
         VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8, $9, $10, $11) \
         ON CONFLICT (canonical_value_ref) DO NOTHING",
    )
    .bind(fields.artifact_id.as_str())
    .bind(fields.evidence_hash.as_str())
    .bind(fields.content_digest.as_str())
    .bind(fields.schema_id.as_str())
    .bind(fields.semantic_type_id.as_str())
    .bind(fields.role.as_str())
    .bind(fields.byte_length.to_string())
    .bind(&fields.media_type)
    .bind(fields.evidence_contract_ref.schema_id().as_str())
    .bind(fields.evidence_contract_ref.content_digest().as_str())
    .bind(member.value_ref().as_bytes())
    .execute(&mut **transaction)
    .await
    .map_err(|error| database_error("insert qualified support object authority", error))?;
    let authority = sqlx::query(
        "SELECT canonical_value_ref \
           FROM artifact_admissions \
          WHERE canonical_value_ref = $1",
    )
    .bind(member.value_ref().as_bytes())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| database_error("verify qualified support object authority", error))?;
    if authority
        .try_get::<Vec<u8>, _>("canonical_value_ref")
        .map_err(|_| {
            PostgresStoreError::Corruption("qualified support object evidence is invalid")
        })?
        != member.value_ref().as_bytes()
    {
        return Err(StoreError::ObjectAuthorityConflict {
            artifact_id: fields.artifact_id,
        }
        .into());
    }

    sqlx::query(
        "INSERT INTO qualified_support_members \
            (qualification_scope_id, field_path, artifact_id, content_digest, evidence_hash, \
             canonical_value_ref) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(qualification_scope_id.as_str())
    .bind(member.field_path().as_str())
    .bind(fields.artifact_id.as_str())
    .bind(fields.content_digest.as_str())
    .bind(fields.evidence_hash.as_str())
    .bind(member.value_ref().as_bytes())
    .execute(&mut **transaction)
    .await
    .map_err(|error| database_error("insert qualified support member", error))?;
    Ok(())
}
