use mfm_canonical::sha256_digest_bytes;
use sqlx::Transaction;

use super::{
    audited_sql, evidence_object_digest, model,
    persistence::{
        insert_effect_record_row, insert_resource_record_row, load_effect_records,
        load_resource_records,
    },
    seal_effect_record, seal_resource_record, EffectRecordBody, JournalHead, Postgres,
    PrototypeError, ResourceAppendIdentity, ResourcePolicyConfiguration, ResourceRecord,
    RestoreCandidate, Result, TestTopology, DESTINATION_INVENTORY_RESOURCE_KEY_REF,
    DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF,
};

const AUTHORITY_EFFECT: &str = "hostile-authority-a";
const SECOND_RESOURCE_EFFECT: &str = "hostile-authority-b";
const PENDING_EFFECT: &str = "hostile-authority-pending";

#[derive(Clone, Copy, Debug)]
pub(crate) enum CandidateMutation {
    MissingAppendReceipt,
    ExtraAppendReceipt,
    MutatedAppendReceipt,
    MissingEffectIntent,
    ExtraEffectIntent,
    MutatedEffectIntent,
    MissingEffectProjection,
    ExtraEffectProjection,
    RolledBackEffectState,
    AdvancedEffectState,
    MutatedEffectRequest,
    MutatedEffectAllocation,
    MutatedEffectResult,
    MutatedEffectCounter,
    MissingFrontier,
    ExtraFrontier,
    MutatedFrontier,
    MissingEffectHead,
    ExtraEffectHead,
    MutatedEffectHead,
    MissingJournalMirror,
    ExtraJournalMirror,
    MutatedJournalMirror,
    MissingEvidenceContract,
    ExtraEvidenceContract,
    MutatedEvidenceContract,
    MissingEvidenceObject,
    ExtraEvidenceObject,
    MutatedEvidenceObject,
    MissingResourceConfiguration,
    ExtraResourceConfiguration,
    MutatedResourceConfiguration,
    MissingResourceHead,
    ExtraResourceHead,
    MutatedResourceHead,
    MissingResourceRecord,
    DescendantResourceRecord,
    AncestorResourceRecord,
    ForkedResourceRecord,
    MutatedResourceOwnership,
    MutatedResourceKey,
    MutatedResourceLinkedEffect,
    MutatedResourceLinkedAppend,
    MutatedResourceAllocationValue,
    MutatedResourceAllocationState,
    MutatedResourceFence,
    MutatedResourceBinding,
    MutatedResourceGeneration,
    MutatedResourceProof,
    ResealedWrongTerminalProof,
}

