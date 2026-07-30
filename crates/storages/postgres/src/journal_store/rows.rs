use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::{
    AppendRequestId, ArtifactId, ContentDigest, ContentRef, FieldPath, JournalCandidateDigest,
    JournalCommitDigest, JournalRecordHash, ObjectEvidenceDigest, RecordId, RunId, SchemaId,
    SpecHash, TenantScopeId,
};
use mfm_journal::{
    ArtifactAdmissionIntent, ArtifactAdmissionMode, AuthorityUse, CandidateRecordEnvelope,
    CommitEnvelope, JournalHead, JournalPredecessor, ObjectPathBinding, RunJournalRecord,
    RunJournalRecordFields, TenantFactCoordinate, TenantFactCoordinateFields, ValueRef,
};
use mfm_store::{CommittedJournalCommit, CommittedJournalRecord, CommittedObject, StoreIdentity};
use sqlx::{PgConnection, Row};

use crate::error::{database_error, PostgresStoreError, Result};

pub(super) struct LoadedRun {
    pub(super) commits: Vec<CommittedJournalCommit>,
    pub(super) objects: Vec<CommittedObject>,
}

pub(super) async fn load_run(
    connection: &mut PgConnection,
    store_identity: &StoreIdentity,
    tenant_scope_id: &TenantScopeId,
    run_id: &RunId,
) -> Result<Option<LoadedRun>> {
    let commit_rows = sqlx::query(
        "SELECT run_sequence::text AS run_sequence, append_request_id, candidate_digest, \
                predecessor_kind, predecessor_run_sequence::text AS predecessor_run_sequence, \
                predecessor_commit_digest, commit_digest, \
                admission_entry_point_operation_id, admission_invocation_identity, \
                tenant_fact_coordinate_kind, tenant_fact_order::text AS tenant_fact_order, \
                record_count, committed_at::text AS committed_at \
           FROM journal_commits \
          WHERE run_id = $1 AND tenant_scope_id = $2 \
          ORDER BY run_sequence",
    )
    .bind(run_id.as_str())
    .bind(tenant_scope_id.as_str())
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("load journal commits", error))?;
    if commit_rows.is_empty() {
        return Ok(None);
    }

    let record_rows = sqlx::query(
        "SELECT run_sequence::text AS run_sequence, tenant_scope_id, \
                fact_order::text AS fact_order, ordinal, record_id, record_schema_id, \
                spec_hash, logical_key, record_hash, canonical_payload, emits_facts \
           FROM journal_records \
          WHERE run_id = $1 \
          ORDER BY run_sequence, ordinal",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("load journal records", error))?;

    let binding_rows = sqlx::query(
        "SELECT run_sequence::text AS run_sequence, record_ordinal, field_path, \
                authority_use, artifact_id, content_digest, evidence_hash, canonical_value_ref \
           FROM commit_artifact_bindings \
          WHERE run_id = $1 \
          ORDER BY run_sequence, record_ordinal, field_path",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("load journal object bindings", error))?;
    let authority_rows = sqlx::query(
        "SELECT authority.run_sequence::text AS run_sequence, authority.artifact_id, \
                authority.content_digest, authority.evidence_hash, authority.admission_mode, \
                authority.canonical_value_ref AS routed_value_ref, \
                admission.evidence_contract_schema_id, \
                admission.evidence_contract_content_digest, \
                admission.canonical_value_ref AS admitted_value_ref \
           FROM commit_object_authorities AS authority \
           JOIN artifact_admissions AS admission \
             ON admission.canonical_value_ref = authority.canonical_value_ref \
            AND admission.artifact_id = authority.artifact_id \
            AND admission.evidence_hash = authority.evidence_hash \
            AND admission.content_digest = authority.content_digest \
          WHERE authority.run_id = $1 \
          ORDER BY authority.run_sequence, authority.artifact_id, authority.evidence_hash, \
                   authority.content_digest, authority.canonical_value_ref",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("load journal object authorities", error))?;

    let objects = load_reachable_objects(connection, run_id).await?;
    let mut records_by_sequence = decode_records(record_rows, tenant_scope_id)?;
    let mut bindings_by_sequence = decode_authorities(authority_rows)?;
    decode_bindings(binding_rows, &mut bindings_by_sequence)?;
    let mut commits = Vec::with_capacity(commit_rows.len());
    let mut route_spec_hashes = Vec::new();

    for row in commit_rows {
        let run_sequence = required_u64(&row, "run_sequence")?;
        let records =
            records_by_sequence
                .remove(&run_sequence)
                .ok_or(PostgresStoreError::Corruption(
                    "journal commit has no retained records",
                ))?;
        let record_count = row
            .try_get::<i16, _>("record_count")
            .ok()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(PostgresStoreError::Corruption(
                "journal record count is invalid",
            ))?;
        if records.len() != record_count {
            return Err(PostgresStoreError::Corruption(
                "journal record count disagrees with retained records",
            ));
        }
        validate_admission_routing(&row, run_sequence, &records)?;
        let mut committed_records = Vec::with_capacity(records.len());
        let mut record_fact_orders = Vec::with_capacity(records.len());
        for record in records {
            route_spec_hashes.push(record.spec_hash);
            record_fact_orders.push(record.fact_order);
            committed_records.push(record.record);
        }

        let (bindings, intents) = bindings_by_sequence
            .remove(&run_sequence)
            .unwrap_or_default()
            .into_parts();
        let predecessor = decode_predecessor(&row, store_identity, run_id)?;
        let coordinate = decode_coordinate(&row, tenant_scope_id)?;
        validate_record_coordinate_routing(&committed_records, &record_fact_orders, &coordinate)?;
        let record_hashes = committed_records
            .iter()
            .map(|record| record.record_hash().clone())
            .collect::<Vec<_>>();
        let append_request_id = parse_required::<AppendRequestId>(&row, "append_request_id")?;
        let candidate_digest = parse_required::<JournalCandidateDigest>(&row, "candidate_digest")?;
        let commit_digest = parse_required::<JournalCommitDigest>(&row, "commit_digest")?;
        let committed_at = required_u64(&row, "committed_at")?;
        let envelope = CommitEnvelope::new(
            store_identity.store_scope_id(),
            run_id,
            run_sequence,
            &predecessor,
            &append_request_id,
            &candidate_digest,
            &commit_digest,
            &coordinate,
            &record_hashes,
            &bindings,
            &intents,
            committed_at,
        )
        .map_err(|_| PostgresStoreError::Corruption("journal commit envelope is invalid"))?;
        commits.push(CommittedJournalCommit::from_persisted(
            envelope,
            committed_records,
        ));
    }
    if !records_by_sequence.is_empty() || !bindings_by_sequence.is_empty() {
        return Err(PostgresStoreError::Corruption(
            "journal child rows have no containing commit",
        ));
    }
    validate_run_spec_routing(&commits, &route_spec_hashes)?;
    Ok(Some(LoadedRun { commits, objects }))
}

