use sqlx::{Postgres, Row, Transaction};

use super::{
    audited_sql, model, retain_evidence_object, EffectFrontier, EffectProjection, EffectRecord,
    EffectRecordBody, EvidenceContract, FoldedEffect, FoldedResource, ModelError,
    ResourceAppendIdentity, ResourceHeadProjection, ResourceLink, ResourcePolicyConfiguration,
    ResourceRecord, Result, StoreInstance, Tombstone, COMPLETION_RESERVE_BYTES,
    COMPLETION_RESERVE_RECORDS, EVIDENCE_CONTRACT_REF, MAX_EFFECT_ATTEMPTS, MAX_EFFECT_RECORDS,
    MAX_EFFECT_RETAINED_BYTES,
};
use super::{AppendIdentity, AttemptAuthorization, AttemptObservation, AttemptOutcome};

pub(super) fn default_evidence_contract() -> EvidenceContract {
    EvidenceContract {
        contract_ref: EVIDENCE_CONTRACT_REF.to_owned(),
        max_attempts: MAX_EFFECT_ATTEMPTS,
        max_records: MAX_EFFECT_RECORDS,
        max_retained_bytes: MAX_EFFECT_RETAINED_BYTES,
        completion_reserve_records: COMPLETION_RESERVE_RECORDS,
        completion_reserve_bytes: COMPLETION_RESERVE_BYTES,
    }
}

pub(super) async fn load_evidence_contract(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<EvidenceContract> {
    let sql = format!(
        "SELECT contract_ref, max_attempts, max_records, max_retained_bytes,
                completion_reserve_records, completion_reserve_bytes
         FROM {}.prototype_evidence_contracts
         WHERE contract_ref = $1",
        instance.schema.as_str()
    );
    let row = sqlx::query(audited_sql(sql))
        .bind(EVIDENCE_CONTRACT_REF)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ModelError::EvidenceBounds)?;
    let contract = EvidenceContract {
        contract_ref: row.try_get("contract_ref")?,
        max_attempts: row.try_get("max_attempts")?,
        max_records: row.try_get("max_records")?,
        max_retained_bytes: row.try_get("max_retained_bytes")?,
        completion_reserve_records: row.try_get("completion_reserve_records")?,
        completion_reserve_bytes: row.try_get("completion_reserve_bytes")?,
    };
    if contract != default_evidence_contract() {
        return Err(ModelError::EvidenceBounds.into());
    }
    Ok(contract)
}

#[derive(Debug)]
struct EffectBodyColumns {
    resource_ownership_ref: Option<String>,
    policy_ref: Option<String>,
    policy_configuration_ref: Option<Vec<u8>>,
    resource_key_ref: Option<String>,
    allocation_state_ref: Option<Vec<u8>>,
    allocation_value: Option<i64>,
    fencing_ref: Option<Vec<u8>>,
    resource_append_request_id: Option<String>,
    resource_record_ref: Option<Vec<u8>>,
    attempt_ordinal: Option<i64>,
    attempt_id: Option<Vec<u8>>,
    target_operation_ref: Option<String>,
    observation_outcome: Option<String>,
    safe_outcome_ref: Option<Vec<u8>>,
    destination_account: Option<i64>,
    terminal_attempt_id: Option<Vec<u8>>,
    external_operation_ref: Option<String>,
    terminal_outcome_ref: Option<Vec<u8>>,
    terminal_proof_ref: Option<Vec<u8>>,
}

impl EffectBodyColumns {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self> {
        Ok(Self {
            resource_ownership_ref: row.try_get("resource_ownership_ref")?,
            policy_ref: row.try_get("policy_ref")?,
            policy_configuration_ref: row.try_get("policy_configuration_ref")?,
            resource_key_ref: row.try_get("resource_key_ref")?,
            allocation_state_ref: row.try_get("allocation_state_ref")?,
            allocation_value: row.try_get("allocation_value")?,
            fencing_ref: row.try_get("fencing_ref")?,
            resource_append_request_id: row.try_get("resource_append_request_id")?,
            resource_record_ref: row.try_get("resource_record_ref")?,
            attempt_ordinal: row.try_get("attempt_ordinal")?,
            attempt_id: row.try_get("attempt_id")?,
            target_operation_ref: row.try_get("target_operation_ref")?,
            observation_outcome: row.try_get("observation_outcome")?,
            safe_outcome_ref: row.try_get("safe_outcome_ref")?,
            destination_account: row.try_get("destination_account")?,
            terminal_attempt_id: row.try_get("terminal_attempt_id")?,
            external_operation_ref: row.try_get("external_operation_ref")?,
            terminal_outcome_ref: row.try_get("terminal_outcome_ref")?,
            terminal_proof_ref: row.try_get("terminal_proof_ref")?,
        })
    }

    fn resource_fields_absent(&self) -> bool {
        self.resource_ownership_ref.is_none()
            && self.policy_ref.is_none()
            && self.policy_configuration_ref.is_none()
            && self.resource_key_ref.is_none()
            && self.allocation_state_ref.is_none()
            && self.allocation_value.is_none()
            && self.fencing_ref.is_none()
            && self.resource_append_request_id.is_none()
            && self.resource_record_ref.is_none()
    }