impl CandidateMutation {
    pub(crate) const ALL: &[Self] = &[
        Self::MissingAppendReceipt,
        Self::ExtraAppendReceipt,
        Self::MutatedAppendReceipt,
        Self::MissingEffectIntent,
        Self::ExtraEffectIntent,
        Self::MutatedEffectIntent,
        Self::MissingEffectProjection,
        Self::ExtraEffectProjection,
        Self::RolledBackEffectState,
        Self::AdvancedEffectState,
        Self::MutatedEffectRequest,
        Self::MutatedEffectAllocation,
        Self::MutatedEffectResult,
        Self::MutatedEffectCounter,
        Self::MissingFrontier,
        Self::ExtraFrontier,
        Self::MutatedFrontier,
        Self::MissingEffectHead,
        Self::ExtraEffectHead,
        Self::MutatedEffectHead,
        Self::MissingJournalMirror,
        Self::ExtraJournalMirror,
        Self::MutatedJournalMirror,
        Self::MissingEvidenceContract,
        Self::ExtraEvidenceContract,
        Self::MutatedEvidenceContract,
        Self::MissingEvidenceObject,
        Self::ExtraEvidenceObject,
        Self::MutatedEvidenceObject,
        Self::MissingResourceConfiguration,
        Self::ExtraResourceConfiguration,
        Self::MutatedResourceConfiguration,
        Self::MissingResourceHead,
        Self::ExtraResourceHead,
        Self::MutatedResourceHead,
        Self::MissingResourceRecord,
        Self::DescendantResourceRecord,
        Self::AncestorResourceRecord,
        Self::ForkedResourceRecord,
        Self::MutatedResourceOwnership,
        Self::MutatedResourceKey,
        Self::MutatedResourceLinkedEffect,
        Self::MutatedResourceLinkedAppend,
        Self::MutatedResourceAllocationValue,
        Self::MutatedResourceAllocationState,
        Self::MutatedResourceFence,
        Self::MutatedResourceBinding,
        Self::MutatedResourceGeneration,
        Self::MutatedResourceProof,
        Self::ResealedWrongTerminalProof,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::MissingAppendReceipt => "missing append receipt",
            Self::ExtraAppendReceipt => "extra append receipt",
            Self::MutatedAppendReceipt => "mutated append receipt",
            Self::MissingEffectIntent => "missing effect intent",
            Self::ExtraEffectIntent => "extra effect intent",
            Self::MutatedEffectIntent => "mutated effect intent",
            Self::MissingEffectProjection => "missing effect projection",
            Self::ExtraEffectProjection => "extra effect projection",
            Self::RolledBackEffectState => "rolled-back effect state",
            Self::AdvancedEffectState => "advanced effect state",
            Self::MutatedEffectRequest => "mutated effect request",
            Self::MutatedEffectAllocation => "mutated effect allocation",
            Self::MutatedEffectResult => "mutated effect result",
            Self::MutatedEffectCounter => "mutated effect counter",
            Self::MissingFrontier => "missing effect frontier",
            Self::ExtraFrontier => "extra effect frontier",
            Self::MutatedFrontier => "mutated effect frontier",
            Self::MissingEffectHead => "missing effect head",
            Self::ExtraEffectHead => "extra effect head",
            Self::MutatedEffectHead => "mutated effect head",
            Self::MissingJournalMirror => "missing journal mirror",
            Self::ExtraJournalMirror => "extra journal mirror",
            Self::MutatedJournalMirror => "mutated journal mirror",
            Self::MissingEvidenceContract => "missing evidence contract",
            Self::ExtraEvidenceContract => "extra evidence contract",
            Self::MutatedEvidenceContract => "mutated evidence contract",
            Self::MissingEvidenceObject => "missing evidence object",
            Self::ExtraEvidenceObject => "extra evidence object",
            Self::MutatedEvidenceObject => "mutated evidence object",
            Self::MissingResourceConfiguration => "missing resource configuration",
            Self::ExtraResourceConfiguration => "extra resource configuration",
            Self::MutatedResourceConfiguration => "mutated resource configuration",
            Self::MissingResourceHead => "missing resource head",
            Self::ExtraResourceHead => "extra resource head",
            Self::MutatedResourceHead => "mutated resource head",
            Self::MissingResourceRecord => "missing resource record",
            Self::DescendantResourceRecord => "descendant resource record",
            Self::AncestorResourceRecord => "ancestor resource record",
            Self::ForkedResourceRecord => "forked resource record",
            Self::MutatedResourceOwnership => "mutated resource ownership",
            Self::MutatedResourceKey => "mutated resource key",
            Self::MutatedResourceLinkedEffect => "mutated linked effect",
            Self::MutatedResourceLinkedAppend => "mutated linked append",
            Self::MutatedResourceAllocationValue => "mutated allocation value",
            Self::MutatedResourceAllocationState => "mutated allocation state",
            Self::MutatedResourceFence => "mutated resource fence",
            Self::MutatedResourceBinding => "mutated resource binding",
            Self::MutatedResourceGeneration => "mutated resource generation",
            Self::MutatedResourceProof => "mutated resource proof",
            Self::ResealedWrongTerminalProof => "resealed wrong terminal proof",
        }
    }
}

pub(crate) const fn authority_effect() -> &'static str {
    AUTHORITY_EFFECT
}

pub(crate) const fn second_resource_effect() -> &'static str {
    SECOND_RESOURCE_EFFECT
}

pub(crate) const fn pending_effect() -> &'static str {
    PENDING_EFFECT
}

