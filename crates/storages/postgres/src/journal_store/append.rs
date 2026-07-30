use std::collections::BTreeMap;

use mfm_canonical::{sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue};
use mfm_ids::{InvocationIdentity, RunId, SpecHash, StableId, TenantScopeId};
use mfm_journal::{
    ArtifactAdmissionMode, JournalHead, JournalPredecessor, JournalPredecessorFields,
    RunJournalRecordFields, TenantFactCoordinate, TenantFactCoordinateFields, TenantFactFrontier,
    ValueRef,
};
use mfm_store::{
    AppendOutcome, AppendRejection, AssignedJournalAppend, CommittedJournalCommit, CommittedObject,
    JournalAppendVerifier, PendingFactScanAttestation, PersistedFactScanAttestation,
    PreparedAppendKind, PreparedObjectGraph, StoreError, StoreIdentity, SuccessorDisposition,
};
use sqlx::{Postgres, Row, Transaction};

use crate::error::{ambiguous_commit_error, database_error, PostgresStoreError, Result};
use crate::store::PostgresRunJournalBackend;
#[cfg(any(test, feature = "parity-tests"))]
use crate::store::TestCommitFailurePoint;

use super::rows::{load_run, LoadedRun};

const ADMISSION_LOCK_DOMAIN: &str = "mfm.postgres.admission-lock.v1";
const RUN_LOCK_DOMAIN: &str = "mfm.postgres.run-lock.v1";

pub(super) async fn append(
    store: &PostgresRunJournalBackend,
    verifier: JournalAppendVerifier,
) -> Result<AppendOutcome> {
    append_verified(store, verifier).await
}