    fn attempt_fields_absent(&self) -> bool {
        self.attempt_ordinal.is_none()
            && self.attempt_id.is_none()
            && self.target_operation_ref.is_none()
            && self.observation_outcome.is_none()
            && self.safe_outcome_ref.is_none()
            && self.destination_account.is_none()
    }

    fn terminal_fields_absent(&self) -> bool {
        self.terminal_attempt_id.is_none()
            && self.external_operation_ref.is_none()
            && self.terminal_outcome_ref.is_none()
            && self.terminal_proof_ref.is_none()
    }
}

fn required<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(|| ModelError::EffectChain.into())
}

fn decode_effect_body(
    record_kind: &str,
    mut columns: EffectBodyColumns,
) -> Result<EffectRecordBody> {
    match record_kind {
        "effect_bound" => {
            if !columns.resource_fields_absent()
                || !columns.attempt_fields_absent()
                || !columns.terminal_fields_absent()
            {
                return Err(ModelError::EffectChain.into());
            }
            Ok(EffectRecordBody::EffectBound)
        }
        "resource_allocated" => {
            if !columns.attempt_fields_absent() || !columns.terminal_fields_absent() {
                return Err(ModelError::EffectChain.into());
            }
            Ok(EffectRecordBody::ResourceAllocated(ResourceLink {
                resource_ownership_ref: required(columns.resource_ownership_ref.take())?,
                policy_ref: required(columns.policy_ref.take())?,
                policy_configuration_ref: required(columns.policy_configuration_ref.take())?,
                resource_key_ref: required(columns.resource_key_ref.take())?,
                typed_allocation_state_ref: required(columns.allocation_state_ref.take())?,
                allocation_value: required(columns.allocation_value.take())?,
                fencing_ref: columns.fencing_ref.take(),
                resource_append_request_id: required(columns.resource_append_request_id.take())?,
                resource_record_ref: required(columns.resource_record_ref.take())?,
            }))
        }
        "delivery_attempt_authorized" => {
            if !columns.resource_fields_absent()
                || columns.observation_outcome.is_some()
                || columns.safe_outcome_ref.is_some()
                || columns.destination_account.is_some()
                || !columns.terminal_fields_absent()
            {
                return Err(ModelError::EffectChain.into());
            }
            Ok(EffectRecordBody::DeliveryAttemptAuthorized(
                AttemptAuthorization {
                    attempt_ordinal: required(columns.attempt_ordinal.take())?,
                    attempt_id: required(columns.attempt_id.take())?,
                    target_operation_ref: required(columns.target_operation_ref.take())?,
                },
            ))
        }
        "delivery_attempt_observed" => {
            if !columns.resource_fields_absent()
                || columns.attempt_ordinal.is_some()
                || columns.target_operation_ref.is_some()
                || !columns.terminal_fields_absent()
            {
                return Err(ModelError::EffectChain.into());
            }
            let safe_outcome_ref = required(columns.safe_outcome_ref.take())?;
            let outcome = match required(columns.observation_outcome.take())?.as_str() {
                "returned" => AttemptOutcome::Returned {
                    safe_outcome_ref,
                    destination_account: required(columns.destination_account.take())?,
                },
                "did_not_enter" => {
                    if columns.destination_account.is_some() {
                        return Err(ModelError::EffectChain.into());
                    }
                    AttemptOutcome::DidNotEnter { safe_outcome_ref }
                }
                "indeterminate" => {
                    if columns.destination_account.is_some() {
                        return Err(ModelError::EffectChain.into());
                    }
                    AttemptOutcome::Indeterminate { safe_outcome_ref }
                }
                _ => return Err(ModelError::EffectChain.into()),
            };
            Ok(EffectRecordBody::DeliveryAttemptObserved(
                AttemptObservation {
                    attempt_id: required(columns.attempt_id.take())?,
                    outcome,
                },
            ))
        }
        "terminal_tombstone" => {
            if !columns.resource_fields_absent() || !columns.attempt_fields_absent() {
                return Err(ModelError::EffectChain.into());
            }
            Ok(EffectRecordBody::TerminalTombstone(Tombstone {
                terminal_attempt_id: required(columns.terminal_attempt_id.take())?,
                external_operation_ref: required(columns.external_operation_ref.take())?,
                terminal_outcome_ref: required(columns.terminal_outcome_ref.take())?,
                terminal_proof_ref: required(columns.terminal_proof_ref.take())?,
            }))
        }
        _ => Err(ModelError::EffectChain.into()),
    }
}

