use std::collections::BTreeSet;

use mfm_canonical::PlainCanonicalJsonBytes;
use serde_json::{Map, Value};
use sqlx::{Postgres, Row, Transaction};

use super::{
    audited_sql, default_evidence_contract, model,
    persistence::{
        fold_effect_records, load_effect_records, load_evidence_contract, load_resource_fold,
        load_resource_records, persist_effect_derivations,
    },
    resource_configuration_from_row, BoundResourceIntent, ModelError, PrototypeError, Result,
    StoreInstance,
};

const RESTORE_DOMAIN: &str = "mfm.postgres-executor-authority";
const RESTORE_VERSION: i64 = 1;
const MAX_ENVELOPE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ROWS_PER_SET: usize = 131_072;
const MAX_ROW_BYTES: usize = 9 * 1024 * 1024;
const MAX_REFERENCE_BYTES: usize = 16 * 1024;
const MAX_BINARY_TEXT_BYTES: usize = 8 * 1024 * 1024 + 2;
const MAX_JSON_DEPTH: usize = 4;

const ENVELOPE_KEYS: &[&str] = &[
    "domain",
    "version",
    "instance_identity",
    "evidence_contracts",
    "evidence_objects",
    "effect_records",
    "resource_configurations",
    "resource_records",
];

const IDENTITY_KEYS: &[&str] = &[
    "singleton",
    "lineage_id",
    "instance_id",
    "executor_binding_ref",
    "ledger_generation",
];

const EVIDENCE_CONTRACT_KEYS: &[&str] = &[
    "contract_ref",
    "max_attempts",
    "max_records",
    "max_retained_bytes",
    "completion_reserve_records",
    "completion_reserve_bytes",
];

const EVIDENCE_OBJECT_KEYS: &[&str] = &["object_digest", "opaque_bytes"];

const EFFECT_RECORD_KEYS: &[&str] = &[
    "stream_id",
    "sequence",
    "predecessor_digest",
    "opaque_payload",
    "executor_binding_ref",
    "ledger_generation",
    "effect_key",
    "request_digest",
    "destination_identity",
    "append_request_id",
    "append_predecessor_sequence",
    "append_predecessor_digest",
    "append_purpose",
    "append_candidate_digest",
    "record_kind",
    "resource_ownership_ref",
    "policy_ref",
    "policy_configuration_ref",
    "resource_key_ref",
    "allocation_state_ref",
    "allocation_value",
    "fencing_ref",
    "resource_append_request_id",
    "resource_record_ref",
    "attempt_ordinal",
    "attempt_id",
    "target_operation_ref",
    "observation_outcome",
    "safe_outcome_ref",
    "destination_account",
    "terminal_attempt_id",
    "external_operation_ref",
    "terminal_outcome_ref",
    "terminal_proof_ref",
    "evidence_object_digest",
    "retained_bytes",
    "commit_digest",
    "proof_digest",
];

const RESOURCE_CONFIGURATION_KEYS: &[&str] = &[
    "resource_ownership_ref",
    "resource_key_ref",
    "policy_ref",
    "policy_configuration_kind",
    "finite_allocation_values",
    "account_sequence_initial_value",
    "policy_configuration_ref",
];

const RESOURCE_RECORD_KEYS: &[&str] = &[
    "resource_ownership_ref",
    "policy_ref",
    "policy_configuration_kind",
    "finite_allocation_values",
    "account_sequence_initial_value",
    "policy_configuration_ref",
    "resource_key_ref",
    "sequence",
    "predecessor_ref",
    "append_request_id",
    "append_predecessor_sequence",
    "append_predecessor_ref",
    "linked_effect_stream_id",
    "linked_effect_append_request_id",
    "append_candidate_digest",
    "effect_key",
    "allocation_state_ref",
    "allocation_value",
    "fencing_ref",
    "executor_binding_ref",
    "ledger_generation",
    "record_ref",
    "proof_digest",
];

const EFFECT_RECORD_KINDS: &[&str] = &[
    "effect_bound",
    "resource_allocated",
    "delivery_attempt_authorized",
    "delivery_attempt_observed",
    "terminal_tombstone",
];

const RESOURCE_CONFIGURATION_KINDS: &[&str] =
    &["finite_inventory", "account_sequence", "exclusive"];