async fn append_verified(
    store: &PostgresRunJournalBackend,
    verifier: JournalAppendVerifier,
) -> Result<AppendOutcome> {
    #[cfg(any(test, feature = "parity-tests"))]
    let test_failure_selector = (verifier.run_id().clone(), verifier.batch_purpose()?);
    stage_blobs(store, verifier.objects()).await?;

    let admission_entry_point = verifier.admission_entry_point_operation_id()?;
    let admission_invocation = verifier.admission_invocation_identity()?;
    let tenant_scope_id = verifier.tenant_scope_id().clone();
    let lock_key = advisory_lock_key(&verifier)?;
    let mut transaction = store
        .writer_pool()
        .begin()
        .await
        .map_err(|error| database_error("begin journal append", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set journal append isolation", error))?;
    sqlx::query("SET TRANSACTION READ WRITE")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set journal append read-write mode", error))?;
    store
        .pin_transaction_schema(&mut transaction)
        .await
        .map_err(|error| database_error("pin qualified journal schema", error))?;
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("assume journal application role", error))?;
    acquire_advisory_lock(&mut transaction, lock_key, "acquire journal append lock").await?;

    let existing_admission = if verifier.kind() == PreparedAppendKind::AdmitRun {
        let existing = find_admission(&mut transaction, &verifier).await?;
        let locked_run_id = existing
            .as_ref()
            .map_or_else(|| verifier.run_id(), |(run_id, _)| run_id);
        #[cfg(any(test, feature = "parity-tests"))]
        store
            .pause_before_admission_run_lock(verifier.append_request_id())
            .await?;
        acquire_advisory_lock(
            &mut transaction,
            run_advisory_lock_key(locked_run_id)?,
            "acquire admission run lock",
        )
        .await?;
        existing
    } else {
        None
    };

    if let Some((existing_run_id, existing_sequence)) = existing_admission {
        let loaded = load_required_run(
            &mut transaction,
            store.store_authority_context().store_identity(),
            verifier.tenant_scope_id(),
            &existing_run_id,
        )
        .await?;
        let commit = commit_at(&loaded, existing_sequence)?;
        let frontier = fact_frontier(
            store.store_authority_context().store_identity(),
            verifier.tenant_scope_id(),
            commit,
        )?;
        verify_existing_fact_scan_attestation(
            &mut transaction,
            verifier.pending_fact_scan_attestation(),
            commit,
        )
        .await?;
        return verifier
            .already_committed_rows(loaded.commits, loaded.objects, existing_sequence, frontier)
            .map_err(Into::into);
    }

    if let Some(existing_sequence) = find_append_request(
        &mut transaction,
        verifier.run_id(),
        verifier.append_request_id(),
    )
    .await?
    {
        let loaded = load_required_run(
            &mut transaction,
            store.store_authority_context().store_identity(),
            verifier.tenant_scope_id(),
            verifier.run_id(),
        )
        .await?;
        let commit = commit_at(&loaded, existing_sequence)?;
        let frontier = fact_frontier(
            store.store_authority_context().store_identity(),
            verifier.tenant_scope_id(),
            commit,
        )?;
        verify_existing_fact_scan_attestation(
            &mut transaction,
            verifier.pending_fact_scan_attestation(),
            commit,
        )
        .await?;
        return verifier
            .already_committed_rows(loaded.commits, loaded.objects, existing_sequence, frontier)
            .map_err(Into::into);
    }

    let loaded = load_run(
        &mut transaction,
        store.store_authority_context().store_identity(),
        verifier.tenant_scope_id(),
        verifier.run_id(),
    )
    .await?;
    let (run_sequence, spec_hash) = match (verifier.kind(), loaded.as_ref()) {
        (PreparedAppendKind::AdmitRun, Some(_)) => {
            return Ok(verifier.rejected(AppendRejection::AdmissionConflict));
        }
        (PreparedAppendKind::AdmitRun, None) => (1, None),
        (
            PreparedAppendKind::CommitTransition
            | PreparedAppendKind::AuthorizeExternalAccess
            | PreparedAppendKind::ObserveExternalAccess,
            None,
        ) => {
            if run_exists(&mut transaction, verifier.run_id()).await? {
                return Err(StoreError::AccessDenied { purpose: "drive" }.into());
            }
            return Err(StoreError::AppendRunNotFound {
                run_id: verifier.run_id().clone(),
            }
            .into());
        }
        (
            PreparedAppendKind::CommitTransition
            | PreparedAppendKind::AuthorizeExternalAccess
            | PreparedAppendKind::ObserveExternalAccess,
            Some(loaded),
        ) => {
            let spec_hash = admission_spec_hash(loaded.commits.first().ok_or(
                PostgresStoreError::Corruption("loaded journal has no admission root"),
            )?)?;
            match verifier
                .classify_successor_rows(loaded.commits.clone(), loaded.objects.clone())?
            {
                SuccessorDisposition::Ready => {}
                SuccessorDisposition::StaleHead { expected, actual } => {
                    let expected =
                        predecessor_head(&expected)?.ok_or(PostgresStoreError::Corruption(
                            "successor candidate has a genesis predecessor",
                        ))?;
                    return Ok(verifier.rejected(AppendRejection::StaleHead { expected, actual }));
                }
                SuccessorDisposition::RunClosed => {
                    return Ok(verifier.rejected(AppendRejection::RunClosed));
                }
            }
            let current = loaded
                .commits
                .last()
                .ok_or(PostgresStoreError::Corruption(
                    "loaded journal has no current head",
                ))?
                .envelope()
                .journal_head()?;
            let next = current
                .fields()?
                .run_sequence
                .checked_add(1)
                .ok_or(StoreError::SequenceOverflow)?;
            (next, Some(spec_hash))
        }
    };

    admit_objects(&mut transaction, verifier.objects()).await?;
    let coordinate = assign_tenant_coordinate(&mut transaction, &verifier).await?;
    let committed_at = database_timestamp_millis(&mut transaction).await?;
    let assigned = verifier.assign(run_sequence, coordinate, committed_at)?;
    let spec_hash = match spec_hash {
        Some(spec_hash) => spec_hash,
        None => admission_spec_hash(assigned.commit())?,
    };
    insert_assigned_append(
        &mut transaction,
        &assigned,
        &tenant_scope_id,
        &spec_hash,
        admission_entry_point.as_ref().map(|value| value.as_str()),
        admission_invocation.as_ref().map(|value| value.as_str()),
    )
    .await?;
    let persisted_attestation = assigned.persisted_fact_scan_attestation()?;
    if let Some(attestation) = &persisted_attestation {
        insert_fact_scan_attestation(&mut transaction, &tenant_scope_id, attestation).await?;
    }

    #[cfg(any(test, feature = "parity-tests"))]
    let injected_failure =
        store.take_commit_failure(&test_failure_selector.0, test_failure_selector.1)?;

    #[cfg(any(test, feature = "parity-tests"))]
    if injected_failure == Some(TestCommitFailurePoint::BeforeCommit) {
        transaction
            .rollback()
            .await
            .map_err(|error| database_error("rollback injected journal append failure", error))?;
        return Err(PostgresStoreError::Database(
            "injected journal append failure before commit",
        ));
    }

    match transaction.commit().await {
        #[cfg(any(test, feature = "parity-tests"))]
        Ok(())
            if injected_failure
                == Some(TestCommitFailurePoint::AfterCommitBeforeAcknowledgement) =>
        {
            Ok(assigned.outcome_unknown())
        }
        Ok(()) => assigned.directly_committed().map_err(Into::into),
        Err(error) if ambiguous_commit_error(&error) => Ok(assigned.outcome_unknown()),
        Err(error) => Err(database_error("commit journal append", error)),
    }
}

async fn acquire_advisory_lock(
    transaction: &mut Transaction<'_, Postgres>,
    lock_key: i64,
    operation: &'static str,
) -> Result<()> {
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut **transaction)
        .await
        .map_err(|error| database_error(operation, error))?;
    Ok(())
}