fn validate_admission_routing(
    row: &sqlx::postgres::PgRow,
    run_sequence: u64,
    records: &[DecodedRecord],
) -> Result<()> {
    let retained_entry_point = optional_string(row, "admission_entry_point_operation_id")?;
    let retained_invocation = optional_string(row, "admission_invocation_identity")?;
    if run_sequence != 1 {
        if retained_entry_point.is_some() || retained_invocation.is_some() {
            return Err(PostgresStoreError::Corruption(
                "successor commit carries admission routing",
            ));
        }
        return Ok(());
    }

    let admission = records
        .first()
        .ok_or(PostgresStoreError::Corruption(
            "admission commit has no root record",
        ))?
        .record
        .candidate()
        .fields()
        .and_then(|candidate| candidate.payload.fields())
        .map_err(|_| PostgresStoreError::Corruption("admission payload is invalid"))?;
    let RunJournalRecordFields::RunAdmitted(admission) = admission else {
        return Err(PostgresStoreError::Corruption(
            "admission routing does not name an admission record",
        ));
    };
    let admission = admission
        .fields()
        .map_err(|_| PostgresStoreError::Corruption("admission payload is invalid"))?;
    if retained_entry_point.as_deref() != Some(admission.entry_point_operation_id.as_str())
        || retained_invocation.as_deref() != Some(admission.invocation_identity.as_str())
    {
        return Err(PostgresStoreError::Corruption(
            "admission routing disagrees with canonical admission",
        ));
    }
    Ok(())
}

