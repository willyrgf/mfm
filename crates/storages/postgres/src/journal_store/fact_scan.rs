use mfm_ids::RunId;
use mfm_journal::{
    AuthorizationRef, JournalHead, ObservationRef, RunJournalRecordFields, ValueRef,
};
use mfm_store::{
    FactAttestationLoadVerifier, FactScanPage, FactScanPageVerifier, PersistedFactScanAttestation,
    StoreError,
};
use sqlx::Row;

use crate::error::{database_error, PostgresStoreError, Result};
use crate::store::PostgresRunJournalBackend;

use super::rows::{load_run, LoadedRun};

pub(super) async fn scan_page(
    store: &PostgresRunJournalBackend,
    mut verifier: FactScanPageVerifier,
) -> Result<FactScanPage> {
    if verifier.store_identity() != store.store_authority_context().store_identity() {
        return Err(StoreError::AccessDenied {
            purpose: "fact_scan",
        }
        .into());
    }
    let mut transaction = store
        .writer_pool()
        .begin()
        .await
        .map_err(|error| database_error("begin fact scan", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set fact scan isolation", error))?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set fact scan read-only mode", error))?;
    store
        .pin_transaction_schema(&mut transaction)
        .await
        .map_err(|error| database_error("pin qualified fact scan schema", error))?;
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("assume fact scan application role", error))?;

    let rows = sqlx::query(
        "SELECT run_id, tenant_fact_order::text AS fact_order \
           FROM journal_commits \
          WHERE tenant_scope_id = $1 \
            AND tenant_fact_coordinate_kind = 'fact_publication' \
            AND tenant_fact_order >= $2::numeric \
            AND tenant_fact_order <= $3::numeric \
          ORDER BY tenant_fact_order \
          LIMIT $4",
    )
    .bind(verifier.tenant_scope_id().as_str())
    .bind(verifier.next_fact_order().to_string())
    .bind(verifier.frontier_fact_order().to_string())
    .bind(
        i64::try_from(verifier.publication_load_limit())
            .map_err(|_| PostgresStoreError::Corruption("fact scan page limit is invalid"))?,
    )
    .fetch_all(&mut *transaction)
    .await
    .map_err(|error| database_error("load dense fact publication routes", error))?;

    for row in rows {
        let run_id = row
            .try_get::<String, _>("run_id")
            .map_err(|_| PostgresStoreError::Corruption("fact producer run id is invalid"))?
            .parse::<RunId>()
            .map_err(|_| PostgresStoreError::Corruption("fact producer run id is invalid"))?;
        let fact_order = row
            .try_get::<String, _>("fact_order")
            .map_err(|_| PostgresStoreError::Corruption("fact publication order is invalid"))?
            .parse::<u64>()
            .map_err(|_| PostgresStoreError::Corruption("fact publication order is invalid"))?;
        let loaded = load_run(
            &mut transaction,
            store.store_authority_context().store_identity(),
            verifier.tenant_scope_id(),
            &run_id,
        )
        .await?
        .ok_or(PostgresStoreError::Corruption(
            "fact publication route has no producer run",
        ))?;
        let page_full =
            verifier.submit_publication(fact_order, run_id, loaded.commits, loaded.objects)?;
        if page_full {
            break;
        }
    }
    let page = verifier.complete()?;
    transaction
        .commit()
        .await
        .map_err(|error| database_error("complete fact scan read", error))?;
    Ok(page)
}

pub(super) async fn load_attestations(
    store: &PostgresRunJournalBackend,
    verifier: FactAttestationLoadVerifier,
) -> Result<Vec<PersistedFactScanAttestation>> {
    if verifier.store_identity() != store.store_authority_context().store_identity() {
        return Err(StoreError::AccessDenied {
            purpose: "fact_attestations",
        }
        .into());
    }
    let mut transaction = store
        .writer_pool()
        .begin()
        .await
        .map_err(|error| database_error("begin fact attestation load", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set fact attestation isolation", error))?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set fact attestation read-only mode", error))?;
    store
        .pin_transaction_schema(&mut transaction)
        .await
        .map_err(|error| database_error("pin qualified fact attestation schema", error))?;
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("assume fact attestation application role", error))?;

    let loaded = load_run(
        &mut transaction,
        store.store_authority_context().store_identity(),
        verifier.tenant_scope_id(),
        verifier.run_id(),
    )
    .await?
    .ok_or(StoreError::RunNotFound)?;
    let rows = sqlx::query(
        "SELECT attestation.containing_run_sequence::text AS containing_run_sequence, \
                attestation.authorization_ref, attestation.attestation_artifact_id, \
                attestation.attestation_content_digest, \
                attestation.attestation_evidence_hash, attestation.attestation_ref, \
                attestation.observation_ref, attestation.containing_journal_head, \
                commit.commit_digest, record.record_hash \
           FROM fact_scan_attestations AS attestation \
           JOIN journal_commits AS commit \
             ON commit.run_id = attestation.consuming_run_id \
            AND commit.run_sequence = attestation.containing_run_sequence \
           JOIN journal_records AS record \
             ON record.run_id = attestation.consuming_run_id \
            AND record.run_sequence = attestation.containing_run_sequence \
            AND record.ordinal = 0 \
          WHERE attestation.tenant_scope_id = $1 \
            AND attestation.consuming_run_id = $2 \
          ORDER BY attestation.containing_run_sequence, attestation.authorization_ref",
    )
    .bind(verifier.tenant_scope_id().as_str())
    .bind(verifier.run_id().as_str())
    .fetch_all(&mut *transaction)
    .await
    .map_err(|error| database_error("load fact scan attestations", error))?;

    let mut attestations = Vec::with_capacity(rows.len());
    for row in rows {
        attestations.push(decode_attestation_row(&row, verifier.run_id(), &loaded)?);
    }
    transaction
        .commit()
        .await
        .map_err(|error| database_error("complete fact attestation load", error))?;
    Ok(attestations)
}