fn advisory_lock_key(verifier: &JournalAppendVerifier) -> Result<i64> {
    match verifier.kind() {
        PreparedAppendKind::AdmitRun => admission_advisory_lock_key(
            verifier.tenant_scope_id(),
            &verifier.admission_entry_point_operation_id()?.ok_or(
                StoreError::InvalidPreparedAppend {
                    purpose: "admit_run",
                    message: "admission logical key is absent",
                },
            )?,
            &verifier.admission_invocation_identity()?.ok_or(
                StoreError::InvalidPreparedAppend {
                    purpose: "admit_run",
                    message: "admission invocation identity is absent",
                },
            )?,
        ),
        PreparedAppendKind::CommitTransition
        | PreparedAppendKind::AuthorizeExternalAccess
        | PreparedAppendKind::ObserveExternalAccess => run_advisory_lock_key(verifier.run_id()),
    }
}

fn admission_advisory_lock_key(
    tenant_scope_id: &TenantScopeId,
    entry_point_operation_id: &StableId,
    invocation_identity: &InvocationIdentity,
) -> Result<i64> {
    let value = CanonicalValue::object([
        (
            "domain",
            CanonicalValue::String(ADMISSION_LOCK_DOMAIN.to_owned()),
        ),
        (
            "tenant_scope_id",
            CanonicalValue::String(tenant_scope_id.as_str().to_owned()),
        ),
        (
            "entry_point_operation_id",
            CanonicalValue::String(entry_point_operation_id.as_str().to_owned()),
        ),
        (
            "invocation_identity",
            CanonicalValue::String(invocation_identity.as_str().to_owned()),
        ),
    ])
    .map_err(|_| PostgresStoreError::Corruption("admission lock key is invalid"))?;
    digest_lock_key(&value)
}

pub(crate) fn run_advisory_lock_key(run_id: &RunId) -> Result<i64> {
    let value = CanonicalValue::object([
        ("domain", CanonicalValue::String(RUN_LOCK_DOMAIN.to_owned())),
        ("run_id", CanonicalValue::String(run_id.as_str().to_owned())),
    ])
    .map_err(|_| PostgresStoreError::Corruption("run lock key is invalid"))?;
    digest_lock_key(&value)
}

fn digest_lock_key(value: &CanonicalValue) -> Result<i64> {
    let digest = sha256_digest_bytes(CanonicalJsonBytes::from_value(value).as_bytes());
    let mut key = [0_u8; 8];
    key.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(i64::from_be_bytes(key))
}

async fn stage_blobs(
    store: &PostgresRunJournalBackend,
    objects: &PreparedObjectGraph,
) -> Result<()> {
    if objects.payloads().is_empty() {
        return Ok(());
    }
    let mut transaction = store
        .writer_pool()
        .begin()
        .await
        .map_err(|error| database_error("begin artifact blob staging", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set artifact staging isolation", error))?;
    sqlx::query("SET TRANSACTION READ WRITE")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set artifact staging read-write mode", error))?;
    store
        .pin_transaction_schema(&mut transaction)
        .await
        .map_err(|error| database_error("pin qualified artifact schema", error))?;
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("assume artifact application role", error))?;
    for payload in objects.payloads() {
        sqlx::query(
            "INSERT INTO artifact_blobs (content_digest, byte_length, bytes) \
             VALUES ($1, $2::numeric, $3) ON CONFLICT (content_digest) DO NOTHING",
        )
        .bind(payload.content_digest().as_str())
        .bind(payload.bytes().len().to_string())
        .bind(payload.bytes())
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("stage artifact blob", error))?;
        let row = sqlx::query(
            "SELECT byte_length::text AS byte_length, bytes \
               FROM artifact_blobs WHERE content_digest = $1",
        )
        .bind(payload.content_digest().as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| database_error("verify staged artifact blob", error))?;
        let retained_length = parse_u64_column(&row, "byte_length")?;
        let retained_bytes = row
            .try_get::<Vec<u8>, _>("bytes")
            .map_err(|_| PostgresStoreError::Corruption("artifact blob bytes are invalid"))?;
        if retained_length
            != u64::try_from(payload.bytes().len()).map_err(|_| {
                PostgresStoreError::Corruption("artifact payload length cannot be represented")
            })?
            || retained_bytes != payload.bytes()
        {
            return Err(StoreError::ObjectContentMismatch {
                artifact_id: payload
                    .value_refs()
                    .first()
                    .ok_or(PostgresStoreError::Corruption(
                        "artifact payload has no logical authority",
                    ))?
                    .fields()?
                    .artifact_id,
            }
            .into());
        }
    }
    transaction
        .commit()
        .await
        .map_err(|error| database_error("commit artifact blob staging", error))
}