impl TestTopology {
    pub(crate) async fn mutate_restore_candidate(
        &self,
        candidate: &RestoreCandidate,
        mutation: CandidateMutation,
    ) -> Result<()> {
        let schema = candidate.instance.schema.as_str();
        let authority_stream = model::effect_stream_id(AUTHORITY_EFFECT);
        let pending_stream = model::effect_stream_id(PENDING_EFFECT);
        match mutation {
            CandidateMutation::MissingAppendReceipt => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_append_requests
                     WHERE stream_id = $1 AND committed_sequence = (
                       SELECT MAX(committed_sequence)
                       FROM {schema}.prototype_append_requests
                       WHERE stream_id = $1
                     )",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::ExtraAppendReceipt => {
                let sql = format!(
                    "INSERT INTO {schema}.prototype_append_requests
                     (stream_id, append_request_id, predecessor_sequence,
                      predecessor_digest, purpose, candidate_digest,
                      committed_sequence, committed_digest)
                     SELECT stream_id, append_request_id || ':hostile-extra',
                            predecessor_sequence, predecessor_digest, purpose,
                            candidate_digest, committed_sequence, committed_digest
                     FROM {schema}.prototype_append_requests
                     WHERE stream_id = $1
                     ORDER BY committed_sequence
                     LIMIT 1"
                );
                execute_bound(self, candidate, sql, &authority_stream).await
            }
            CandidateMutation::MutatedAppendReceipt => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_append_requests
                     SET purpose = purpose || ':hostile'
                     WHERE stream_id = $1 AND committed_sequence = (
                       SELECT MAX(committed_sequence)
                       FROM {schema}.prototype_append_requests
                       WHERE stream_id = $1
                     )",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MissingEffectIntent => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_effect_intents WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::ExtraEffectIntent => {
                let sql = format!(
                    "INSERT INTO {schema}.prototype_effect_intents
                     (effect_id, stream_id, requested_resource_ownership_ref,
                      requested_resource_key_ref, requested_policy_ref,
                      requested_policy_configuration_ref, semantic_request_digest)
                     VALUES ('hostile-extra-intent', 'effect:hostile-extra-intent',
                             NULL, NULL, NULL, NULL, decode(repeat('01', 32), 'hex'))"
                );
                execute_unbound(self, candidate, sql).await
            }
            CandidateMutation::MutatedEffectIntent => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_effect_intents
                     SET semantic_request_digest = decode(repeat('02', 32), 'hex')
                     WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MissingEffectProjection => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_effects WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::ExtraEffectProjection => {
                let sql = format!(
                    "INSERT INTO {schema}.prototype_effects
                     SELECT 'hostile-extra-projection', 'effect:hostile-extra-projection',
                            effect_key, request_digest, destination_identity, state,
                            resource_id, resource_ownership_ref, policy_ref,
                            policy_configuration_ref, allocation_state_ref, allocation_value,
                            fencing_ref, resource_append_request_id, resource_record_ref,
                            semantic_request_digest, destination_account, authorization_count,
                            observation_count, late_observation_count
                     FROM {schema}.prototype_effects
                     WHERE stream_id = $1"
                );
                execute_bound(self, candidate, sql, &authority_stream).await
            }
            CandidateMutation::RolledBackEffectState => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_effects
                     SET state = 'observed'
                     WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::AdvancedEffectState => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_effects
                     SET state = 'tombstoned'
                     WHERE stream_id = $1",
                    &pending_stream,
                )
                .await
            }
            CandidateMutation::MutatedEffectRequest => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_effects
                     SET request_digest = decode(repeat('03', 32), 'hex'),
                         semantic_request_digest = decode(repeat('03', 32), 'hex')
                     WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MutatedEffectAllocation => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_effects
                     SET allocation_value = allocation_value + 1
                     WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MutatedEffectResult => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_effects
                     SET destination_account = destination_account + 1
                     WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MutatedEffectCounter => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_effects
                     SET authorization_count = authorization_count + 1
                     WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MissingFrontier => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_effect_frontiers WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::ExtraFrontier => {
                let sql = format!(
                    "INSERT INTO {schema}.prototype_effect_frontiers
                     SELECT 'effect:hostile-extra-frontier', executor_binding_ref,
                            ledger_generation, effect_key, request_digest,
                            destination_identity, sequence, predecessor_digest,
                            evidence_object_digest, retained_bytes, commit_digest,
                            proof_digest
                     FROM {schema}.prototype_effect_frontiers
                     WHERE stream_id = $1"
                );
                execute_bound(self, candidate, sql, &authority_stream).await
            }
            CandidateMutation::MutatedFrontier => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_effect_frontiers
                     SET request_digest = decode(repeat('04', 32), 'hex')
                     WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MissingEffectHead => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_journal_heads WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::ExtraEffectHead => {
                let sql = format!(
                    "INSERT INTO {schema}.prototype_journal_heads
                     (stream_id, sequence, commit_digest)
                     VALUES ('effect:hostile-extra-head', 1,
                             decode(repeat('05', 32), 'hex'))"
                );
                execute_unbound(self, candidate, sql).await
            }
            CandidateMutation::MutatedEffectHead => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_journal_heads
                     SET commit_digest = decode(repeat('06', 32), 'hex')
                     WHERE stream_id = $1",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MissingJournalMirror => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_journal_records
                     WHERE stream_id = $1 AND sequence = (
                       SELECT MAX(sequence)
                       FROM {schema}.prototype_journal_records
                       WHERE stream_id = $1
                     )",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::ExtraJournalMirror => {
                let sql = format!(
                    "INSERT INTO {schema}.prototype_journal_records
                     SELECT 'effect:hostile-extra-journal', sequence,
                            predecessor_digest, record_kind, opaque_payload,
                            commit_digest, executor_binding_ref, ledger_generation,
                            evidence_object_digest, proof_digest
                     FROM {schema}.prototype_journal_records
                     WHERE stream_id = $1
                     ORDER BY sequence
                     LIMIT 1"
                );
                execute_bound(self, candidate, sql, &authority_stream).await
            }
            CandidateMutation::MutatedJournalMirror => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_journal_records
                     SET opaque_payload = opaque_payload || decode('00', 'hex')
                     WHERE stream_id = $1 AND sequence = (
                       SELECT MAX(sequence)
                       FROM {schema}.prototype_journal_records
                       WHERE stream_id = $1
                     )",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MissingEvidenceContract => {
                let sql = format!("DELETE FROM {schema}.prototype_evidence_contracts");
                execute_unbound(self, candidate, sql).await
            }
            CandidateMutation::ExtraEvidenceContract => {
                let sql = format!(
                    "INSERT INTO {schema}.prototype_evidence_contracts
                     SELECT contract_ref || ':hostile-extra', max_attempts, max_records,
                            max_retained_bytes, completion_reserve_records,
                            completion_reserve_bytes
                     FROM {schema}.prototype_evidence_contracts"
                );
                execute_unbound(self, candidate, sql).await
            }
            CandidateMutation::MutatedEvidenceContract => {
                let sql = format!(
                    "UPDATE {schema}.prototype_evidence_contracts
                     SET max_attempts = max_attempts + 1"
                );
                execute_unbound(self, candidate, sql).await
            }
            CandidateMutation::MissingEvidenceObject => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_evidence_objects
                     WHERE object_digest = (
                       SELECT evidence_object_digest
                       FROM {schema}.prototype_effect_records
                       WHERE stream_id = $1
                       ORDER BY sequence
                       LIMIT 1
                     )",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::ExtraEvidenceObject => {
                let payload = b"hostile-extra-evidence-object";
                let sql = format!(
                    "INSERT INTO {schema}.prototype_evidence_objects
                     (object_digest, opaque_bytes)
                     VALUES ($1, $2)"
                );
                let mut transaction = self.inner.pool.begin().await?;
                sqlx::query(audited_sql(sql))
                    .bind(evidence_object_digest(payload))
                    .bind(payload.as_slice())
                    .execute(&mut *transaction)
                    .await?;
                transaction.commit().await?;
                Ok(())
            }
            CandidateMutation::MutatedEvidenceObject => {
                execute_effect_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_evidence_objects
                     SET opaque_bytes = opaque_bytes || decode('00', 'hex')
                     WHERE object_digest = (
                       SELECT evidence_object_digest
                       FROM {schema}.prototype_effect_records
                       WHERE stream_id = $1
                       ORDER BY sequence
                       LIMIT 1
                     )",
                    &authority_stream,
                )
                .await
            }
            CandidateMutation::MissingResourceConfiguration => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_resource_configurations
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2",
                )
                .await
            }
            CandidateMutation::ExtraResourceConfiguration => {
                let configuration = ResourcePolicyConfiguration::Exclusive;
                let sql = format!(
                    "INSERT INTO {schema}.prototype_resource_configurations
                     (resource_ownership_ref, resource_key_ref, policy_ref,
                      policy_configuration_kind, finite_allocation_values,
                      account_sequence_initial_value, policy_configuration_ref)
                     VALUES ('resource-owner:hostile-extra', 'resource-key:hostile-extra',
                             $1, 'exclusive', NULL, NULL, $2)"
                );
                let mut transaction = self.inner.pool.begin().await?;
                sqlx::query(audited_sql(sql))
                    .bind(configuration.policy_ref())
                    .bind(model::resource_policy_configuration_ref(&configuration))
                    .execute(&mut *transaction)
                    .await?;
                transaction.commit().await?;
                Ok(())
            }
            CandidateMutation::MutatedResourceConfiguration => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_configurations
                     SET finite_allocation_values =
                           array_append(finite_allocation_values, 999999)
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2",
                )
                .await
            }
            CandidateMutation::MissingResourceHead => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_resource_heads
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2",
                )
                .await
            }
            CandidateMutation::ExtraResourceHead => {
                let sql = format!(
                    "INSERT INTO {schema}.prototype_resource_heads
                     (resource_ownership_ref, policy_ref, policy_configuration_ref,
                      resource_key_ref, sequence, record_ref)
                     VALUES ('resource-owner:hostile-extra-head',
                             'resource-policy:exclusive:v1',
                             decode(repeat('07', 32), 'hex'),
                             'resource-key:hostile-extra-head', 1,
                             decode(repeat('08', 32), 'hex'))"
                );
                execute_unbound(self, candidate, sql).await
            }
            CandidateMutation::MutatedResourceHead => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_heads
                     SET record_ref = decode(repeat('09', 32), 'hex')
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2",
                )
                .await
            }
            CandidateMutation::MissingResourceRecord => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "DELETE FROM {schema}.prototype_resource_records
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 1",
                )
                .await
            }
            CandidateMutation::DescendantResourceRecord => {
                append_descendant_resource(self, candidate).await
            }
            CandidateMutation::AncestorResourceRecord => {
                make_resource_ancestor(self, candidate).await
            }
            CandidateMutation::ForkedResourceRecord => fork_resource(self, candidate).await,
            CandidateMutation::MutatedResourceOwnership => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET resource_ownership_ref = resource_ownership_ref || ':hostile'
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceKey => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET resource_key_ref = resource_key_ref || ':hostile'
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceLinkedEffect => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET linked_effect_stream_id = 'effect:hostile-linked-effect'
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceLinkedAppend => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET linked_effect_append_request_id =
                           linked_effect_append_request_id || ':hostile'
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceAllocationValue => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET allocation_value = allocation_value + 1
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceAllocationState => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET allocation_state_ref = decode(repeat('0a', 32), 'hex')
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceFence => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET fencing_ref = decode(repeat('0b', 32), 'hex')
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceBinding => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET executor_binding_ref = executor_binding_ref || ':hostile'
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceGeneration => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET ledger_generation = ledger_generation + 1
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::MutatedResourceProof => {
                execute_resource_mutation(
                    self,
                    candidate,
                    "UPDATE {schema}.prototype_resource_records
                     SET proof_digest = decode(repeat('0c', 32), 'hex')
                     WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
                       AND sequence = 2",
                )
                .await
            }
            CandidateMutation::ResealedWrongTerminalProof => {
                reseal_wrong_terminal_proof(self, candidate, &authority_stream).await
            }
        }
    }
}