async fn load_reachable_objects(
    connection: &mut PgConnection,
    run_id: &RunId,
) -> Result<Vec<CommittedObject>> {
    let rows = sqlx::query(
        "SELECT DISTINCT admission.artifact_id, admission.evidence_hash, \
                admission.content_digest, admission.schema_id, admission.semantic_type_id, \
                admission.role, admission.byte_length::text AS byte_length, \
                admission.media_type, admission.evidence_contract_schema_id, \
                admission.evidence_contract_content_digest, admission.canonical_value_ref, \
                blob.byte_length::text AS blob_byte_length, blob.bytes \
           FROM commit_object_authorities AS authority \
           JOIN artifact_admissions AS admission \
             ON admission.canonical_value_ref = authority.canonical_value_ref \
            AND admission.artifact_id = authority.artifact_id \
            AND admission.evidence_hash = authority.evidence_hash \
            AND admission.content_digest = authority.content_digest \
           JOIN artifact_blobs AS blob ON blob.content_digest = admission.content_digest \
          WHERE authority.run_id = $1 \
          ORDER BY admission.artifact_id, admission.evidence_hash, admission.content_digest, \
                   admission.canonical_value_ref",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("load reachable journal objects", error))?;
    let mut objects = Vec::with_capacity(rows.len());
    let mut keys = BTreeSet::new();
    for row in rows {
        let evidence = required_bytes(&row, "canonical_value_ref")?;
        let value_ref = ValueRef::strict_decode(&evidence)
            .map_err(|_| PostgresStoreError::Corruption("retained object evidence is invalid"))?;
        let fields = value_ref
            .fields()
            .map_err(|_| PostgresStoreError::Corruption("retained object evidence is invalid"))?;
        let byte_length = required_u64(&row, "byte_length")?;
        let blob_byte_length = required_u64(&row, "blob_byte_length")?;
        if fields.artifact_id.as_str() != required_string(&row, "artifact_id")?
            || fields.evidence_hash.as_str() != required_string(&row, "evidence_hash")?
            || fields.content_digest.as_str() != required_string(&row, "content_digest")?
            || fields.schema_id.as_str() != required_string(&row, "schema_id")?
            || fields.semantic_type_id.as_str() != required_string(&row, "semantic_type_id")?
            || fields.role.as_str() != required_string(&row, "role")?
            || fields.media_type != required_string(&row, "media_type")?
            || fields.evidence_contract_ref.schema_id().as_str()
                != required_string(&row, "evidence_contract_schema_id")?
            || fields.evidence_contract_ref.content_digest().as_str()
                != required_string(&row, "evidence_contract_content_digest")?
            || fields.byte_length != byte_length
            || fields.byte_length != blob_byte_length
        {
            return Err(PostgresStoreError::Corruption(
                "retained object routing disagrees with canonical evidence",
            ));
        }
        let bytes = required_bytes(&row, "bytes")?;
        let object = CommittedObject::from_persisted(value_ref, bytes)
            .map_err(|_| PostgresStoreError::Corruption("retained object bytes are invalid"))?;
        if !keys.insert(object.key().clone()) {
            return Err(PostgresStoreError::Corruption(
                "retained object authority is duplicated",
            ));
        }
        objects.push(object);
    }
    Ok(objects)
}

fn decode_records(
    rows: Vec<sqlx::postgres::PgRow>,
    tenant_scope_id: &TenantScopeId,
) -> Result<BTreeMap<u64, Vec<DecodedRecord>>> {
    let mut grouped = BTreeMap::<u64, Vec<DecodedRecord>>::new();
    for row in rows {
        if required_string(&row, "tenant_scope_id")? != tenant_scope_id.as_str() {
            return Err(PostgresStoreError::Corruption(
                "journal record tenant routing is invalid",
            ));
        }
        let run_sequence = required_u64(&row, "run_sequence")?;
        let ordinal = row
            .try_get::<i32, _>("ordinal")
            .ok()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(PostgresStoreError::Corruption(
                "journal record ordinal is invalid",
            ))?;
        let schema_id = parse_required::<SchemaId>(&row, "record_schema_id")?;
        let logical_key =
            mfm_journal::RecordLogicalKey::strict_decode(&required_bytes(&row, "logical_key")?)
                .map_err(|_| PostgresStoreError::Corruption("journal logical key is invalid"))?;
        let payload = RunJournalRecord::strict_decode(&required_bytes(&row, "canonical_payload")?)
            .map_err(|_| PostgresStoreError::Corruption("journal payload is invalid"))?;
        let emits_facts = row
            .try_get::<bool, _>("emits_facts")
            .map_err(|_| PostgresStoreError::Corruption("journal fact marker is invalid"))?;
        if payload
            .emits_facts()
            .map_err(|_| PostgresStoreError::Corruption("journal payload is invalid"))?
            != emits_facts
        {
            return Err(PostgresStoreError::Corruption(
                "journal fact marker disagrees with payload",
            ));
        }
        let candidate =
            CandidateRecordEnvelope::new(ordinal, &schema_id, &logical_key, &payload, emits_facts)
                .map_err(|_| {
                    PostgresStoreError::Corruption("journal candidate envelope is invalid")
                })?;
        let record_id = parse_required::<RecordId>(&row, "record_id")?;
        let record_hash = parse_required::<JournalRecordHash>(&row, "record_hash")?;
        let spec_hash = parse_required::<SpecHash>(&row, "spec_hash")?;
        let fact_order = optional_u64(&row, "fact_order")?;
        let record = CommittedJournalRecord::from_persisted(record_id, record_hash, candidate);
        let records = grouped.entry(run_sequence).or_default();
        if usize::try_from(ordinal).ok() != Some(records.len()) {
            return Err(PostgresStoreError::Corruption(
                "journal record ordinals are not dense",
            ));
        }
        records.push(DecodedRecord {
            record,
            spec_hash,
            fact_order,
        });
    }
    Ok(grouped)
}