async fn find_admission(
    transaction: &mut Transaction<'_, Postgres>,
    verifier: &JournalAppendVerifier,
) -> Result<Option<(RunId, u64)>> {
    let entry_point = verifier.admission_entry_point_operation_id()?.ok_or(
        StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "admission logical key is absent",
        },
    )?;
    let invocation =
        verifier
            .admission_invocation_identity()?
            .ok_or(StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "admission invocation identity is absent",
            })?;
    let row = sqlx::query(
        "SELECT run_id, run_sequence::text AS run_sequence \
           FROM journal_commits \
          WHERE tenant_scope_id = $1 \
            AND admission_entry_point_operation_id = $2 \
            AND admission_invocation_identity = $3",
    )
    .bind(verifier.tenant_scope_id().as_str())
    .bind(entry_point.as_str())
    .bind(invocation.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| database_error("resolve admission logical key", error))?;
    row.map(|row| {
        let run_id = row
            .try_get::<String, _>("run_id")
            .map_err(|_| PostgresStoreError::Corruption("admission run id is invalid"))?
            .parse()
            .map_err(|_| PostgresStoreError::Corruption("admission run id is invalid"))?;
        Ok((run_id, parse_u64_column(&row, "run_sequence")?))
    })
    .transpose()
}

async fn find_append_request(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    append_request_id: &mfm_ids::AppendRequestId,
) -> Result<Option<u64>> {
    let row = sqlx::query(
        "SELECT run_sequence::text AS run_sequence \
           FROM journal_commits WHERE run_id = $1 AND append_request_id = $2",
    )
    .bind(run_id.as_str())
    .bind(append_request_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| database_error("resolve append request id", error))?;
    row.map(|row| parse_u64_column(&row, "run_sequence"))
        .transpose()
}

async fn run_exists(transaction: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<bool> {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM journal_commits WHERE run_id = $1)")
        .bind(run_id.as_str())
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| database_error("check journal run existence", error))
}

async fn load_required_run(
    transaction: &mut Transaction<'_, Postgres>,
    store_identity: &StoreIdentity,
    tenant_scope_id: &mfm_ids::TenantScopeId,
    run_id: &RunId,
) -> Result<LoadedRun> {
    load_run(transaction, store_identity, tenant_scope_id, run_id)
        .await?
        .ok_or(PostgresStoreError::Corruption(
            "journal index points to an absent run",
        ))
}

fn commit_at(loaded: &LoadedRun, run_sequence: u64) -> Result<&CommittedJournalCommit> {
    let index = usize::try_from(run_sequence.checked_sub(1).ok_or(
        PostgresStoreError::Corruption("journal sequence is not positive"),
    )?)
    .map_err(|_| PostgresStoreError::Corruption("journal sequence cannot be represented"))?;
    loaded
        .commits
        .get(index)
        .ok_or(PostgresStoreError::Corruption(
            "journal idempotency sequence is absent",
        ))
}

fn predecessor_head(predecessor: &JournalPredecessor) -> Result<Option<JournalHead>> {
    match predecessor.fields()? {
        JournalPredecessorFields::Genesis { .. } => Ok(None),
        JournalPredecessorFields::JournalHead(fields) => {
            JournalHead::new(fields.run_sequence, &fields.commit_digest)
                .map(Some)
                .map_err(Into::into)
        }
    }
}