fn decode_attestation_row(
    row: &sqlx::postgres::PgRow,
    run_id: &RunId,
    loaded: &LoadedRun,
) -> Result<PersistedFactScanAttestation> {
    let containing_sequence = required_u64(row, "containing_run_sequence")?;
    let authorization_ref =
        AuthorizationRef::strict_decode(&required_bytes(row, "authorization_ref")?).map_err(
            |_| PostgresStoreError::Corruption("fact authorization reference is invalid"),
        )?;
    let attestation_ref = ValueRef::strict_decode(&required_bytes(row, "attestation_ref")?)
        .map_err(|_| PostgresStoreError::Corruption("fact attestation reference is invalid"))?;
    let observation_ref =
        ObservationRef::strict_decode(&required_bytes(row, "observation_ref")?)
            .map_err(|_| PostgresStoreError::Corruption("fact observation reference is invalid"))?;
    let containing_head =
        JournalHead::strict_decode(&required_bytes(row, "containing_journal_head")?)
            .map_err(|_| PostgresStoreError::Corruption("fact containing head is invalid"))?;

    let authorization = authorization_ref.fields()?;
    let observation = observation_ref.fields()?;
    let head = containing_head.fields()?;
    let attestation = attestation_ref.fields()?;
    if authorization.run_id != *run_id
        || observation.run_id != *run_id
        || authorization.run_sequence >= observation.run_sequence
        || observation.run_sequence != containing_sequence
        || observation.ordinal != 0
        || head.run_sequence != containing_sequence
        || head.commit_digest.as_str() != required_string(row, "commit_digest")?
        || observation.record_hash.as_str() != required_string(row, "record_hash")?
        || attestation.artifact_id.as_str() != required_string(row, "attestation_artifact_id")?
        || attestation.content_digest.as_str()
            != required_string(row, "attestation_content_digest")?
        || attestation.evidence_hash.as_str() != required_string(row, "attestation_evidence_hash")?
    {
        return Err(PostgresStoreError::Corruption(
            "fact attestation routing disagrees with canonical authority",
        ));
    }

    let authorization_record =
        record_at(loaded, authorization.run_sequence, authorization.ordinal)?;
    if authorization_record.record_hash() != &authorization.record_hash
        || !matches!(
            authorization_record
                .candidate()
                .fields()?
                .payload
                .fields()?,
            RunJournalRecordFields::ExternalAccessAuthorized(_)
        )
    {
        return Err(PostgresStoreError::Corruption(
            "fact authorization route is not authoritative",
        ));
    }
    let observation_record = record_at(loaded, observation.run_sequence, observation.ordinal)?;
    if observation_record.record_hash() != &observation.record_hash
        || !matches!(
            observation_record.candidate().fields()?.payload.fields()?,
            RunJournalRecordFields::ExternalAccessObserved(_)
        )
    {
        return Err(PostgresStoreError::Corruption(
            "fact observation route is not authoritative",
        ));
    }
    if !loaded
        .objects
        .iter()
        .any(|object| object.value_ref() == &attestation_ref)
    {
        return Err(PostgresStoreError::Corruption(
            "fact attestation object authority is absent",
        ));
    }

    Ok(PersistedFactScanAttestation::from_persisted(
        authorization_ref,
        attestation_ref,
        observation_ref,
        containing_head,
    ))
}

fn record_at(
    loaded: &LoadedRun,
    run_sequence: u64,
    ordinal: u32,
) -> Result<&mfm_store::CommittedJournalRecord> {
    let commit_index = usize::try_from(run_sequence.checked_sub(1).ok_or(
        PostgresStoreError::Corruption("fact record sequence is not positive"),
    )?)
    .map_err(|_| PostgresStoreError::Corruption("fact record sequence is invalid"))?;
    let record_index = usize::try_from(ordinal)
        .map_err(|_| PostgresStoreError::Corruption("fact record ordinal is invalid"))?;
    loaded
        .commits
        .get(commit_index)
        .and_then(|commit| commit.records().get(record_index))
        .ok_or(PostgresStoreError::Corruption(
            "fact record route is absent",
        ))
}

fn required_string(row: &sqlx::postgres::PgRow, column: &str) -> Result<String> {
    row.try_get(column)
        .map_err(|_| PostgresStoreError::Corruption("fact routing text is invalid"))
}

fn required_bytes(row: &sqlx::postgres::PgRow, column: &str) -> Result<Vec<u8>> {
    row.try_get(column)
        .map_err(|_| PostgresStoreError::Corruption("fact routing bytes are invalid"))
}

fn required_u64(row: &sqlx::postgres::PgRow, column: &str) -> Result<u64> {
    required_string(row, column)?
        .parse()
        .map_err(|_| PostgresStoreError::Corruption("fact routing number is invalid"))
}