async fn reseal_wrong_terminal_proof(
    topology: &TestTopology,
    candidate: &RestoreCandidate,
    stream_id: &str,
) -> Result<()> {
    let mut transaction = topology.inner.pool.begin().await?;
    let records = load_effect_records(&mut transaction, &candidate.instance, stream_id).await?;
    let mut terminal = records
        .last()
        .cloned()
        .ok_or(PrototypeError::HeadMismatch)?;
    match &mut terminal.body {
        EffectRecordBody::TerminalTombstone(tombstone) => {
            tombstone.terminal_proof_ref = vec![0; 32];
        }
        _ => return Err(PrototypeError::HeadMismatch),
    }
    let append = terminal
        .append
        .as_mut()
        .ok_or(PrototypeError::HeadMismatch)?;
    append.candidate_digest = model::effect_candidate_digest(
        append.expected_predecessor_sequence,
        &append.expected_predecessor_digest,
        &append.purpose,
        &terminal.opaque_payload,
        &terminal.body,
    );
    let terminal = seal_effect_record(terminal);
    let delete_sql = format!(
        "DELETE FROM {schema}.prototype_effect_records
         WHERE stream_id = $1 AND sequence = $2;
         DELETE FROM {schema}.prototype_journal_records
         WHERE stream_id = $1 AND sequence = $2",
        schema = candidate.instance.schema.as_str()
    );
    for statement in delete_sql
        .split(';')
        .filter(|statement| !statement.trim().is_empty())
    {
        sqlx::query(audited_sql(statement.to_owned()))
            .bind(stream_id)
            .bind(terminal.sequence)
            .execute(&mut *transaction)
            .await?;
    }
    insert_effect_record_row(&mut transaction, &candidate.instance, &terminal).await?;
    transaction.commit().await?;
    Ok(())
}