async fn admit_objects(
    transaction: &mut Transaction<'_, Postgres>,
    prepared: &PreparedObjectGraph,
) -> Result<()> {
    let mut payloads = BTreeMap::new();
    for payload in prepared.payloads() {
        for value_ref in payload.value_refs() {
            if payloads
                .insert(value_ref.as_bytes().to_vec(), (value_ref, payload.bytes()))
                .is_some()
            {
                return Err(PostgresStoreError::Corruption(
                    "prepared graph duplicates one logical object authority",
                ));
            }
        }
    }
    for intent in prepared.admission_intents() {
        let fields = intent.fields()?;
        let value_fields = fields.value_ref.fields()?;
        let (value_ref, bytes) = payloads.get(fields.value_ref.as_bytes()).copied().ok_or(
            PostgresStoreError::Corruption("prepared object authority has no payload"),
        )?;
        let retained = load_object_authority(transaction, &fields.value_ref).await?;
        match (fields.mode, retained.as_ref()) {
            (ArtifactAdmissionMode::RequireExisting, Some(existing)) => {
                ensure_object_key(existing, &fields.value_ref)?;
                ensure_object_matches_payload(existing, value_ref, bytes)?;
            }
            (ArtifactAdmissionMode::RequireExisting, None) => {
                return Err(StoreError::MissingObjectAuthority {
                    artifact_id: value_fields.artifact_id,
                }
                .into());
            }
            (ArtifactAdmissionMode::AdmitOrVerifyExact, Some(existing)) => {
                ensure_object_key(existing, &fields.value_ref)?;
                ensure_object_matches_payload(existing, value_ref, bytes)?;
            }
            (ArtifactAdmissionMode::AdmitOrVerifyExact, None) => {
                insert_object_authority(transaction, value_ref, bytes).await?;
                let retained = load_object_authority(transaction, &fields.value_ref)
                    .await?
                    .ok_or(PostgresStoreError::Corruption(
                        "inserted object authority is absent",
                    ))?;
                ensure_object_key(&retained, &fields.value_ref)?;
                ensure_object_matches_payload(&retained, value_ref, bytes)?;
            }
        }
    }
    Ok(())
}

async fn load_object_authority(
    transaction: &mut Transaction<'_, Postgres>,
    value_ref: &mfm_journal::ValueRef,
) -> Result<Option<CommittedObject>> {
    let row = sqlx::query(
        "SELECT admission.canonical_value_ref, blob.bytes \
           FROM artifact_admissions AS admission \
           JOIN artifact_blobs AS blob ON blob.content_digest = admission.content_digest \
          WHERE admission.canonical_value_ref = $1",
    )
    .bind(value_ref.as_bytes())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| database_error("load exact object authority", error))?;
    row.map(|row| {
        let evidence = row
            .try_get::<Vec<u8>, _>("canonical_value_ref")
            .map_err(|_| PostgresStoreError::Corruption("object evidence is invalid"))?;
        let value_ref = mfm_journal::ValueRef::strict_decode(&evidence)
            .map_err(|_| PostgresStoreError::Corruption("object evidence is invalid"))?;
        let bytes = row
            .try_get::<Vec<u8>, _>("bytes")
            .map_err(|_| PostgresStoreError::Corruption("object bytes are invalid"))?;
        CommittedObject::from_persisted(value_ref, bytes).map_err(Into::into)
    })
    .transpose()
}

fn ensure_object_key(object: &CommittedObject, expected: &ValueRef) -> Result<()> {
    if object.value_ref().as_bytes() != expected.as_bytes() {
        return Err(StoreError::ObjectAuthorityConflict {
            artifact_id: expected.fields()?.artifact_id,
        }
        .into());
    }
    Ok(())
}

fn ensure_object_matches_payload(
    object: &CommittedObject,
    value_ref: &mfm_journal::ValueRef,
    bytes: &[u8],
) -> Result<()> {
    if object.value_ref().as_bytes() != value_ref.as_bytes() || object.bytes() != bytes {
        return Err(StoreError::ObjectAuthorityConflict {
            artifact_id: value_ref.fields()?.artifact_id,
        }
        .into());
    }
    Ok(())
}

async fn insert_object_authority(
    transaction: &mut Transaction<'_, Postgres>,
    value_ref: &mfm_journal::ValueRef,
    bytes: &[u8],
) -> Result<()> {
    let fields = value_ref.fields()?;
    if fields.byte_length
        != u64::try_from(bytes.len()).map_err(|_| {
            PostgresStoreError::Corruption("object payload length cannot be represented")
        })?
    {
        return Err(PostgresStoreError::Corruption(
            "object payload length disagrees with full value reference",
        ));
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
    .bind(value_ref.as_bytes())
    .execute(&mut **transaction)
    .await
    .map_err(|error| database_error("admit exact object authority", error))?;
    Ok(())
}

async fn assign_tenant_coordinate(
    transaction: &mut Transaction<'_, Postgres>,
    verifier: &JournalAppendVerifier,
) -> Result<TenantFactCoordinate> {
    let kind = if verifier.emits_facts() {
        Some("fact_publication")
    } else if verifier.reserves_fact_selection_barrier() {
        Some("fact_selection_barrier")
    } else {
        None
    };
    let Some(kind) = kind else {
        return TenantFactCoordinate::none().map_err(Into::into);
    };
    let row =
        sqlx::query("SELECT mfm_assign_tenant_fact_coordinate($1, $2)::text AS assigned_order")
            .bind(verifier.tenant_scope_id().as_str())
            .bind(kind)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|error| database_error("assign tenant fact coordinate", error))?;
    let order = parse_u64_column(&row, "assigned_order")?;
    match kind {
        "fact_publication" => {
            TenantFactCoordinate::fact_publication(verifier.tenant_scope_id(), order)
        }
        "fact_selection_barrier" => {
            TenantFactCoordinate::fact_selection_barrier(verifier.tenant_scope_id(), order)
        }
        _ => unreachable!("coordinate kind is closed above"),
    }
    .map_err(Into::into)
}