struct DecodedRecord {
    record: CommittedJournalRecord,
    spec_hash: SpecHash,
    fact_order: Option<u64>,
}

type PersistedAuthorityKey = (Vec<u8>, ArtifactId, ContentDigest, ObjectEvidenceDigest);

#[derive(Default)]
struct DecodedBindings {
    bindings: Vec<ObjectPathBinding>,
    intents: BTreeMap<PersistedAuthorityKey, ArtifactAdmissionIntent>,
    authorities: BTreeMap<PersistedAuthorityKey, (ValueRef, ContentRef)>,
}

impl DecodedBindings {
    fn into_parts(mut self) -> (Vec<ObjectPathBinding>, Vec<ArtifactAdmissionIntent>) {
        self.bindings
            .sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        let mut intents = self.intents.into_values().collect::<Vec<_>>();
        intents.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        (self.bindings, intents)
    }
}

fn decode_authorities(rows: Vec<sqlx::postgres::PgRow>) -> Result<BTreeMap<u64, DecodedBindings>> {
    let mut grouped = BTreeMap::<u64, DecodedBindings>::new();
    for row in rows {
        let run_sequence = required_u64(&row, "run_sequence")?;
        let artifact_id = parse_required::<ArtifactId>(&row, "artifact_id")?;
        let content_digest = parse_required::<ContentDigest>(&row, "content_digest")?;
        let evidence_hash = parse_required::<ObjectEvidenceDigest>(&row, "evidence_hash")?;
        let routed_value_ref = required_bytes(&row, "routed_value_ref")?;
        let admitted_value_ref = required_bytes(&row, "admitted_value_ref")?;
        if routed_value_ref != admitted_value_ref {
            return Err(PostgresStoreError::Corruption(
                "object authority route selects a different full value reference",
            ));
        }
        let value_ref = ValueRef::strict_decode(&admitted_value_ref)
            .map_err(|_| PostgresStoreError::Corruption("object authority is invalid"))?;
        let value = value_ref
            .fields()
            .map_err(|_| PostgresStoreError::Corruption("object authority is invalid"))?;
        let contract_schema = parse_required::<SchemaId>(&row, "evidence_contract_schema_id")?;
        let contract_digest =
            parse_required::<ContentDigest>(&row, "evidence_contract_content_digest")?;
        let evidence_contract_ref = ContentRef::new(contract_schema, contract_digest)
            .map_err(|_| PostgresStoreError::Corruption("object evidence contract is invalid"))?;
        if value.artifact_id != artifact_id
            || value.content_digest != content_digest
            || value.evidence_hash != evidence_hash
            || value.evidence_contract_ref != evidence_contract_ref
        {
            return Err(PostgresStoreError::Corruption(
                "object authority routing disagrees with its full value reference",
            ));
        }
        let mode = match required_string(&row, "admission_mode")?.as_str() {
            "require_existing" => ArtifactAdmissionMode::RequireExisting,
            "admit_or_verify_exact" => ArtifactAdmissionMode::AdmitOrVerifyExact,
            _ => {
                return Err(PostgresStoreError::Corruption(
                    "object admission mode is invalid",
                ));
            }
        };
        let intent = ArtifactAdmissionIntent::new(&value_ref, &evidence_contract_ref, mode)
            .map_err(|_| PostgresStoreError::Corruption("object admission intent is invalid"))?;
        let key = (
            admitted_value_ref,
            artifact_id,
            content_digest,
            evidence_hash,
        );
        let decoded = grouped.entry(run_sequence).or_default();
        if decoded.intents.insert(key.clone(), intent).is_some()
            || decoded
                .authorities
                .insert(key, (value_ref, evidence_contract_ref))
                .is_some()
        {
            return Err(PostgresStoreError::Corruption(
                "object authority route is duplicated",
            ));
        }
    }
    Ok(grouped)
}