pub(super) async fn load_effect_records(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    stream_id: &str,
) -> Result<Vec<EffectRecord>> {
    let sql = format!(
        "SELECT *
         FROM {}.prototype_effect_records
         WHERE stream_id = $1
         ORDER BY sequence",
        instance.schema.as_str()
    );
    let rows = sqlx::query(audited_sql(sql))
        .bind(stream_id)
        .fetch_all(&mut **transaction)
        .await?;
    rows.into_iter()
        .map(|row| {
            let append_columns = (
                row.try_get::<Option<String>, _>("append_request_id")?,
                row.try_get::<Option<i64>, _>("append_predecessor_sequence")?,
                row.try_get::<Option<Vec<u8>>, _>("append_predecessor_digest")?,
                row.try_get::<Option<String>, _>("append_purpose")?,
                row.try_get::<Option<Vec<u8>>, _>("append_candidate_digest")?,
            );
            let append = match append_columns {
                (None, None, None, None, None) => None,
                (
                    Some(append_request_id),
                    Some(expected_predecessor_sequence),
                    Some(expected_predecessor_digest),
                    Some(purpose),
                    Some(candidate_digest),
                ) => Some(AppendIdentity {
                    append_request_id,
                    expected_predecessor_sequence,
                    expected_predecessor_digest,
                    purpose,
                    candidate_digest,
                }),
                _ => return Err(ModelError::AppendIdentity.into()),
            };
            let body = decode_effect_body(
                row.try_get::<String, _>("record_kind")?.as_str(),
                EffectBodyColumns::from_row(&row)?,
            )?;
            Ok(EffectRecord {
                stream_id: row.try_get("stream_id")?,
                sequence: row.try_get("sequence")?,
                predecessor_digest: row.try_get("predecessor_digest")?,
                opaque_payload: row.try_get("opaque_payload")?,
                executor_binding_ref: row.try_get("executor_binding_ref")?,
                ledger_generation: row.try_get("ledger_generation")?,
                effect_key: row.try_get("effect_key")?,
                request_digest: row.try_get("request_digest")?,
                destination_identity: row.try_get("destination_identity")?,
                append,
                body,
                evidence_object_digest: row.try_get("evidence_object_digest")?,
                retained_bytes: row.try_get("retained_bytes")?,
                commit_digest: row.try_get("commit_digest")?,
                proof_digest: row.try_get("proof_digest")?,
            })
        })
        .collect()
}

pub(super) async fn fold_effect_records(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    records: &[EffectRecord],
) -> Result<FoldedEffect> {
    let contract = load_evidence_contract(transaction, instance).await?;
    let folded = model::fold_effect(records, &contract)?;
    let bound = records.first().ok_or(ModelError::EffectIdentity)?;
    let intent = super::decode_bound_resource_intent(&bound.opaque_payload)?;
    model::validate_bound_resource_intent(&folded, intent.as_ref())?;
    Ok(folded)
}

fn decode_effect_projection(row: &sqlx::postgres::PgRow) -> Result<EffectProjection> {
    let request_digest: Vec<u8> = row.try_get("request_digest")?;
    let semantic_request_digest: Vec<u8> = row.try_get("semantic_request_digest")?;
    if request_digest != semantic_request_digest {
        return Err(ModelError::Projection.into());
    }
    Ok(EffectProjection {
        effect_id: row.try_get("effect_id")?,
        stream_id: row.try_get("stream_id")?,
        effect_key: row.try_get("effect_key")?,
        request_digest,
        destination_identity: row.try_get("destination_identity")?,
        state: row.try_get("state")?,
        resource_ownership_ref: row.try_get("resource_ownership_ref")?,
        policy_ref: row.try_get("policy_ref")?,
        policy_configuration_ref: row.try_get("policy_configuration_ref")?,
        resource_key_ref: row.try_get("resource_id")?,
        allocation_state_ref: row.try_get("allocation_state_ref")?,
        allocation_value: row.try_get("allocation_value")?,
        fencing_ref: row.try_get("fencing_ref")?,
        resource_append_request_id: row.try_get("resource_append_request_id")?,
        resource_record_ref: row.try_get("resource_record_ref")?,
        destination_account: row.try_get("destination_account")?,
        authorization_count: row.try_get("authorization_count")?,
        observation_count: row.try_get("observation_count")?,
        late_observation_count: row.try_get("late_observation_count")?,
    })
}

fn decode_effect_frontier(row: &sqlx::postgres::PgRow) -> Result<EffectFrontier> {
    Ok(EffectFrontier {
        stream_id: row.try_get("stream_id")?,
        executor_binding_ref: row.try_get("executor_binding_ref")?,
        ledger_generation: row.try_get("ledger_generation")?,
        effect_key: row.try_get("effect_key")?,
        request_digest: row.try_get("request_digest")?,
        destination_identity: row.try_get("destination_identity")?,
        sequence: row.try_get("sequence")?,
        predecessor_digest: row.try_get("predecessor_digest")?,
        evidence_object_digest: row.try_get("evidence_object_digest")?,
        retained_bytes: row.try_get("retained_bytes")?,
        commit_digest: row.try_get("commit_digest")?,
        proof_digest: row.try_get("proof_digest")?,
    })
}