async fn execute_effect_mutation(
    topology: &TestTopology,
    candidate: &RestoreCandidate,
    template: &str,
    stream_id: &str,
) -> Result<()> {
    let sql = template.replace("{schema}", candidate.instance.schema.as_str());
    execute_bound(topology, candidate, sql, stream_id).await
}

async fn execute_resource_mutation(
    topology: &TestTopology,
    candidate: &RestoreCandidate,
    template: &str,
) -> Result<()> {
    let sql = template.replace("{schema}", candidate.instance.schema.as_str());
    let mut transaction = topology.inner.pool.begin().await?;
    sqlx::query(audited_sql(sql))
        .bind(DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF)
        .bind(DESTINATION_INVENTORY_RESOURCE_KEY_REF)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn execute_bound(
    topology: &TestTopology,
    _candidate: &RestoreCandidate,
    sql: String,
    value: &str,
) -> Result<()> {
    let mut transaction = topology.inner.pool.begin().await?;
    sqlx::query(audited_sql(sql))
        .bind(value)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn execute_unbound(
    topology: &TestTopology,
    _candidate: &RestoreCandidate,
    sql: String,
) -> Result<()> {
    sqlx::query(audited_sql(sql))
        .execute(&topology.inner.pool)
        .await?;
    Ok(())
}

async fn append_descendant_resource(
    topology: &TestTopology,
    candidate: &RestoreCandidate,
) -> Result<()> {
    let mut transaction = topology.inner.pool.begin().await?;
    let records = load_resource_records(
        &mut transaction,
        &candidate.instance,
        DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF,
        DESTINATION_INVENTORY_RESOURCE_KEY_REF,
    )
    .await?;
    let last = records.last().ok_or(PrototypeError::HeadMismatch)?;
    let ResourcePolicyConfiguration::FiniteInventory {
        ordered_allocation_values,
    } = &last.policy_configuration
    else {
        return Err(PrototypeError::HeadMismatch);
    };
    let allocation_value = *ordered_allocation_values
        .get(2)
        .ok_or(PrototypeError::HeadMismatch)?;
    let effect_key = sha256_digest_bytes(b"hostile-descendant-effect-key")
        .as_bytes()
        .to_vec();
    let record = seal_resource_record(ResourceRecord {
        resource_ownership_ref: last.resource_ownership_ref.clone(),
        policy_ref: last.policy_ref.clone(),
        policy_configuration: last.policy_configuration.clone(),
        policy_configuration_ref: last.policy_configuration_ref.clone(),
        resource_key_ref: last.resource_key_ref.clone(),
        sequence: 3,
        predecessor_ref: Some(last.record_ref.clone()),
        append: ResourceAppendIdentity {
            append_request_id: "resource-record:hostile-descendant".to_owned(),
            expected_predecessor_sequence: 2,
            expected_predecessor_ref: Some(last.record_ref.clone()),
            linked_effect_stream_id: "effect:hostile-descendant".to_owned(),
            linked_effect_append_request_id: "resource-allocation:hostile-descendant".to_owned(),
            candidate_digest: Vec::new(),
        },
        effect_key,
        typed_allocation_state_ref: model::allocation_state_ref(
            &last.policy_ref,
            &last.policy_configuration_ref,
            &last.resource_key_ref,
            allocation_value,
        ),
        allocation_value,
        fencing_ref: None,
        executor_binding_ref: last.executor_binding_ref.clone(),
        ledger_generation: last.ledger_generation,
        record_ref: Vec::new(),
        proof_digest: Vec::new(),
    });
    insert_resource_record_row(&mut transaction, &candidate.instance, &record).await?;
    update_resource_head(
        &mut transaction,
        candidate,
        JournalHead {
            sequence: record.sequence,
            digest: record.record_ref,
        },
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn make_resource_ancestor(
    topology: &TestTopology,
    candidate: &RestoreCandidate,
) -> Result<()> {
    let mut transaction = topology.inner.pool.begin().await?;
    let records = load_resource_records(
        &mut transaction,
        &candidate.instance,
        DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF,
        DESTINATION_INVENTORY_RESOURCE_KEY_REF,
    )
    .await?;
    let first = records.first().ok_or(PrototypeError::HeadMismatch)?;
    let delete_sql = format!(
        "DELETE FROM {}.prototype_resource_records
         WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
           AND sequence > 1",
        candidate.instance.schema.as_str()
    );
    sqlx::query(audited_sql(delete_sql))
        .bind(DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF)
        .bind(DESTINATION_INVENTORY_RESOURCE_KEY_REF)
        .execute(&mut *transaction)
        .await?;
    update_resource_head(
        &mut transaction,
        candidate,
        JournalHead {
            sequence: 1,
            digest: first.record_ref.clone(),
        },
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn fork_resource(topology: &TestTopology, candidate: &RestoreCandidate) -> Result<()> {
    let mut transaction = topology.inner.pool.begin().await?;
    let records = load_resource_records(
        &mut transaction,
        &candidate.instance,
        DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF,
        DESTINATION_INVENTORY_RESOURCE_KEY_REF,
    )
    .await?;
    let mut fork = records
        .get(1)
        .cloned()
        .ok_or(PrototypeError::HeadMismatch)?;
    fork.effect_key = sha256_digest_bytes(b"hostile-fork-effect-key")
        .as_bytes()
        .to_vec();
    fork.append.linked_effect_stream_id = "effect:hostile-resource-fork".to_owned();
    fork = seal_resource_record(fork);
    let delete_sql = format!(
        "DELETE FROM {}.prototype_resource_records
         WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
           AND sequence = 2",
        candidate.instance.schema.as_str()
    );
    sqlx::query(audited_sql(delete_sql))
        .bind(DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF)
        .bind(DESTINATION_INVENTORY_RESOURCE_KEY_REF)
        .execute(&mut *transaction)
        .await?;
    insert_resource_record_row(&mut transaction, &candidate.instance, &fork).await?;
    update_resource_head(
        &mut transaction,
        candidate,
        JournalHead {
            sequence: fork.sequence,
            digest: fork.record_ref,
        },
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn update_resource_head(
    transaction: &mut Transaction<'_, Postgres>,
    candidate: &RestoreCandidate,
    head: JournalHead,
) -> Result<()> {
    let sql = format!(
        "UPDATE {}.prototype_resource_heads
         SET sequence = $1, record_ref = $2
         WHERE resource_ownership_ref = $3 AND resource_key_ref = $4",
        candidate.instance.schema.as_str()
    );
    sqlx::query(audited_sql(sql))
        .bind(head.sequence)
        .bind(head.digest)
        .bind(DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF)
        .bind(DESTINATION_INVENTORY_RESOURCE_KEY_REF)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