fn decode_bindings(
    rows: Vec<sqlx::postgres::PgRow>,
    grouped: &mut BTreeMap<u64, DecodedBindings>,
) -> Result<()> {
    for row in rows {
        let run_sequence = required_u64(&row, "run_sequence")?;
        let record_ordinal = row
            .try_get::<i32, _>("record_ordinal")
            .ok()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(PostgresStoreError::Corruption(
                "object binding record ordinal is invalid",
            ))?;
        let field_path = parse_required::<FieldPath>(&row, "field_path")?;
        let authority_use = match required_string(&row, "authority_use")?.as_str() {
            "preexisting" => AuthorityUse::Preexisting,
            "produced_here" => AuthorityUse::ProducedHere,
            _ => {
                return Err(PostgresStoreError::Corruption(
                    "object binding authority use is invalid",
                ));
            }
        };
        let artifact_id = parse_required::<ArtifactId>(&row, "artifact_id")?;
        let content_digest = parse_required::<ContentDigest>(&row, "content_digest")?;
        let evidence_hash = parse_required::<ObjectEvidenceDigest>(&row, "evidence_hash")?;
        let canonical_value_ref = required_bytes(&row, "canonical_value_ref")?;
        let decoded = grouped
            .get_mut(&run_sequence)
            .ok_or(PostgresStoreError::Corruption(
                "object binding has no containing authority set",
            ))?;
        let (value_ref, evidence_contract_ref) = decoded
            .authorities
            .get(&(
                canonical_value_ref,
                artifact_id,
                content_digest,
                evidence_hash,
            ))
            .ok_or(PostgresStoreError::Corruption(
                "object binding has no exact object authority",
            ))?;
        let binding = ObjectPathBinding::new(
            record_ordinal,
            &field_path,
            authority_use,
            value_ref,
            evidence_contract_ref,
        )
        .map_err(|_| PostgresStoreError::Corruption("object binding is invalid"))?;
        decoded.bindings.push(binding);
    }
    Ok(())
}

fn decode_predecessor(
    row: &sqlx::postgres::PgRow,
    store_identity: &StoreIdentity,
    run_id: &RunId,
) -> Result<JournalPredecessor> {
    let kind = required_string(row, "predecessor_kind")?;
    let digest = required_string(row, "predecessor_commit_digest")?;
    match kind.as_str() {
        "genesis" => {
            let genesis = digest
                .parse()
                .map_err(|_| PostgresStoreError::Corruption("genesis digest is invalid"))?;
            JournalPredecessor::genesis(
                store_identity.store_scope_id(),
                store_identity.store_epoch(),
                run_id,
                &genesis,
            )
            .map_err(|_| PostgresStoreError::Corruption("genesis predecessor is invalid"))
        }
        "journal_head" => {
            let sequence = optional_u64(row, "predecessor_run_sequence")?.ok_or(
                PostgresStoreError::Corruption("journal predecessor sequence is absent"),
            )?;
            let digest = digest.parse::<JournalCommitDigest>().map_err(|_| {
                PostgresStoreError::Corruption("journal predecessor digest is invalid")
            })?;
            let head = JournalHead::new(sequence, &digest)
                .map_err(|_| PostgresStoreError::Corruption("journal predecessor is invalid"))?;
            JournalPredecessor::journal_head(&head)
                .map_err(|_| PostgresStoreError::Corruption("journal predecessor is invalid"))
        }
        _ => Err(PostgresStoreError::Corruption(
            "journal predecessor kind is invalid",
        )),
    }
}

fn decode_coordinate(
    row: &sqlx::postgres::PgRow,
    tenant_scope_id: &TenantScopeId,
) -> Result<TenantFactCoordinate> {
    let kind = required_string(row, "tenant_fact_coordinate_kind")?;
    let order = optional_u64(row, "tenant_fact_order")?;
    match (kind.as_str(), order) {
        ("none", None) => TenantFactCoordinate::none(),
        ("fact_publication", Some(order)) => {
            TenantFactCoordinate::fact_publication(tenant_scope_id, order)
        }
        ("fact_selection_barrier", Some(order)) => {
            TenantFactCoordinate::fact_selection_barrier(tenant_scope_id, order)
        }
        _ => {
            return Err(PostgresStoreError::Corruption(
                "tenant fact coordinate routing is invalid",
            ));
        }
    }
    .map_err(|_| PostgresStoreError::Corruption("tenant fact coordinate is invalid"))
}