pub(super) async fn validate_effect_derivations(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    folded: &FoldedEffect,
) -> Result<()> {
    let projection_sql = format!(
        "SELECT *
         FROM {}.prototype_effects
         WHERE effect_id = $1",
        instance.schema.as_str()
    );
    let projection_row = sqlx::query(audited_sql(projection_sql))
        .bind(&folded.projection.effect_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ModelError::Projection)?;
    if decode_effect_projection(&projection_row)? != folded.projection {
        return Err(ModelError::Projection.into());
    }

    let frontier_sql = format!(
        "SELECT *
         FROM {}.prototype_effect_frontiers
         WHERE stream_id = $1",
        instance.schema.as_str()
    );
    let frontier_row = sqlx::query(audited_sql(frontier_sql))
        .bind(&folded.projection.stream_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ModelError::Projection)?;
    if decode_effect_frontier(&frontier_row)? != folded.frontier {
        return Err(ModelError::Projection.into());
    }

    let receipt_sql = format!(
        "SELECT stream_id, append_request_id, predecessor_sequence,
                predecessor_digest, purpose, candidate_digest,
                committed_sequence, committed_digest
         FROM {}.prototype_append_requests
         WHERE stream_id = $1
         ORDER BY committed_sequence",
        instance.schema.as_str()
    );
    let receipts = sqlx::query(audited_sql(receipt_sql))
        .bind(&folded.projection.stream_id)
        .fetch_all(&mut **transaction)
        .await?
        .into_iter()
        .map(|row| {
            Ok(model::AppendProjection {
                stream_id: row.try_get("stream_id")?,
                append_request_id: row.try_get("append_request_id")?,
                predecessor_sequence: row.try_get("predecessor_sequence")?,
                predecessor_digest: row.try_get("predecessor_digest")?,
                purpose: row.try_get("purpose")?,
                candidate_digest: row.try_get("candidate_digest")?,
                committed_sequence: row.try_get("committed_sequence")?,
                committed_digest: row.try_get("committed_digest")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if receipts != folded.append_requests {
        return Err(ModelError::Projection.into());
    }

    let journal_head_sql = format!(
        "SELECT sequence, commit_digest
         FROM {}.prototype_journal_heads
         WHERE stream_id = $1",
        instance.schema.as_str()
    );
    let head = sqlx::query(audited_sql(journal_head_sql))
        .bind(&folded.projection.stream_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ModelError::Projection)?;
    if head.try_get::<i64, _>("sequence")? != folded.frontier.sequence
        || head.try_get::<Vec<u8>, _>("commit_digest")? != folded.frontier.commit_digest
    {
        return Err(ModelError::Projection.into());
    }
    Ok(())
}

pub(super) async fn load_folded_effect(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    effect_id: &str,
) -> Result<FoldedEffect> {
    let intent_sql = format!(
        "SELECT stream_id, requested_resource_ownership_ref,
                requested_resource_key_ref, requested_policy_ref,
                requested_policy_configuration_ref, semantic_request_digest
         FROM {}.prototype_effect_intents
         WHERE effect_id = $1
         FOR SHARE",
        instance.schema.as_str()
    );
    let intent = sqlx::query(audited_sql(intent_sql))
        .bind(effect_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(super::PrototypeError::MissingEffect)?;
    let stream_id: String = intent.try_get("stream_id")?;
    if stream_id != super::effect_stream_id(effect_id) {
        return Err(ModelError::Projection.into());
    }
    let records = load_effect_records(transaction, instance, &stream_id).await?;
    let folded = fold_effect_records(transaction, instance, &records).await?;
    let bound_record = records.first().ok_or(ModelError::EffectIdentity)?;
    let authenticated_intent = super::decode_bound_resource_intent(&bound_record.opaque_payload)?;
    let projected_intent = match (
        intent.try_get::<Option<String>, _>("requested_resource_ownership_ref")?,
        intent.try_get::<Option<String>, _>("requested_resource_key_ref")?,
        intent.try_get::<Option<String>, _>("requested_policy_ref")?,
        intent.try_get::<Option<Vec<u8>>, _>("requested_policy_configuration_ref")?,
    ) {
        (
            Some(resource_ownership_ref),
            Some(resource_key_ref),
            Some(policy_ref),
            Some(policy_configuration_ref),
        ) => Some(super::BoundResourceIntent {
            resource_ownership_ref,
            resource_key_ref,
            policy_ref,
            policy_configuration_ref,
        }),
        (None, None, None, None) => None,
        _ => return Err(ModelError::Projection.into()),
    };
    if folded.projection.effect_id != effect_id
        || folded.projection.stream_id != stream_id
        || intent.try_get::<Vec<u8>, _>("semantic_request_digest")?
            != folded.projection.request_digest
        || projected_intent != authenticated_intent
    {
        return Err(ModelError::Projection.into());
    }
    validate_effect_derivations(transaction, instance, &folded).await?;
    Ok(folded)
}

pub(super) async fn persist_effect_derivations(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    folded: &FoldedEffect,
) -> Result<()> {
    let projection = &folded.projection;
    let projection_sql = format!(
        "INSERT INTO {}.prototype_effects
         (effect_id, stream_id, effect_key, request_digest, destination_identity,
          state, resource_id, resource_ownership_ref, policy_ref,
          policy_configuration_ref, allocation_state_ref, allocation_value,
          fencing_ref, resource_append_request_id, resource_record_ref,
          semantic_request_digest, destination_account, authorization_count,
          observation_count, late_observation_count)
         VALUES
         ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
          $15, $16, $17, $18, $19, $20)
         ON CONFLICT (effect_id) DO UPDATE
         SET stream_id = EXCLUDED.stream_id,
             effect_key = EXCLUDED.effect_key,
             request_digest = EXCLUDED.request_digest,
             destination_identity = EXCLUDED.destination_identity,
             state = EXCLUDED.state,
             resource_id = EXCLUDED.resource_id,
             resource_ownership_ref = EXCLUDED.resource_ownership_ref,
             policy_ref = EXCLUDED.policy_ref,
             policy_configuration_ref = EXCLUDED.policy_configuration_ref,
             allocation_state_ref = EXCLUDED.allocation_state_ref,
             allocation_value = EXCLUDED.allocation_value,
             fencing_ref = EXCLUDED.fencing_ref,
             resource_append_request_id = EXCLUDED.resource_append_request_id,
             resource_record_ref = EXCLUDED.resource_record_ref,
             semantic_request_digest = EXCLUDED.semantic_request_digest,
             destination_account = EXCLUDED.destination_account,
             authorization_count = EXCLUDED.authorization_count,
             observation_count = EXCLUDED.observation_count,
             late_observation_count = EXCLUDED.late_observation_count",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(projection_sql))
        .bind(&projection.effect_id)
        .bind(&projection.stream_id)
        .bind(&projection.effect_key)
        .bind(&projection.request_digest)
        .bind(&projection.destination_identity)
        .bind(&projection.state)
        .bind(&projection.resource_key_ref)
        .bind(&projection.resource_ownership_ref)
        .bind(&projection.policy_ref)
        .bind(&projection.policy_configuration_ref)
        .bind(&projection.allocation_state_ref)
        .bind(projection.allocation_value)
        .bind(&projection.fencing_ref)
        .bind(&projection.resource_append_request_id)
        .bind(&projection.resource_record_ref)
        .bind(&projection.request_digest)
        .bind(projection.destination_account)
        .bind(projection.authorization_count)
        .bind(projection.observation_count)
        .bind(projection.late_observation_count)
        .execute(&mut **transaction)
        .await?;

    let frontier = &folded.frontier;
    let frontier_sql = format!(
        "INSERT INTO {}.prototype_effect_frontiers
         (stream_id, executor_binding_ref, ledger_generation, effect_key,
          request_digest, destination_identity, sequence, predecessor_digest,
          evidence_object_digest, retained_bytes, commit_digest, proof_digest)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (stream_id) DO UPDATE
         SET executor_binding_ref = EXCLUDED.executor_binding_ref,
             ledger_generation = EXCLUDED.ledger_generation,
             effect_key = EXCLUDED.effect_key,
             request_digest = EXCLUDED.request_digest,
             destination_identity = EXCLUDED.destination_identity,
             sequence = EXCLUDED.sequence,
             predecessor_digest = EXCLUDED.predecessor_digest,
             evidence_object_digest = EXCLUDED.evidence_object_digest,
             retained_bytes = EXCLUDED.retained_bytes,
             commit_digest = EXCLUDED.commit_digest,
             proof_digest = EXCLUDED.proof_digest",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(frontier_sql))
        .bind(&frontier.stream_id)
        .bind(&frontier.executor_binding_ref)
        .bind(frontier.ledger_generation)
        .bind(&frontier.effect_key)
        .bind(&frontier.request_digest)
        .bind(&frontier.destination_identity)
        .bind(frontier.sequence)
        .bind(&frontier.predecessor_digest)
        .bind(&frontier.evidence_object_digest)
        .bind(frontier.retained_bytes)
        .bind(&frontier.commit_digest)
        .bind(&frontier.proof_digest)
        .execute(&mut **transaction)
        .await?;

    validate_effect_derivations(transaction, instance, folded).await
}

#[derive(Default)]
struct EffectBodyBindings {
    resource_ownership_ref: Option<String>,
    policy_ref: Option<String>,
    policy_configuration_ref: Option<Vec<u8>>,
    resource_key_ref: Option<String>,
    allocation_state_ref: Option<Vec<u8>>,
    allocation_value: Option<i64>,
    fencing_ref: Option<Vec<u8>>,
    resource_append_request_id: Option<String>,
    resource_record_ref: Option<Vec<u8>>,
    attempt_ordinal: Option<i64>,
    attempt_id: Option<Vec<u8>>,
    target_operation_ref: Option<String>,
    observation_outcome: Option<String>,
    safe_outcome_ref: Option<Vec<u8>>,
    destination_account: Option<i64>,
    terminal_attempt_id: Option<Vec<u8>>,
    external_operation_ref: Option<String>,
    terminal_outcome_ref: Option<Vec<u8>>,
    terminal_proof_ref: Option<Vec<u8>>,
}

impl EffectBodyBindings {
    fn from_body(body: &EffectRecordBody) -> Self {
        match body {
            EffectRecordBody::EffectBound => Self::default(),
            EffectRecordBody::ResourceAllocated(link) => Self {
                resource_ownership_ref: Some(link.resource_ownership_ref.clone()),
                policy_ref: Some(link.policy_ref.clone()),
                policy_configuration_ref: Some(link.policy_configuration_ref.clone()),
                resource_key_ref: Some(link.resource_key_ref.clone()),
                allocation_state_ref: Some(link.typed_allocation_state_ref.clone()),
                allocation_value: Some(link.allocation_value),
                fencing_ref: link.fencing_ref.clone(),
                resource_append_request_id: Some(link.resource_append_request_id.clone()),
                resource_record_ref: Some(link.resource_record_ref.clone()),
                ..Self::default()
            },
            EffectRecordBody::DeliveryAttemptAuthorized(authorization) => Self {
                attempt_ordinal: Some(authorization.attempt_ordinal),
                attempt_id: Some(authorization.attempt_id.clone()),
                target_operation_ref: Some(authorization.target_operation_ref.clone()),
                ..Self::default()
            },
            EffectRecordBody::DeliveryAttemptObserved(observation) => {
                let (kind, safe_outcome_ref, destination_account) = match &observation.outcome {
                    AttemptOutcome::Returned {
                        safe_outcome_ref,
                        destination_account,
                    } => (
                        "returned",
                        safe_outcome_ref.clone(),
                        Some(*destination_account),
                    ),
                    AttemptOutcome::DidNotEnter { safe_outcome_ref } => {
                        ("did_not_enter", safe_outcome_ref.clone(), None)
                    }
                    AttemptOutcome::Indeterminate { safe_outcome_ref } => {
                        ("indeterminate", safe_outcome_ref.clone(), None)
                    }
                };
                Self {
                    attempt_id: Some(observation.attempt_id.clone()),
                    observation_outcome: Some(kind.to_owned()),
                    safe_outcome_ref: Some(safe_outcome_ref),
                    destination_account,
                    ..Self::default()
                }
            }
            EffectRecordBody::TerminalTombstone(tombstone) => Self {
                terminal_attempt_id: Some(tombstone.terminal_attempt_id.clone()),
                external_operation_ref: Some(tombstone.external_operation_ref.clone()),
                terminal_outcome_ref: Some(tombstone.terminal_outcome_ref.clone()),
                terminal_proof_ref: Some(tombstone.terminal_proof_ref.clone()),
                ..Self::default()
            },
        }
    }
}

pub(super) async fn insert_effect_record_row(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    record: &EffectRecord,
) -> Result<()> {
    let retained_digest =
        retain_evidence_object(transaction, instance, &record.opaque_payload).await?;
    if retained_digest != record.evidence_object_digest {
        return Err(ModelError::EffectChain.into());
    }
    let append_request_id = record
        .append
        .as_ref()
        .map(|append| append.append_request_id.clone());
    let append_predecessor_sequence = record
        .append
        .as_ref()
        .map(|append| append.expected_predecessor_sequence);
    let append_predecessor_digest = record
        .append
        .as_ref()
        .map(|append| append.expected_predecessor_digest.clone());
    let append_purpose = record.append.as_ref().map(|append| append.purpose.clone());
    let append_candidate_digest = record
        .append
        .as_ref()
        .map(|append| append.candidate_digest.clone());
    let body = EffectBodyBindings::from_body(&record.body);
    let sql = format!(
        "INSERT INTO {}.prototype_effect_records
         (stream_id, sequence, predecessor_digest, opaque_payload,
          executor_binding_ref, ledger_generation, effect_key, request_digest,
          destination_identity, append_request_id, append_predecessor_sequence,
          append_predecessor_digest, append_purpose, append_candidate_digest,
          record_kind, resource_ownership_ref, policy_ref,
          policy_configuration_ref, resource_key_ref, allocation_state_ref,
          allocation_value, fencing_ref, resource_append_request_id,
          resource_record_ref, attempt_ordinal, attempt_id,
          target_operation_ref, observation_outcome, safe_outcome_ref,
          destination_account, terminal_attempt_id, external_operation_ref,
          terminal_outcome_ref, terminal_proof_ref, evidence_object_digest,
          retained_bytes, commit_digest, proof_digest)
         VALUES
         ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
          $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26,
          $27, $28, $29, $30, $31, $32, $33, $34, $35, $36, $37, $38)",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(sql))
        .bind(&record.stream_id)
        .bind(record.sequence)
        .bind(&record.predecessor_digest)
        .bind(&record.opaque_payload)
        .bind(&record.executor_binding_ref)
        .bind(record.ledger_generation)
        .bind(&record.effect_key)
        .bind(&record.request_digest)
        .bind(&record.destination_identity)
        .bind(append_request_id)
        .bind(append_predecessor_sequence)
        .bind(append_predecessor_digest)
        .bind(append_purpose)
        .bind(append_candidate_digest)
        .bind(record.body.kind())
        .bind(body.resource_ownership_ref)
        .bind(body.policy_ref)
        .bind(body.policy_configuration_ref)
        .bind(body.resource_key_ref)
        .bind(body.allocation_state_ref)
        .bind(body.allocation_value)
        .bind(body.fencing_ref)
        .bind(body.resource_append_request_id)
        .bind(body.resource_record_ref)
        .bind(body.attempt_ordinal)
        .bind(body.attempt_id)
        .bind(body.target_operation_ref)
        .bind(body.observation_outcome)
        .bind(body.safe_outcome_ref)
        .bind(body.destination_account)
        .bind(body.terminal_attempt_id)
        .bind(body.external_operation_ref)
        .bind(body.terminal_outcome_ref)
        .bind(body.terminal_proof_ref)
        .bind(&record.evidence_object_digest)
        .bind(record.retained_bytes)
        .bind(&record.commit_digest)
        .bind(&record.proof_digest)
        .execute(&mut **transaction)
        .await?;

    let journal_sql = format!(
        "INSERT INTO {}.prototype_journal_records
         (stream_id, sequence, predecessor_digest, record_kind, opaque_payload,
          commit_digest, executor_binding_ref, ledger_generation,
          evidence_object_digest, proof_digest)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(journal_sql))
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
    Ok(())
}

fn decode_policy_configuration(row: &sqlx::postgres::PgRow) -> Result<ResourcePolicyConfiguration> {
    let kind: String = row.try_get("policy_configuration_kind")?;
    let finite_values: Option<Vec<i64>> = row.try_get("finite_allocation_values")?;
    let initial_value: Option<i64> = row.try_get("account_sequence_initial_value")?;
    match kind.as_str() {
        "finite_inventory" if initial_value.is_none() => {
            Ok(ResourcePolicyConfiguration::FiniteInventory {
                ordered_allocation_values: required(finite_values)?,
            })
        }
        "account_sequence" if finite_values.is_none() => {
            Ok(ResourcePolicyConfiguration::AccountSequence {
                initial_value: required(initial_value)?,
            })
        }
        "exclusive" if finite_values.is_none() && initial_value.is_none() => {
            Ok(ResourcePolicyConfiguration::Exclusive)
        }
        _ => Err(ModelError::ResourceChain.into()),
    }
}

pub(super) async fn load_resource_fold(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    resource_ownership_ref: &str,
    resource_key_ref: &str,
) -> Result<Option<FoldedResource>> {
    let records = load_resource_records(
        transaction,
        instance,
        resource_ownership_ref,
        resource_key_ref,
    )
    .await?;
    if records.is_empty() {
        let head_sql = format!(
            "SELECT COUNT(*)
             FROM {}.prototype_resource_heads
             WHERE resource_ownership_ref = $1 AND resource_key_ref = $2",
            instance.schema.as_str()
        );
        let head_count: i64 = sqlx::query_scalar(audited_sql(head_sql))
            .bind(resource_ownership_ref)
            .bind(resource_key_ref)
            .fetch_one(&mut **transaction)
            .await?;
        if head_count != 0 {
            return Err(ModelError::Projection.into());
        }
        return Ok(None);
    }
    let folded = model::fold_resource(&records)?;
    let head_sql = format!(
        "SELECT resource_ownership_ref, policy_ref, policy_configuration_ref,
                resource_key_ref, sequence, record_ref
         FROM {}.prototype_resource_heads
         WHERE resource_ownership_ref = $1 AND resource_key_ref = $2",
        instance.schema.as_str()
    );
    let row = sqlx::query(audited_sql(head_sql))
        .bind(resource_ownership_ref)
        .bind(resource_key_ref)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ModelError::Projection)?;
    let persisted_head = ResourceHeadProjection {
        resource_ownership_ref: row.try_get("resource_ownership_ref")?,
        policy_ref: row.try_get("policy_ref")?,
        policy_configuration_ref: row.try_get("policy_configuration_ref")?,
        resource_key_ref: row.try_get("resource_key_ref")?,
        sequence: row.try_get("sequence")?,
        record_ref: row.try_get("record_ref")?,
    };
    if persisted_head != folded.head {
        return Err(ModelError::Projection.into());
    }
    Ok(Some(folded))
}

pub(super) async fn load_resource_records(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    resource_ownership_ref: &str,
    resource_key_ref: &str,
) -> Result<Vec<ResourceRecord>> {
    let sql = format!(
        "SELECT *
         FROM {}.prototype_resource_records
         WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
         ORDER BY sequence",
        instance.schema.as_str()
    );
    let rows = sqlx::query(audited_sql(sql))
        .bind(resource_ownership_ref)
        .bind(resource_key_ref)
        .fetch_all(&mut **transaction)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ResourceRecord {
                resource_ownership_ref: row.try_get("resource_ownership_ref")?,
                policy_ref: row.try_get("policy_ref")?,
                policy_configuration: decode_policy_configuration(&row)?,
                policy_configuration_ref: row.try_get("policy_configuration_ref")?,
                resource_key_ref: row.try_get("resource_key_ref")?,
                sequence: row.try_get("sequence")?,
                predecessor_ref: row.try_get("predecessor_ref")?,
                append: ResourceAppendIdentity {
                    append_request_id: row.try_get("append_request_id")?,
                    expected_predecessor_sequence: row.try_get("append_predecessor_sequence")?,
                    expected_predecessor_ref: row.try_get("append_predecessor_ref")?,
                    linked_effect_stream_id: row.try_get("linked_effect_stream_id")?,
                    linked_effect_append_request_id: row
                        .try_get("linked_effect_append_request_id")?,
                    candidate_digest: row.try_get("append_candidate_digest")?,
                },
                effect_key: row.try_get("effect_key")?,
                typed_allocation_state_ref: row.try_get("allocation_state_ref")?,
                allocation_value: row.try_get("allocation_value")?,
                fencing_ref: row.try_get("fencing_ref")?,
                executor_binding_ref: row.try_get("executor_binding_ref")?,
                ledger_generation: row.try_get("ledger_generation")?,
                record_ref: row.try_get("record_ref")?,
                proof_digest: row.try_get("proof_digest")?,
            })
        })
        .collect()
}

pub(super) async fn insert_resource_record_row(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    record: &ResourceRecord,
) -> Result<()> {
    let (configuration_kind, finite_values, initial_value) = match &record.policy_configuration {
        ResourcePolicyConfiguration::FiniteInventory {
            ordered_allocation_values,
        } => (
            "finite_inventory",
            Some(ordered_allocation_values.clone()),
            None,
        ),
        ResourcePolicyConfiguration::AccountSequence { initial_value } => {
            ("account_sequence", None, Some(*initial_value))
        }
        ResourcePolicyConfiguration::Exclusive => ("exclusive", None, None),
    };
    let sql = format!(
        "INSERT INTO {}.prototype_resource_records
         (resource_ownership_ref, policy_ref, policy_configuration_kind,
          finite_allocation_values, account_sequence_initial_value,
          policy_configuration_ref, resource_key_ref, sequence, predecessor_ref,
          append_request_id, append_predecessor_sequence, append_predecessor_ref,
          linked_effect_stream_id, linked_effect_append_request_id,
          append_candidate_digest, effect_key, allocation_state_ref,
          allocation_value, fencing_ref, executor_binding_ref, ledger_generation,
          record_ref, proof_digest)
         VALUES
         ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
          $15, $16, $17, $18, $19, $20, $21, $22, $23)",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(sql))
        .bind(&record.resource_ownership_ref)
        .bind(&record.policy_ref)
        .bind(configuration_kind)
        .bind(finite_values)
        .bind(initial_value)
        .bind(&record.policy_configuration_ref)
        .bind(&record.resource_key_ref)
        .bind(record.sequence)
        .bind(&record.predecessor_ref)
        .bind(&record.append.append_request_id)
        .bind(record.append.expected_predecessor_sequence)
        .bind(&record.append.expected_predecessor_ref)
        .bind(&record.append.linked_effect_stream_id)
        .bind(&record.append.linked_effect_append_request_id)
        .bind(&record.append.candidate_digest)
        .bind(&record.effect_key)
        .bind(&record.typed_allocation_state_ref)
        .bind(record.allocation_value)
        .bind(&record.fencing_ref)
        .bind(&record.executor_binding_ref)
        .bind(record.ledger_generation)
        .bind(&record.record_ref)
        .bind(&record.proof_digest)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub(super) async fn compare_and_swap_resource_head(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    expected: Option<&ResourceHeadProjection>,
    folded: &FoldedResource,
) -> Result<()> {
    let head = &folded.head;
    let changed = if let Some(expected) = expected {
        let sql = format!(
            "UPDATE {}.prototype_resource_heads
             SET policy_ref = $1,
                 policy_configuration_ref = $2,
                 sequence = $3,
                 record_ref = $4
             WHERE resource_ownership_ref = $5
               AND resource_key_ref = $6
               AND policy_ref = $7
               AND policy_configuration_ref = $8
               AND sequence = $9
               AND record_ref = $10",
            instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(&head.policy_ref)
            .bind(&head.policy_configuration_ref)
            .bind(head.sequence)
            .bind(&head.record_ref)
            .bind(&expected.resource_ownership_ref)
            .bind(&expected.resource_key_ref)
            .bind(&expected.policy_ref)
            .bind(&expected.policy_configuration_ref)
            .bind(expected.sequence)
            .bind(&expected.record_ref)
            .execute(&mut **transaction)
            .await?
            .rows_affected()
    } else {
        let sql = format!(
            "INSERT INTO {}.prototype_resource_heads
             (resource_ownership_ref, policy_ref, policy_configuration_ref,
              resource_key_ref, sequence, record_ref)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (resource_ownership_ref, resource_key_ref) DO NOTHING",
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
            .await?
            .rows_affected()
    };
    if changed != 1 {
        return Err(super::PrototypeError::HeadMismatch);
    }
    let persisted = load_resource_fold(
        transaction,
        instance,
        &head.resource_ownership_ref,
        &head.resource_key_ref,
    )
    .await?
    .ok_or(ModelError::Projection)?;
    if persisted != *folded {
        return Err(ModelError::Projection.into());
    }
    Ok(())
}