const OBSERVATION_OUTCOMES: &[&str] = &["returned", "did_not_enter", "indeterminate"];

const FIXED_DIGEST_FIELDS: &[&str] = &[
    "object_digest",
    "predecessor_digest",
    "effect_key",
    "request_digest",
    "append_predecessor_digest",
    "append_candidate_digest",
    "policy_configuration_ref",
    "allocation_state_ref",
    "fencing_ref",
    "resource_record_ref",
    "attempt_id",
    "safe_outcome_ref",
    "terminal_attempt_id",
    "terminal_outcome_ref",
    "terminal_proof_ref",
    "evidence_object_digest",
    "commit_digest",
    "proof_digest",
    "predecessor_ref",
    "append_predecessor_ref",
    "record_ref",
];

const BINARY_FIELDS: &[&str] = &["opaque_bytes", "opaque_payload"];

#[derive(Debug)]
pub(super) struct DecodedAuthority {
    envelope: Value,
    pub(super) lineage_id: String,
    pub(super) instance_id: String,
    pub(super) executor_binding_ref: String,
    pub(super) ledger_generation: i64,
}

pub(super) async fn export_authority(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<Vec<u8>> {
    let mut envelope = Map::new();
    envelope.insert(
        "domain".to_owned(),
        Value::String(RESTORE_DOMAIN.to_owned()),
    );
    envelope.insert("version".to_owned(), Value::from(RESTORE_VERSION));
    for (envelope_key, table, ordering) in [
        (
            "instance_identity",
            "prototype_instance_identity",
            "singleton",
        ),
        (
            "evidence_contracts",
            "prototype_evidence_contracts",
            "contract_ref",
        ),
        (
            "evidence_objects",
            "prototype_evidence_objects",
            "object_digest",
        ),
        (
            "effect_records",
            "prototype_effect_records",
            "stream_id, sequence",
        ),
        (
            "resource_configurations",
            "prototype_resource_configurations",
            "resource_ownership_ref, resource_key_ref",
        ),
        (
            "resource_records",
            "prototype_resource_records",
            "resource_ownership_ref, resource_key_ref, sequence",
        ),
    ] {
        let sql = format!(
            "SELECT to_jsonb(authority_row) AS authority_row
             FROM {}.{table} AS authority_row
             ORDER BY {ordering}",
            instance.schema.as_str()
        );
        let rows = sqlx::query(audited_sql(sql))
            .fetch_all(&mut **transaction)
            .await?;
        let values = rows
            .into_iter()
            .map(|row| row.try_get::<Value, _>("authority_row"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        envelope.insert(envelope_key.to_owned(), Value::Array(values));
    }
    canonical_encode(&Value::Object(envelope))
}

pub(super) fn decode_authority(bytes: &[u8]) -> Result<DecodedAuthority> {
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)?;
    if canonical.as_bytes() != bytes {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    let envelope: Value =
        serde_json::from_slice(bytes).map_err(|_| PrototypeError::RestoreEnvelopeInvalid)?;
    validate_json_bounds(&envelope, 0)?;
    let object = require_object_keys(&envelope, ENVELOPE_KEYS)?;
    if object.get("domain").and_then(Value::as_str) != Some(RESTORE_DOMAIN)
        || object.get("version").and_then(Value::as_i64) != Some(RESTORE_VERSION)
    {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    validate_rows(object, "instance_identity", IDENTITY_KEYS, Some(1))?;
    validate_rows(
        object,
        "evidence_contracts",
        EVIDENCE_CONTRACT_KEYS,
        Some(1),
    )?;
    validate_rows(object, "evidence_objects", EVIDENCE_OBJECT_KEYS, None)?;
    validate_rows(object, "effect_records", EFFECT_RECORD_KEYS, None)?;
    validate_rows(
        object,
        "resource_configurations",
        RESOURCE_CONFIGURATION_KEYS,
        None,
    )?;
    validate_rows(object, "resource_records", RESOURCE_RECORD_KEYS, None)?;
    validate_closed_tags(object)?;
    validate_binary_fields(object)?;
    validate_effect_append_columns(object)?;
    validate_evidence_contract(object)?;

    let identity = row_set(object, "instance_identity")?
        .first()
        .and_then(Value::as_object)
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
    if identity.get("singleton").and_then(Value::as_bool) != Some(true) {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    let lineage_id = required_bounded_string(identity, "lineage_id")?.to_owned();
    let instance_id = required_bounded_string(identity, "instance_id")?.to_owned();
    let executor_binding_ref =
        required_bounded_string(identity, "executor_binding_ref")?.to_owned();
    let ledger_generation = identity
        .get("ledger_generation")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;

    Ok(DecodedAuthority {
        envelope,
        lineage_id,
        instance_id,
        executor_binding_ref,
        ledger_generation,
    })
}

pub(super) async fn import_authority(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    decoded: &DecodedAuthority,
) -> Result<()> {
    let envelope = decoded
        .envelope
        .as_object()
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
    for (envelope_key, table) in [
        ("instance_identity", "prototype_instance_identity"),
        ("evidence_contracts", "prototype_evidence_contracts"),
        ("evidence_objects", "prototype_evidence_objects"),
        ("effect_records", "prototype_effect_records"),
        (
            "resource_configurations",
            "prototype_resource_configurations",
        ),
        ("resource_records", "prototype_resource_records"),
    ] {
        let sql = format!(
            "INSERT INTO {}.{table}
             SELECT populated.*
             FROM jsonb_populate_record(NULL::{}.{table}, $1::jsonb) AS populated",
            instance.schema.as_str(),
            instance.schema.as_str()
        );
        for row in row_set(envelope, envelope_key)? {
            sqlx::query(audited_sql(sql.clone()))
                .bind(row)
                .execute(&mut **transaction)
                .await?;
        }
    }
    Ok(())
}

pub(super) async fn rebuild_derivations(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<()> {
    load_evidence_contract(transaction, instance).await?;
    let mut effects = Vec::new();
    for stream_id in effect_stream_ids(transaction, instance).await? {
        let records = load_effect_records(transaction, instance, &stream_id).await?;
        let folded = fold_effect_records(transaction, instance, &records).await?;
        let bound = records.first().ok_or(ModelError::EffectIdentity)?;
        let resource_intent = super::decode_bound_resource_intent(&bound.opaque_payload)?;
        validate_resource_intent_configuration(transaction, instance, resource_intent.as_ref())
            .await?;
        insert_effect_intent(transaction, instance, &folded, resource_intent.as_ref()).await?;
        insert_journal_mirror(transaction, instance, &records).await?;
        insert_effect_receipts_and_head(transaction, instance, &folded).await?;
        persist_effect_derivations(transaction, instance, &folded).await?;
        effects.push(folded);
    }

    let mut resources = Vec::new();
    for (resource_ownership_ref, resource_key_ref) in
        resource_stream_keys(transaction, instance).await?
    {
        let records = load_resource_records(
            transaction,
            instance,
            &resource_ownership_ref,
            &resource_key_ref,
        )
        .await?;
        let folded = model::fold_resource(&records)?;
        validate_resource_configuration(transaction, instance, &folded).await?;
        insert_resource_head(transaction, instance, &folded).await?;
        resources.push(folded);
    }
    model::validate_resource_links(&effects, &resources)?;
    validate_derived(transaction, instance).await
}

pub(super) async fn validate_derived(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<()> {
    load_evidence_contract(transaction, instance).await?;
    validate_configuration_catalog(transaction, instance).await?;

    let effect_streams = effect_stream_ids(transaction, instance).await?;
    let mut effects = Vec::with_capacity(effect_streams.len());
    for stream_id in &effect_streams {
        let records = load_effect_records(transaction, instance, stream_id).await?;
        let folded = fold_effect_records(transaction, instance, &records).await?;
        let bound = records.first().ok_or(ModelError::EffectIdentity)?;
        let authenticated_intent = super::decode_bound_resource_intent(&bound.opaque_payload)?;
        validate_resource_intent_configuration(
            transaction,
            instance,
            authenticated_intent.as_ref(),
        )
        .await?;
        let loaded = super::persistence::load_folded_effect(
            transaction,
            instance,
            &folded.projection.effect_id,
        )
        .await?;
        if loaded != folded || !journal_mirror_matches(transaction, instance, stream_id).await? {
            return Err(ModelError::Projection.into());
        }
        effects.push(folded);
    }
    validate_effect_set_counts(transaction, instance, effect_streams.len()).await?;
    validate_evidence_closure(transaction, instance).await?;

    let resource_keys = resource_stream_keys(transaction, instance).await?;
    let mut resources = Vec::with_capacity(resource_keys.len());
    for (resource_ownership_ref, resource_key_ref) in &resource_keys {
        let folded = load_resource_fold(
            transaction,
            instance,
            resource_ownership_ref,
            resource_key_ref,
        )
        .await?
        .ok_or(ModelError::ResourceChain)?;
        validate_resource_configuration(transaction, instance, &folded).await?;
        resources.push(folded);
    }
    let resource_head_count_sql = format!(
        "SELECT COUNT(*) FROM {}.prototype_resource_heads",
        instance.schema.as_str()
    );
    let resource_head_count: i64 = sqlx::query_scalar(audited_sql(resource_head_count_sql))
        .fetch_one(&mut **transaction)
        .await?;
    if usize::try_from(resource_head_count).ok() != Some(resource_keys.len()) {
        return Err(ModelError::Projection.into());
    }
    model::validate_resource_links(&effects, &resources)?;
    Ok(())
}

fn canonical_encode(value: &Value) -> Result<Vec<u8>> {
    let json = serde_json::to_string(value)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)?;
    if canonical.as_bytes().len() > MAX_ENVELOPE_BYTES {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    Ok(canonical.to_vec())
}

fn require_object_keys<'a>(value: &'a Value, expected: &[&str]) -> Result<&'a Map<String, Value>> {
    let object = value
        .as_object()
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    Ok(object)
}

fn row_set<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a [Value]> {
    object
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)
}

fn validate_rows(
    object: &Map<String, Value>,
    key: &str,
    expected_keys: &[&str],
    exact_count: Option<usize>,
) -> Result<()> {
    let rows = row_set(object, key)?;
    if rows.len() > MAX_ROWS_PER_SET || exact_count.is_some_and(|count| rows.len() != count) {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    for row in rows {
        require_object_keys(row, expected_keys)?;
        if serde_json::to_vec(row)?.len() > MAX_ROW_BYTES {
            return Err(PrototypeError::RestoreEnvelopeInvalid);
        }
    }
    Ok(())
}

fn validate_json_bounds(value: &Value, depth: usize) -> Result<()> {
    if depth > MAX_JSON_DEPTH {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    match value {
        Value::Null | Value::Bool(_) => {}
        Value::Number(number) => {
            if number.as_i64().is_none() {
                return Err(PrototypeError::RestoreEnvelopeInvalid);
            }
        }
        Value::String(value) => {
            if value.len() > MAX_BINARY_TEXT_BYTES {
                return Err(PrototypeError::RestoreEnvelopeInvalid);
            }
        }
        Value::Array(values) => {
            if values.len() > MAX_ROWS_PER_SET {
                return Err(PrototypeError::RestoreEnvelopeInvalid);
            }
            for value in values {
                validate_json_bounds(value, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_json_bounds(value, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn required_bounded_string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= MAX_REFERENCE_BYTES)
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)
}

fn validate_closed_tags(object: &Map<String, Value>) -> Result<()> {
    for row in row_set(object, "effect_records")? {
        let row = row
            .as_object()
            .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
        let record_kind = required_bounded_string(row, "record_kind")?;
        if !EFFECT_RECORD_KINDS.contains(&record_kind) {
            return Err(PrototypeError::RestoreEnvelopeInvalid);
        }
        if let Some(outcome) = row.get("observation_outcome").and_then(Value::as_str) {
            if !OBSERVATION_OUTCOMES.contains(&outcome) {
                return Err(PrototypeError::RestoreEnvelopeInvalid);
            }
        }
    }
    for set in ["resource_configurations", "resource_records"] {
        for row in row_set(object, set)? {
            let row = row
                .as_object()
                .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
            let kind = required_bounded_string(row, "policy_configuration_kind")?;
            if !RESOURCE_CONFIGURATION_KINDS.contains(&kind) {
                return Err(PrototypeError::RestoreEnvelopeInvalid);
            }
        }
    }
    Ok(())
}

fn validate_binary_fields(object: &Map<String, Value>) -> Result<()> {
    for set in [
        "evidence_objects",
        "effect_records",
        "resource_configurations",
        "resource_records",
    ] {
        for row in row_set(object, set)? {
            let row = row
                .as_object()
                .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
            for (key, value) in row {
                if value.is_null() {
                    continue;
                }
                if FIXED_DIGEST_FIELDS.contains(&key.as_str()) {
                    validate_bytea(value, Some(32))?;
                } else if BINARY_FIELDS.contains(&key.as_str()) {
                    validate_bytea(value, None)?;
                } else if let Some(text) = value.as_str() {
                    if text.is_empty() || text.len() > MAX_REFERENCE_BYTES {
                        return Err(PrototypeError::RestoreEnvelopeInvalid);
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_effect_append_columns(object: &Map<String, Value>) -> Result<()> {
    for row in row_set(object, "effect_records")? {
        let row = row
            .as_object()
            .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
        let sequence = row
            .get("sequence")
            .and_then(Value::as_i64)
            .filter(|sequence| *sequence > 0)
            .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
        let append_values = [
            row.get("append_request_id"),
            row.get("append_predecessor_sequence"),
            row.get("append_predecessor_digest"),
            row.get("append_purpose"),
            row.get("append_candidate_digest"),
        ];
        let all_null = append_values
            .iter()
            .all(|value| value.is_some_and(|value| value.is_null()));
        let all_present = append_values
            .iter()
            .all(|value| value.is_some_and(|value| !value.is_null()));
        if (sequence == 1 && !all_null)
            || (sequence > 1 && !all_present)
            || (sequence > 1
                && row
                    .get("append_predecessor_sequence")
                    .and_then(Value::as_i64)
                    .is_none_or(|predecessor| predecessor <= 0))
        {
            return Err(PrototypeError::RestoreEnvelopeInvalid);
        }
        if sequence > 1 {
            required_bounded_string(row, "append_request_id")?;
            required_bounded_string(row, "append_purpose")?;
        }
    }
    Ok(())
}

fn validate_bytea(value: &Value, exact_bytes: Option<usize>) -> Result<()> {
    let encoded = value
        .as_str()
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
    let hex = encoded
        .strip_prefix("\\x")
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
    if hex.len() % 2 != 0
        || hex.len() > MAX_BINARY_TEXT_BYTES - 2
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || exact_bytes.is_some_and(|bytes| hex.len() != bytes * 2)
    {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    Ok(())
}

fn validate_evidence_contract(object: &Map<String, Value>) -> Result<()> {
    let row = row_set(object, "evidence_contracts")?
        .first()
        .and_then(Value::as_object)
        .ok_or(PrototypeError::RestoreEnvelopeInvalid)?;
    let expected = default_evidence_contract();
    if row.get("contract_ref").and_then(Value::as_str) != Some(expected.contract_ref.as_str())
        || row.get("max_attempts").and_then(Value::as_i64) != Some(expected.max_attempts)
        || row.get("max_records").and_then(Value::as_i64) != Some(expected.max_records)
        || row.get("max_retained_bytes").and_then(Value::as_i64)
            != Some(expected.max_retained_bytes)
        || row
            .get("completion_reserve_records")
            .and_then(Value::as_i64)
            != Some(expected.completion_reserve_records)
        || row.get("completion_reserve_bytes").and_then(Value::as_i64)
            != Some(expected.completion_reserve_bytes)
    {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    Ok(())
}

async fn effect_stream_ids(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT DISTINCT stream_id
         FROM {}.prototype_effect_records
         ORDER BY stream_id",
        instance.schema.as_str()
    );
    sqlx::query_scalar(audited_sql(sql))
        .fetch_all(&mut **transaction)
        .await
        .map_err(Into::into)
}

async fn resource_stream_keys(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<Vec<(String, String)>> {
    let sql = format!(
        "SELECT DISTINCT resource_ownership_ref, resource_key_ref
         FROM {}.prototype_resource_records
         ORDER BY resource_ownership_ref, resource_key_ref",
        instance.schema.as_str()
    );
    sqlx::query_as(audited_sql(sql))
        .fetch_all(&mut **transaction)
        .await
        .map_err(Into::into)
}

async fn insert_effect_intent(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    folded: &model::FoldedEffect,
    resource_intent: Option<&BoundResourceIntent>,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO {}.prototype_effect_intents
         (effect_id, stream_id, requested_resource_ownership_ref,
          requested_resource_key_ref, requested_policy_ref,
          requested_policy_configuration_ref, semantic_request_digest)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(sql))
        .bind(&folded.projection.effect_id)
        .bind(&folded.projection.stream_id)
        .bind(resource_intent.map(|intent| &intent.resource_ownership_ref))
        .bind(resource_intent.map(|intent| &intent.resource_key_ref))
        .bind(resource_intent.map(|intent| &intent.policy_ref))
        .bind(resource_intent.map(|intent| &intent.policy_configuration_ref))
        .bind(&folded.projection.request_digest)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn insert_journal_mirror(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    records: &[model::EffectRecord],
) -> Result<()> {
    let sql = format!(
        "INSERT INTO {}.prototype_journal_records
         (stream_id, sequence, predecessor_digest, record_kind, opaque_payload,
          commit_digest, executor_binding_ref, ledger_generation,
          evidence_object_digest, proof_digest)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        instance.schema.as_str()
    );
    for record in records {
        sqlx::query(audited_sql(sql.clone()))
            .bind(&record.stream_id)
            .bind(record.sequence)
            .bind(&record.predecessor_digest)
            .bind(record.body.kind())
            .bind(&record.opaque_payload)
            .bind(&record.commit_digest)
            .bind(&record.executor_binding_ref)
            .bind(record.ledger_generation)
            .bind(&record.evidence_object_digest)
            .bind(&record.proof_digest)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

async fn insert_effect_receipts_and_head(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    folded: &model::FoldedEffect,
) -> Result<()> {
    let receipt_sql = format!(
        "INSERT INTO {}.prototype_append_requests
         (stream_id, append_request_id, predecessor_sequence, predecessor_digest,
          purpose, candidate_digest, committed_sequence, committed_digest)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        instance.schema.as_str()
    );
    for receipt in &folded.append_requests {
        sqlx::query(audited_sql(receipt_sql.clone()))
            .bind(&receipt.stream_id)
            .bind(&receipt.append_request_id)
            .bind(receipt.predecessor_sequence)
            .bind(&receipt.predecessor_digest)
            .bind(&receipt.purpose)
            .bind(&receipt.candidate_digest)
            .bind(receipt.committed_sequence)
            .bind(&receipt.committed_digest)
            .execute(&mut **transaction)
            .await?;
    }
    let head_sql = format!(
        "INSERT INTO {}.prototype_journal_heads
         (stream_id, sequence, commit_digest)
         VALUES ($1, $2, $3)",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(head_sql))
        .bind(&folded.frontier.stream_id)
        .bind(folded.frontier.sequence)
        .bind(&folded.frontier.commit_digest)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn insert_resource_head(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    folded: &model::FoldedResource,
) -> Result<()> {
    let head = &folded.head;
    let sql = format!(
        "INSERT INTO {}.prototype_resource_heads
         (resource_ownership_ref, policy_ref, policy_configuration_ref,
          resource_key_ref, sequence, record_ref)
         VALUES ($1, $2, $3, $4, $5, $6)",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(sql))
        .bind(&head.resource_ownership_ref)
        .bind(&head.policy_ref)
        .bind(&head.policy_configuration_ref)
        .bind(&head.resource_key_ref)
        .bind(head.sequence)
        .bind(&head.record_ref)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn validate_configuration_catalog(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<()> {
    let sql = format!(
        "SELECT *
         FROM {}.prototype_resource_configurations
         ORDER BY resource_ownership_ref, resource_key_ref",
        instance.schema.as_str()
    );
    for row in sqlx::query(audited_sql(sql))
        .fetch_all(&mut **transaction)
        .await?
    {
        let configuration = resource_configuration_from_row(&row)?;
        let policy_ref: String = row.try_get("policy_ref")?;
        let configuration_ref: Vec<u8> = row.try_get("policy_configuration_ref")?;
        if policy_ref != configuration.policy_ref()
            || configuration_ref != model::resource_policy_configuration_ref(&configuration)
        {
            return Err(ModelError::ResourcePolicy.into());
        }
    }
    Ok(())
}

async fn validate_resource_configuration(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    folded: &model::FoldedResource,
) -> Result<()> {
    let sql = format!(
        "SELECT *
         FROM {}.prototype_resource_configurations
         WHERE resource_ownership_ref = $1 AND resource_key_ref = $2",
        instance.schema.as_str()
    );
    let row = sqlx::query(audited_sql(sql))
        .bind(&folded.head.resource_ownership_ref)
        .bind(&folded.head.resource_key_ref)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ModelError::ResourcePolicy)?;
    let configuration = resource_configuration_from_row(&row)?;
    let first = folded.records.first().ok_or(ModelError::ResourceChain)?;
    if configuration != first.policy_configuration
        || row.try_get::<String, _>("policy_ref")? != first.policy_ref
        || row.try_get::<Vec<u8>, _>("policy_configuration_ref")? != first.policy_configuration_ref
    {
        return Err(ModelError::ResourcePolicy.into());
    }
    Ok(())
}

async fn validate_resource_intent_configuration(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    intent: Option<&BoundResourceIntent>,
) -> Result<()> {
    let Some(intent) = intent else {
        return Ok(());
    };
    let sql = format!(
        "SELECT policy_ref, policy_configuration_ref
         FROM {}.prototype_resource_configurations
         WHERE resource_ownership_ref = $1 AND resource_key_ref = $2",
        instance.schema.as_str()
    );
    let row = sqlx::query(audited_sql(sql))
        .bind(&intent.resource_ownership_ref)
        .bind(&intent.resource_key_ref)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ModelError::ResourcePolicy)?;
    if row.try_get::<String, _>("policy_ref")? != intent.policy_ref
        || row.try_get::<Vec<u8>, _>("policy_configuration_ref")? != intent.policy_configuration_ref
    {
        return Err(ModelError::ResourcePolicy.into());
    }
    Ok(())
}

async fn journal_mirror_matches(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    stream_id: &str,
) -> Result<bool> {
    let sql = format!(
        "SELECT NOT EXISTS (
           (
             SELECT stream_id, sequence, predecessor_digest, record_kind,
                    opaque_payload, commit_digest, executor_binding_ref,
                    ledger_generation, evidence_object_digest, proof_digest
             FROM {schema}.prototype_effect_records
             WHERE stream_id = $1
             EXCEPT ALL
             SELECT stream_id, sequence, predecessor_digest, record_kind,
                    opaque_payload, commit_digest, executor_binding_ref,
                    ledger_generation, evidence_object_digest, proof_digest
             FROM {schema}.prototype_journal_records
             WHERE stream_id = $1
           )
           UNION ALL
           (
             SELECT stream_id, sequence, predecessor_digest, record_kind,
                    opaque_payload, commit_digest, executor_binding_ref,
                    ledger_generation, evidence_object_digest, proof_digest
             FROM {schema}.prototype_journal_records
             WHERE stream_id = $1
             EXCEPT ALL
             SELECT stream_id, sequence, predecessor_digest, record_kind,
                    opaque_payload, commit_digest, executor_binding_ref,
                    ledger_generation, evidence_object_digest, proof_digest
             FROM {schema}.prototype_effect_records
             WHERE stream_id = $1
           )
         )",
        schema = instance.schema.as_str()
    );
    sqlx::query_scalar(audited_sql(sql))
        .bind(stream_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(Into::into)
}

async fn validate_effect_set_counts(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    expected_streams: usize,
) -> Result<()> {
    let expected =
        i64::try_from(expected_streams).map_err(|_| PrototypeError::RestoreEnvelopeInvalid)?;
    for table in [
        "prototype_effect_intents",
        "prototype_effects",
        "prototype_effect_frontiers",
        "prototype_journal_heads",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {}.{table}", instance.schema.as_str());
        let count: i64 = sqlx::query_scalar(audited_sql(sql))
            .fetch_one(&mut **transaction)
            .await?;
        if count != expected {
            return Err(ModelError::Projection.into());
        }
    }
    let orphan_receipt_sql = format!(
        "SELECT EXISTS (
           SELECT 1
           FROM {schema}.prototype_append_requests AS receipt
           LEFT JOIN {schema}.prototype_effect_records AS authority
             ON authority.stream_id = receipt.stream_id
                AND authority.sequence = receipt.committed_sequence
           WHERE authority.stream_id IS NULL
         )",
        schema = instance.schema.as_str()
    );
    let orphan_journal_sql = format!(
        "SELECT EXISTS (
           SELECT 1
           FROM {schema}.prototype_journal_records AS mirror
           LEFT JOIN {schema}.prototype_effect_records AS authority
             ON authority.stream_id = mirror.stream_id
                AND authority.sequence = mirror.sequence
           WHERE authority.stream_id IS NULL
         )",
        schema = instance.schema.as_str()
    );
    for sql in [orphan_receipt_sql, orphan_journal_sql] {
        if sqlx::query_scalar::<_, bool>(audited_sql(sql))
            .fetch_one(&mut **transaction)
            .await?
        {
            return Err(ModelError::Projection.into());
        }
    }
    Ok(())
}

async fn validate_evidence_closure(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<()> {
    let sql = format!(
        "SELECT EXISTS (
           SELECT 1
           FROM {schema}.prototype_effect_records AS authority
           LEFT JOIN {schema}.prototype_evidence_objects AS object
             ON object.object_digest = authority.evidence_object_digest
           WHERE object.object_digest IS NULL
              OR object.opaque_bytes <> authority.opaque_payload
           UNION ALL
           SELECT 1
           FROM {schema}.prototype_evidence_objects AS object
           LEFT JOIN {schema}.prototype_effect_records AS authority
             ON authority.evidence_object_digest = object.object_digest
           WHERE authority.stream_id IS NULL
         )",
        schema = instance.schema.as_str()
    );
    if sqlx::query_scalar::<_, bool>(audited_sql(sql))
        .fetch_one(&mut **transaction)
        .await?
    {
        return Err(ModelError::EffectChain.into());
    }
    Ok(())
}

pub(super) async fn authoritative_heads(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<Vec<(String, i64, Vec<u8>)>> {
    let mut heads = Vec::new();
    for stream_id in effect_stream_ids(transaction, instance).await? {
        let records = load_effect_records(transaction, instance, &stream_id).await?;
        let folded = fold_effect_records(transaction, instance, &records).await?;
        heads.push((
            stream_id,
            folded.frontier.sequence,
            folded.frontier.commit_digest,
        ));
    }
    for (owner, key) in resource_stream_keys(transaction, instance).await? {
        let records = load_resource_records(transaction, instance, &owner, &key).await?;
        let folded = model::fold_resource(&records)?;
        heads.push((
            model::resource_stream_id(&owner, &key),
            folded.head.sequence,
            folded.head.record_ref,
        ));
    }
    let configuration_sql = format!(
        "SELECT resource_ownership_ref, resource_key_ref, policy_configuration_ref
         FROM {}.prototype_resource_configurations
         ORDER BY resource_ownership_ref, resource_key_ref",
        instance.schema.as_str()
    );
    for row in sqlx::query(audited_sql(configuration_sql))
        .fetch_all(&mut **transaction)
        .await?
    {
        let owner: String = row.try_get("resource_ownership_ref")?;
        let key: String = row.try_get("resource_key_ref")?;
        heads.push((
            model::resource_configuration_stream_id(&owner, &key),
            1,
            row.try_get("policy_configuration_ref")?,
        ));
    }
    heads.sort_by(|left, right| left.0.cmp(&right.0));
    let mut seen = BTreeSet::new();
    if heads.iter().any(|head| !seen.insert(head.0.clone())) {
        return Err(PrototypeError::RestoreEnvelopeInvalid);
    }
    Ok(heads)
}

pub(super) fn decoded_identity_matches_instance(
    decoded: &DecodedAuthority,
    instance: &StoreInstance,
) -> bool {
    decoded.lineage_id == instance.lineage_id
        && decoded.instance_id == instance.instance_id
        && decoded.executor_binding_ref == instance.executor_binding_ref
        && decoded.ledger_generation == instance.ledger_generation
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