fn validate_record_coordinate_routing(
    records: &[CommittedJournalRecord],
    retained_fact_orders: &[Option<u64>],
    coordinate: &TenantFactCoordinate,
) -> Result<()> {
    if records.len() != retained_fact_orders.len() {
        return Err(PostgresStoreError::Corruption(
            "journal record fact routing is incomplete",
        ));
    }
    let expected_order = match coordinate
        .fields()
        .map_err(|_| PostgresStoreError::Corruption("tenant fact coordinate is invalid"))?
    {
        TenantFactCoordinateFields::FactPublication { fact_order, .. } => Some(fact_order),
        TenantFactCoordinateFields::None
        | TenantFactCoordinateFields::FactSelectionBarrier { .. } => None,
    };
    let fields = records
        .iter()
        .map(|record| {
            record
                .candidate()
                .fields()
                .map_err(|_| PostgresStoreError::Corruption("journal candidate is invalid"))
        })
        .collect::<Result<Vec<_>>>()?;
    for (field, retained) in fields.into_iter().zip(retained_fact_orders) {
        let expected = field.emits_facts.then_some(expected_order).flatten();
        if *retained != expected {
            return Err(PostgresStoreError::Corruption(
                "journal record fact routing disagrees with commit coordinate",
            ));
        }
    }
    Ok(())
}

fn validate_run_spec_routing(
    commits: &[CommittedJournalCommit],
    route_spec_hashes: &[SpecHash],
) -> Result<()> {
    let first = commits
        .first()
        .and_then(|commit| commit.records().first())
        .ok_or(PostgresStoreError::Corruption(
            "journal admission root is absent",
        ))?;
    let candidate = first
        .candidate()
        .fields()
        .map_err(|_| PostgresStoreError::Corruption("journal admission record is invalid"))?;
    let spec_hash = match candidate
        .payload
        .fields()
        .map_err(|_| PostgresStoreError::Corruption("journal admission payload is invalid"))?
    {
        RunJournalRecordFields::RunAdmitted(admission) => {
            admission
                .fields()
                .map_err(|_| {
                    PostgresStoreError::Corruption("journal admission payload is invalid")
                })?
                .spec_hash
        }
        _ => {
            return Err(PostgresStoreError::Corruption(
                "journal first record is not an admission",
            ));
        }
    };
    if route_spec_hashes.iter().any(|route| route != &spec_hash) {
        return Err(PostgresStoreError::Corruption(
            "journal record spec routing disagrees with admission",
        ));
    }
    Ok(())
}

fn parse_required<T>(row: &sqlx::postgres::PgRow, column: &str) -> Result<T>
where
    T: std::str::FromStr,
{
    required_string(row, column)?
        .parse()
        .map_err(|_| PostgresStoreError::Corruption("retained typed identity is invalid"))
}

fn required_string(row: &sqlx::postgres::PgRow, column: &str) -> Result<String> {
    row.try_get(column)
        .map_err(|_| PostgresStoreError::Corruption("retained text column is invalid"))
}

fn optional_string(row: &sqlx::postgres::PgRow, column: &str) -> Result<Option<String>> {
    row.try_get(column)
        .map_err(|_| PostgresStoreError::Corruption("retained optional text column is invalid"))
}

fn required_bytes(row: &sqlx::postgres::PgRow, column: &str) -> Result<Vec<u8>> {
    row.try_get(column)
        .map_err(|_| PostgresStoreError::Corruption("retained byte column is invalid"))
}

fn required_u64(row: &sqlx::postgres::PgRow, column: &str) -> Result<u64> {
    required_string(row, column)?
        .parse()
        .map_err(|_| PostgresStoreError::Corruption("retained u64 column is invalid"))
}

fn optional_u64(row: &sqlx::postgres::PgRow, column: &str) -> Result<Option<u64>> {
    row.try_get::<Option<String>, _>(column)
        .map_err(|_| PostgresStoreError::Corruption("retained optional u64 is invalid"))?
        .map(|value| {
            value
                .parse()
                .map_err(|_| PostgresStoreError::Corruption("retained u64 column is invalid"))
        })
        .transpose()
}