async fn database_timestamp_millis(transaction: &mut Transaction<'_, Postgres>) -> Result<u64> {
    let row = sqlx::query(
        "SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::numeric(20, 0)::text \
                AS committed_at",
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| database_error("assign journal commit timestamp", error))?;
    parse_u64_column(&row, "committed_at")
}

async fn insert_assigned_append(
    transaction: &mut Transaction<'_, Postgres>,
    assigned: &AssignedJournalAppend,
    tenant_scope_id: &TenantScopeId,
    spec_hash: &SpecHash,
    admission_entry_point: Option<&str>,
    admission_invocation: Option<&str>,
) -> Result<()> {
    let commit = assigned.commit();
    let envelope = commit.envelope().fields()?;
    let (predecessor_kind, predecessor_sequence, predecessor_digest) =
        match envelope.core.predecessor.fields()? {
            JournalPredecessorFields::Genesis { genesis_digest, .. } => {
                ("genesis", None, genesis_digest.as_str().to_owned())
            }
            JournalPredecessorFields::JournalHead(head) => (
                "journal_head",
                Some(head.run_sequence),
                head.commit_digest.as_str().to_owned(),
            ),
        };
    let (coordinate_kind, fact_order) = match envelope.core.tenant_fact_coordinate.fields()? {
        TenantFactCoordinateFields::None => ("none", None),
        TenantFactCoordinateFields::FactPublication { fact_order, .. } => {
            ("fact_publication", Some(fact_order))
        }
        TenantFactCoordinateFields::FactSelectionBarrier {
            frontier_fact_order,
            ..
        } => ("fact_selection_barrier", Some(frontier_fact_order)),
    };
    let record_count = i16::try_from(commit.records().len())
        .map_err(|_| PostgresStoreError::Corruption("journal record count is too large"))?;

    sqlx::query(
        "INSERT INTO journal_commits \
            (run_id, run_sequence, append_request_id, candidate_digest, predecessor_kind, \
             predecessor_run_sequence, predecessor_commit_digest, commit_digest, \
             tenant_scope_id, admission_entry_point_operation_id, \
             admission_invocation_identity, tenant_fact_coordinate_kind, tenant_fact_order, \
             record_count, committed_at) \
         VALUES \
            ($1, $2::numeric, $3, $4, $5, $6::numeric, $7, $8, $9, $10, $11, $12, \
             $13::numeric, $14, $15::numeric)",
    )
    .bind(envelope.core.run_id.as_str())
    .bind(envelope.core.run_sequence.to_string())
    .bind(envelope.core.append_request_id.as_str())
    .bind(envelope.core.candidate_digest.as_str())
    .bind(predecessor_kind)
    .bind(predecessor_sequence.map(|value| value.to_string()))
    .bind(&predecessor_digest)
    .bind(envelope.commit_digest.as_str())
    .bind(tenant_scope_id.as_str())
    .bind(admission_entry_point)
    .bind(admission_invocation)
    .bind(coordinate_kind)
    .bind(fact_order.map(|value| value.to_string()))
    .bind(record_count)
    .bind(envelope.committed_at.to_string())
    .execute(&mut **transaction)
    .await
    .map_err(|error| database_error("insert journal commit", error))?;

    for record in commit.records() {
        let candidate = record.candidate().fields()?;
        let record_fact_order = candidate.emits_facts.then_some(fact_order).flatten();
        sqlx::query(
            "INSERT INTO journal_records \
                (run_id, run_sequence, tenant_scope_id, fact_order, ordinal, record_id, \
                 record_schema_id, spec_hash, logical_key, record_hash, canonical_payload, \
                 emits_facts) \
             VALUES \
                ($1, $2::numeric, $3, $4::numeric, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(envelope.core.run_id.as_str())
        .bind(envelope.core.run_sequence.to_string())
        .bind(tenant_scope_id.as_str())
        .bind(record_fact_order.map(|value| value.to_string()))
        .bind(i32::try_from(candidate.ordinal).map_err(|_| {
            PostgresStoreError::Corruption("journal record ordinal cannot be represented")
        })?)
        .bind(record.record_id().as_str())
        .bind(candidate.schema_id.as_str())
        .bind(spec_hash.as_str())
        .bind(candidate.logical_key.as_bytes())
        .bind(record.record_hash().as_str())
        .bind(candidate.payload.as_bytes())
        .bind(candidate.emits_facts)
        .execute(&mut **transaction)
        .await
        .map_err(|error| database_error("insert journal record", error))?;
    }

    for intent in &envelope.core.artifact_admission_intents {
        let fields = intent.fields()?;
        let value = fields.value_ref.fields()?;
        sqlx::query(
            "INSERT INTO commit_object_authorities \
                (run_id, run_sequence, artifact_id, content_digest, evidence_hash, \
                 canonical_value_ref, \
                 admission_mode) \
             VALUES ($1, $2::numeric, $3, $4, $5, $6, $7)",
        )
        .bind(envelope.core.run_id.as_str())
        .bind(envelope.core.run_sequence.to_string())
        .bind(value.artifact_id.as_str())
        .bind(value.content_digest.as_str())
        .bind(value.evidence_hash.as_str())
        .bind(fields.value_ref.as_bytes())
        .bind(fields.mode.as_str())
        .execute(&mut **transaction)
        .await
        .map_err(|error| database_error("insert journal object authority route", error))?;
    }
    for binding in &envelope.core.ordered_object_bindings {
        let fields = binding.fields()?;
        let value = fields.value_ref.fields()?;
        sqlx::query(
            "INSERT INTO commit_artifact_bindings \
                (run_id, run_sequence, record_ordinal, field_path, authority_use, artifact_id, \
                 content_digest, evidence_hash, canonical_value_ref) \
             VALUES ($1, $2::numeric, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(envelope.core.run_id.as_str())
        .bind(envelope.core.run_sequence.to_string())
        .bind(i32::try_from(fields.record_ordinal).map_err(|_| {
            PostgresStoreError::Corruption("object binding ordinal cannot be represented")
        })?)
        .bind(fields.field_path.as_str())
        .bind(fields.authority_use.as_str())
        .bind(value.artifact_id.as_str())
        .bind(value.content_digest.as_str())
        .bind(value.evidence_hash.as_str())
        .bind(fields.value_ref.as_bytes())
        .execute(&mut **transaction)
        .await
        .map_err(|error| database_error("insert journal object binding", error))?;
    }
    Ok(())
}

fn admission_spec_hash(commit: &CommittedJournalCommit) -> Result<SpecHash> {
    let first = commit
        .records()
        .first()
        .ok_or(PostgresStoreError::Corruption(
            "admission commit has no root record",
        ))?;
    match first.candidate().fields()?.payload.fields()? {
        RunJournalRecordFields::RunAdmitted(admission) => Ok(admission.fields()?.spec_hash),
        _ => Err(PostgresStoreError::Corruption(
            "admission commit does not carry an admission root",
        )),
    }
}

fn fact_frontier(
    identity: &StoreIdentity,
    tenant_scope_id: &mfm_ids::TenantScopeId,
    commit: &CommittedJournalCommit,
) -> Result<Option<TenantFactFrontier>> {
    let order = match commit
        .envelope()
        .fields()?
        .core
        .tenant_fact_coordinate
        .fields()?
    {
        TenantFactCoordinateFields::None => return Ok(None),
        TenantFactCoordinateFields::FactPublication { fact_order, .. } => fact_order,
        TenantFactCoordinateFields::FactSelectionBarrier {
            frontier_fact_order,
            ..
        } => frontier_fact_order,
    };
    TenantFactFrontier::new(
        identity.store_scope_id(),
        identity.store_epoch(),
        tenant_scope_id,
        order,
    )
    .map(Some)
    .map_err(Into::into)
}

async fn verify_existing_fact_scan_attestation(
    transaction: &mut Transaction<'_, Postgres>,
    pending: Option<&PendingFactScanAttestation>,
    commit: &CommittedJournalCommit,
) -> Result<()> {
    let Some(pending) = pending else {
        return Ok(());
    };
    let expected = pending.persisted_for_commit(commit)?;
    let retained = load_fact_scan_attestation(transaction, expected.authorization_ref().as_bytes())
        .await?
        .ok_or(StoreError::FactScanBindingMismatch)?;
    if retained != expected {
        return Err(StoreError::FactScanBindingMismatch.into());
    }
    Ok(())
}

async fn load_fact_scan_attestation(
    transaction: &mut Transaction<'_, Postgres>,
    authorization_ref: &[u8],
) -> Result<Option<PersistedFactScanAttestation>> {
    let row = sqlx::query(
        "SELECT authorization_ref, attestation_ref, observation_ref, \
                containing_journal_head \
           FROM fact_scan_attestations \
          WHERE authorization_ref = $1",
    )
    .bind(authorization_ref)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| database_error("load exact fact scan attestation", error))?;
    row.map(|row| {
        let authorization_ref = mfm_journal::AuthorizationRef::strict_decode(
            &row.try_get::<Vec<u8>, _>("authorization_ref")
                .map_err(|_| {
                    PostgresStoreError::Corruption("fact scan authorization reference is invalid")
                })?,
        )
        .map_err(|_| {
            PostgresStoreError::Corruption("fact scan authorization reference is invalid")
        })?;
        let attestation_ref = mfm_journal::ValueRef::strict_decode(
            &row.try_get::<Vec<u8>, _>("attestation_ref").map_err(|_| {
                PostgresStoreError::Corruption("fact scan attestation reference is invalid")
            })?,
        )
        .map_err(|_| {
            PostgresStoreError::Corruption("fact scan attestation reference is invalid")
        })?;
        let observation_ref = mfm_journal::ObservationRef::strict_decode(
            &row.try_get::<Vec<u8>, _>("observation_ref").map_err(|_| {
                PostgresStoreError::Corruption("fact scan observation reference is invalid")
            })?,
        )
        .map_err(|_| {
            PostgresStoreError::Corruption("fact scan observation reference is invalid")
        })?;
        let containing_head = mfm_journal::JournalHead::strict_decode(
            &row.try_get::<Vec<u8>, _>("containing_journal_head")
                .map_err(|_| {
                    PostgresStoreError::Corruption("fact scan containing head is invalid")
                })?,
        )
        .map_err(|_| PostgresStoreError::Corruption("fact scan containing head is invalid"))?;
        Ok(PersistedFactScanAttestation::from_persisted(
            authorization_ref,
            attestation_ref,
            observation_ref,
            containing_head,
        ))
    })
    .transpose()
}

async fn insert_fact_scan_attestation(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_scope_id: &TenantScopeId,
    attestation: &PersistedFactScanAttestation,
) -> Result<()> {
    let authorization = attestation.authorization_ref().fields()?;
    let observation = attestation.observation_ref().fields()?;
    let head = attestation.containing_journal_head().fields()?;
    let retained = attestation.attestation_ref().fields()?;
    if authorization.run_id != observation.run_id
        || observation.run_sequence != head.run_sequence
        || observation.ordinal != 0
    {
        return Err(StoreError::FactScanBindingMismatch.into());
    }
    if load_fact_scan_attestation(transaction, attestation.authorization_ref().as_bytes())
        .await?
        .is_some()
    {
        return Err(StoreError::FactScanBindingMismatch.into());
    }
    sqlx::query(
        "INSERT INTO fact_scan_attestations \
            (tenant_scope_id, consuming_run_id, containing_run_sequence, authorization_ref, \
             attestation_artifact_id, attestation_content_digest, \
             attestation_evidence_hash, attestation_ref, observation_ref, \
             containing_journal_head) \
         VALUES ($1, $2, $3::numeric, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(tenant_scope_id.as_str())
    .bind(observation.run_id.as_str())
    .bind(observation.run_sequence.to_string())
    .bind(attestation.authorization_ref().as_bytes())
    .bind(retained.artifact_id.as_str())
    .bind(retained.content_digest.as_str())
    .bind(retained.evidence_hash.as_str())
    .bind(attestation.attestation_ref().as_bytes())
    .bind(attestation.observation_ref().as_bytes())
    .bind(attestation.containing_journal_head().as_bytes())
    .execute(&mut **transaction)
    .await
    .map_err(|error| database_error("insert fact scan attestation", error))?;
    Ok(())
}

fn parse_u64_column(row: &sqlx::postgres::PgRow, column: &str) -> Result<u64> {
    row.try_get::<String, _>(column)
        .map_err(|_| PostgresStoreError::Corruption("retained numeric column is invalid"))?
        .parse()
        .map_err(|_| PostgresStoreError::Corruption("retained numeric column is invalid"))
}
