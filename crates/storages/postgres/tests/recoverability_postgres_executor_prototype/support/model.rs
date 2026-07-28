use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::sha256_digest_bytes;
use thiserror::Error;

pub(super) const EFFECT_STREAM_PREFIX: &str = "effect:";
pub(super) const RESOURCE_STREAM_PREFIX: &str = "resource:";
/// Maximum UTF-8 byte length of an effect identifier accepted by the prototype.
///
/// Completion records repeat the identifier in their stream, append identity, and (for a
/// tombstone) external operation reference. Keeping this input bounded makes the fixed completion
/// byte reserve sound before an authorization can enter the destination.
pub(super) const MAX_EFFECT_ID_UTF8_BYTES: usize = 256;
pub(super) const FINITE_INVENTORY_POLICY_REF: &str = "resource-policy:finite-inventory:v1";
pub(super) const ACCOUNT_SEQUENCE_POLICY_REF: &str = "resource-policy:account-sequence:v1";
pub(super) const EXCLUSIVE_POLICY_REF: &str = "resource-policy:exclusive:v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EvidenceContract {
    pub(super) contract_ref: String,
    pub(super) max_attempts: i64,
    pub(super) max_records: i64,
    pub(super) max_retained_bytes: i64,
    pub(super) completion_reserve_records: i64,
    pub(super) completion_reserve_bytes: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AppendIdentity {
    pub(super) append_request_id: String,
    pub(super) expected_predecessor_sequence: i64,
    pub(super) expected_predecessor_digest: Vec<u8>,
    pub(super) purpose: String,
    pub(super) candidate_digest: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct AppendProjection {
    pub(super) stream_id: String,
    pub(super) append_request_id: String,
    pub(super) predecessor_sequence: i64,
    pub(super) predecessor_digest: Vec<u8>,
    pub(super) purpose: String,
    pub(super) candidate_digest: Vec<u8>,
    pub(super) committed_sequence: i64,
    pub(super) committed_digest: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResourceLink {
    pub(super) resource_ownership_ref: String,
    pub(super) policy_ref: String,
    pub(super) policy_configuration_ref: Vec<u8>,
    pub(super) resource_key_ref: String,
    pub(super) typed_allocation_state_ref: Vec<u8>,
    pub(super) allocation_value: i64,
    pub(super) fencing_ref: Option<Vec<u8>>,
    pub(super) resource_append_request_id: String,
    pub(super) resource_record_ref: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundResourceIntent {
    pub(super) resource_ownership_ref: String,
    pub(super) resource_key_ref: String,
    pub(super) policy_ref: String,
    pub(super) policy_configuration_ref: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AttemptAuthorization {
    pub(super) attempt_ordinal: i64,
    pub(super) attempt_id: Vec<u8>,
    pub(super) target_operation_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AttemptObservation {
    pub(super) attempt_id: Vec<u8>,
    pub(super) outcome: AttemptOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum AttemptOutcome {
    Returned {
        safe_outcome_ref: Vec<u8>,
        destination_account: i64,
    },
    DidNotEnter {
        safe_outcome_ref: Vec<u8>,
    },
    Indeterminate {
        safe_outcome_ref: Vec<u8>,
    },
}

impl AttemptOutcome {
    fn safe_outcome_ref(&self) -> &[u8] {
        match self {
            Self::Returned {
                safe_outcome_ref, ..
            }
            | Self::DidNotEnter { safe_outcome_ref }
            | Self::Indeterminate { safe_outcome_ref } => safe_outcome_ref,
        }
    }

    fn destination_account(&self) -> Option<i64> {
        match self {
            Self::Returned {
                destination_account,
                ..
            } => Some(*destination_account),
            Self::DidNotEnter { .. } | Self::Indeterminate { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Tombstone {
    pub(super) terminal_attempt_id: Vec<u8>,
    pub(super) external_operation_ref: String,
    pub(super) terminal_outcome_ref: Vec<u8>,
    pub(super) terminal_proof_ref: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum EffectRecordBody {
    EffectBound,
    ResourceAllocated(ResourceLink),
    DeliveryAttemptAuthorized(AttemptAuthorization),
    DeliveryAttemptObserved(AttemptObservation),
    TerminalTombstone(Tombstone),
}

impl EffectRecordBody {
    pub(super) const fn kind(&self) -> &'static str {
        match self {
            Self::EffectBound => "effect_bound",
            Self::ResourceAllocated(_) => "resource_allocated",
            Self::DeliveryAttemptAuthorized(_) => "delivery_attempt_authorized",
            Self::DeliveryAttemptObserved(_) => "delivery_attempt_observed",
            Self::TerminalTombstone(_) => "terminal_tombstone",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EffectRecord {
    pub(super) stream_id: String,
    pub(super) sequence: i64,
    pub(super) predecessor_digest: Option<Vec<u8>>,
    pub(super) opaque_payload: Vec<u8>,
    pub(super) executor_binding_ref: String,
    pub(super) ledger_generation: i64,
    pub(super) effect_key: Vec<u8>,
    pub(super) request_digest: Vec<u8>,
    pub(super) destination_identity: String,
    pub(super) append: Option<AppendIdentity>,
    pub(super) body: EffectRecordBody,
    pub(super) evidence_object_digest: Vec<u8>,
    pub(super) retained_bytes: i64,
    pub(super) commit_digest: Vec<u8>,
    pub(super) proof_digest: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProjectedEffectState {
    Pending,
    Authorized,
    Observed,
    Tombstoned,
}

impl ProjectedEffectState {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Authorized => "authorized",
            Self::Observed => "observed",
            Self::Tombstoned => "tombstoned",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct EffectProjection {
    pub(super) effect_id: String,
    pub(super) stream_id: String,
    pub(super) effect_key: Vec<u8>,
    pub(super) request_digest: Vec<u8>,
    pub(super) destination_identity: String,
    pub(super) state: String,
    pub(super) resource_ownership_ref: Option<String>,
    pub(super) policy_ref: Option<String>,
    pub(super) policy_configuration_ref: Option<Vec<u8>>,
    pub(super) resource_key_ref: Option<String>,
    pub(super) allocation_state_ref: Option<Vec<u8>>,
    pub(super) allocation_value: Option<i64>,
    pub(super) fencing_ref: Option<Vec<u8>>,
    pub(super) resource_append_request_id: Option<String>,
    pub(super) resource_record_ref: Option<Vec<u8>>,
    pub(super) destination_account: Option<i64>,
    pub(super) authorization_count: i64,
    pub(super) observation_count: i64,
    pub(super) late_observation_count: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EffectFrontier {
    pub(super) stream_id: String,
    pub(super) executor_binding_ref: String,
    pub(super) ledger_generation: i64,
    pub(super) effect_key: Vec<u8>,
    pub(super) request_digest: Vec<u8>,
    pub(super) destination_identity: String,
    pub(super) sequence: i64,
    pub(super) predecessor_digest: Option<Vec<u8>>,
    pub(super) evidence_object_digest: Vec<u8>,
    pub(super) retained_bytes: i64,
    pub(super) commit_digest: Vec<u8>,
    pub(super) proof_digest: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FoldedEffect {
    pub(super) projection: EffectProjection,
    pub(super) append_requests: Vec<AppendProjection>,
    pub(super) frontier: EffectFrontier,
    pub(super) record_count: i64,
    pub(super) retained_bytes: i64,
    pub(super) unmatched_attempts: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ResourcePolicyConfiguration {
    FiniteInventory { ordered_allocation_values: Vec<i64> },
    AccountSequence { initial_value: i64 },
    Exclusive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResourceAppendIdentity {
    pub(super) append_request_id: String,
    pub(super) expected_predecessor_sequence: i64,
    pub(super) expected_predecessor_ref: Option<Vec<u8>>,
    pub(super) linked_effect_stream_id: String,
    pub(super) linked_effect_append_request_id: String,
    pub(super) candidate_digest: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResourceRecord {
    pub(super) resource_ownership_ref: String,
    pub(super) policy_ref: String,
    pub(super) policy_configuration: ResourcePolicyConfiguration,
    pub(super) policy_configuration_ref: Vec<u8>,
    pub(super) resource_key_ref: String,
    pub(super) sequence: i64,
    pub(super) predecessor_ref: Option<Vec<u8>>,
    pub(super) append: ResourceAppendIdentity,
    pub(super) effect_key: Vec<u8>,
    pub(super) typed_allocation_state_ref: Vec<u8>,
    pub(super) allocation_value: i64,
    pub(super) fencing_ref: Option<Vec<u8>>,
    pub(super) executor_binding_ref: String,
    pub(super) ledger_generation: i64,
    pub(super) record_ref: Vec<u8>,
    pub(super) proof_digest: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ResourceHeadProjection {
    pub(super) resource_ownership_ref: String,
    pub(super) policy_ref: String,
    pub(super) policy_configuration_ref: Vec<u8>,
    pub(super) resource_key_ref: String,
    pub(super) sequence: i64,
    pub(super) record_ref: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FoldedResource {
    pub(super) head: ResourceHeadProjection,
    pub(super) records: Vec<ResourceRecord>,
}

#[derive(Debug, Error)]
pub(crate) enum ModelError {
    #[error("effect stream is empty or has an invalid identity")]
    EffectIdentity,
    #[error("effect stream record chain is invalid")]
    EffectChain,
    #[error("effect record algebra is invalid")]
    EffectAlgebra,
    #[error("effect append identity is invalid")]
    AppendIdentity,
    #[error("executor evidence bounds are exhausted")]
    EvidenceBounds,
    #[error("resource stream record chain is invalid")]
    ResourceChain,
    #[error("resource policy history is invalid")]
    ResourcePolicy,
    #[error("effect and resource allocation links disagree")]
    ResourceLink,
    #[error("stored executor projection does not match its immutable records")]
    Projection,
}

pub(super) fn effect_stream_id(effect_id: &str) -> String {
    format!("{EFFECT_STREAM_PREFIX}{effect_id}")
}

pub(super) fn validate_effect_id(effect_id: &str) -> Result<(), ModelError> {
    if effect_id.is_empty() || effect_id.len() > MAX_EFFECT_ID_UTF8_BYTES {
        return Err(ModelError::EffectIdentity);
    }
    Ok(())
}

pub(super) fn bundled_resource_allocation_append_request_id(
    effect_id: &str,
    authorization_append_request_id: &str,
) -> String {
    let digest = digest_parts(&[
        b"mfm.pg-prototype.bundled-resource-allocation-append.v1",
        effect_id.as_bytes(),
        authorization_append_request_id.as_bytes(),
    ]);
    format!("resource-allocation-bundle:{}", lower_hex(&digest))
}

pub(super) fn resource_stream_id(resource_ownership_ref: &str, resource_key_ref: &str) -> String {
    format!(
        "{RESOURCE_STREAM_PREFIX}{}:{}",
        lower_hex(resource_ownership_ref.as_bytes()),
        lower_hex(resource_key_ref.as_bytes())
    )
}

pub(super) fn resource_configuration_stream_id(
    resource_ownership_ref: &str,
    resource_key_ref: &str,
) -> String {
    format!(
        "resource-configuration:{}:{}",
        lower_hex(resource_ownership_ref.as_bytes()),
        lower_hex(resource_key_ref.as_bytes())
    )
}

pub(super) fn effect_key(
    executor_binding_ref: &str,
    ledger_generation: i64,
    effect_id: &str,
) -> Vec<u8> {
    digest_parts(&[
        b"mfm.pg-prototype.effect-key.v1",
        executor_binding_ref.as_bytes(),
        &ledger_generation.to_be_bytes(),
        effect_id.as_bytes(),
    ])
}

pub(super) fn request_digest(
    destination_identity: &str,
    opaque_semantic_request: &[u8],
) -> Vec<u8> {
    digest_parts(&[
        b"mfm.pg-prototype.request.v1",
        destination_identity.as_bytes(),
        opaque_semantic_request,
    ])
}

pub(super) fn derive_attempt_id(
    executor_binding_ref: &str,
    ledger_generation: i64,
    effect_key: &[u8],
    request_digest: &[u8],
    attempt_ordinal: i64,
    target_operation_ref: &str,
) -> Vec<u8> {
    digest_parts(&[
        b"mfm.pg-prototype.delivery-attempt.v1",
        executor_binding_ref.as_bytes(),
        &ledger_generation.to_be_bytes(),
        effect_key,
        request_digest,
        &attempt_ordinal.to_be_bytes(),
        target_operation_ref.as_bytes(),
    ])
}

pub(super) fn terminal_proof_ref(
    terminal_attempt_id: &[u8],
    external_operation_ref: &str,
    terminal_outcome_ref: &[u8],
) -> Vec<u8> {
    digest_parts(&[
        b"mfm.pg-prototype.terminal-proof.v2",
        terminal_attempt_id,
        external_operation_ref.as_bytes(),
        terminal_outcome_ref,
    ])
}

impl ResourcePolicyConfiguration {
    pub(super) const fn policy_ref(&self) -> &'static str {
        match self {
            Self::FiniteInventory { .. } => FINITE_INVENTORY_POLICY_REF,
            Self::AccountSequence { .. } => ACCOUNT_SEQUENCE_POLICY_REF,
            Self::Exclusive => EXCLUSIVE_POLICY_REF,
        }
    }
}

pub(super) fn resource_policy_configuration_ref(
    configuration: &ResourcePolicyConfiguration,
) -> Vec<u8> {
    digest_parts(&[
        b"mfm.pg-prototype.resource-policy-configuration.v1",
        &resource_policy_configuration_preimage(configuration),
    ])
}

pub(super) fn allocation_state_ref(
    policy_ref: &str,
    policy_configuration_ref: &[u8],
    resource_key_ref: &str,
    allocation_value: i64,
) -> Vec<u8> {
    digest_parts(&[
        b"mfm.pg-prototype.allocation-state.v1",
        policy_ref.as_bytes(),
        policy_configuration_ref,
        resource_key_ref.as_bytes(),
        &allocation_value.to_be_bytes(),
    ])
}

pub(super) fn effect_candidate_digest(
    expected_predecessor_sequence: i64,
    expected_predecessor_digest: &[u8],
    purpose: &str,
    opaque_payload: &[u8],
    body: &EffectRecordBody,
) -> Vec<u8> {
    let body_bytes = body_preimage(body);
    digest_parts(&[
        b"mfm.pg-prototype.effect-candidate.v1",
        &expected_predecessor_sequence.to_be_bytes(),
        expected_predecessor_digest,
        purpose.as_bytes(),
        opaque_payload,
        &body_bytes,
    ])
}

pub(super) fn effect_record_digest(record: &EffectRecord) -> Vec<u8> {
    let predecessor = record.predecessor_digest.as_deref().unwrap_or_default();
    let append = append_preimage(record.append.as_ref());
    let body = body_preimage(&record.body);
    digest_parts(&[
        b"mfm.pg-prototype.effect-record.v1",
        record.stream_id.as_bytes(),
        &record.sequence.to_be_bytes(),
        predecessor,
        record.opaque_payload.as_slice(),
        record.executor_binding_ref.as_bytes(),
        &record.ledger_generation.to_be_bytes(),
        &record.effect_key,
        &record.request_digest,
        record.destination_identity.as_bytes(),
        &append,
        &body,
        &record.evidence_object_digest,
        &record.retained_bytes.to_be_bytes(),
    ])
}

pub(super) fn effect_proof_digest(record: &EffectRecord) -> Vec<u8> {
    digest_parts(&[
        b"mfm.pg-prototype.effect-proof.v1",
        record.executor_binding_ref.as_bytes(),
        &record.ledger_generation.to_be_bytes(),
        &record.commit_digest,
        &record.evidence_object_digest,
    ])
}

pub(super) fn resource_candidate_digest(record: &ResourceRecord) -> Vec<u8> {
    let predecessor = record
        .append
        .expected_predecessor_ref
        .as_deref()
        .unwrap_or_default();
    let fence = record.fencing_ref.as_deref().unwrap_or_default();
    digest_parts(&[
        b"mfm.pg-prototype.resource-candidate.v1",
        record.resource_ownership_ref.as_bytes(),
        record.policy_ref.as_bytes(),
        &record.policy_configuration_ref,
        &resource_policy_configuration_preimage(&record.policy_configuration),
        record.resource_key_ref.as_bytes(),
        &record.sequence.to_be_bytes(),
        &record.append.expected_predecessor_sequence.to_be_bytes(),
        predecessor,
        record.append.linked_effect_stream_id.as_bytes(),
        record.append.linked_effect_append_request_id.as_bytes(),
        &record.effect_key,
        &record.typed_allocation_state_ref,
        &record.allocation_value.to_be_bytes(),
        fence,
        record.executor_binding_ref.as_bytes(),
        &record.ledger_generation.to_be_bytes(),
    ])
}

pub(super) fn resource_record_ref(record: &ResourceRecord) -> Vec<u8> {
    let predecessor = record.predecessor_ref.as_deref().unwrap_or_default();
    let fence = record.fencing_ref.as_deref().unwrap_or_default();
    let append = resource_append_preimage(&record.append);
    digest_parts(&[
        b"mfm.pg-prototype.resource-record.v1",
        record.resource_ownership_ref.as_bytes(),
        record.policy_ref.as_bytes(),
        &record.policy_configuration_ref,
        &resource_policy_configuration_preimage(&record.policy_configuration),
        record.resource_key_ref.as_bytes(),
        &record.sequence.to_be_bytes(),
        predecessor,
        &append,
        &record.effect_key,
        &record.typed_allocation_state_ref,
        &record.allocation_value.to_be_bytes(),
        fence,
        record.executor_binding_ref.as_bytes(),
        &record.ledger_generation.to_be_bytes(),
    ])
}

pub(super) fn resource_proof_digest(record: &ResourceRecord) -> Vec<u8> {
    digest_parts(&[
        b"mfm.pg-prototype.resource-proof.v1",
        record.executor_binding_ref.as_bytes(),
        &record.ledger_generation.to_be_bytes(),
        &record.record_ref,
    ])
}

pub(super) fn retained_effect_record_bytes(record: &EffectRecord) -> i64 {
    let append_len = record.append.as_ref().map_or(0_usize, |append| {
        append.append_request_id.len()
            + append.expected_predecessor_digest.len()
            + append.purpose.len()
            + append.candidate_digest.len()
            + 16
    });
    let body_len = body_preimage(&record.body).len();
    i64::try_from(
        record.stream_id.len()
            + record.opaque_payload.len()
            + record.executor_binding_ref.len()
            + record.effect_key.len()
            + record.request_digest.len()
            + record.destination_identity.len()
            + record.evidence_object_digest.len()
            + append_len
            + body_len
            + 96,
    )
    .unwrap_or(i64::MAX)
}

pub(super) fn fold_effect(
    records: &[EffectRecord],
    contract: &EvidenceContract,
) -> Result<FoldedEffect, ModelError> {
    if !valid_contract(contract) {
        return Err(ModelError::EvidenceBounds);
    }
    let first = records.first().ok_or(ModelError::EffectIdentity)?;
    let effect_id = first
        .stream_id
        .strip_prefix(EFFECT_STREAM_PREFIX)
        .filter(|value| !value.is_empty())
        .ok_or(ModelError::EffectIdentity)?;
    validate_effect_id(effect_id)?;
    let effect_id = effect_id.to_owned();
    if !matches!(first.body, EffectRecordBody::EffectBound)
        || first.sequence != 1
        || first.predecessor_digest.is_some()
        || first.append.is_some()
        || first.effect_key
            != effect_key(
                &first.executor_binding_ref,
                first.ledger_generation,
                &effect_id,
            )
        || first.request_digest
            != request_digest(&first.destination_identity, &first.opaque_payload)
    {
        return Err(ModelError::EffectIdentity);
    }

    let mut expected_sequence = 1_i64;
    let mut predecessor: Option<Vec<u8>> = None;
    let mut allocation: Option<ResourceLink> = None;
    let mut state = ProjectedEffectState::Pending;
    let mut append_requests = Vec::new();
    let mut append_request_ids = BTreeSet::new();
    let mut attempts = BTreeMap::<Vec<u8>, (String, Option<AttemptOutcome>)>::new();
    let mut next_attempt_ordinal = 0_i64;
    let mut destination_account = None;
    let mut observation_count = 0_i64;
    let mut late_observation_count = 0_i64;
    let mut retained_bytes = 0_i64;
    let mut saw_tombstone = false;
    let mut previous_record: Option<&EffectRecord> = None;

    for record in records {
        if record.stream_id != first.stream_id
            || record.sequence != expected_sequence
            || record.predecessor_digest != predecessor
            || record.executor_binding_ref != first.executor_binding_ref
            || record.ledger_generation != first.ledger_generation
            || record.effect_key != first.effect_key
            || record.request_digest != first.request_digest
            || record.destination_identity != first.destination_identity
            || record.evidence_object_digest
                != sha256_digest_bytes(&record.opaque_payload)
                    .as_bytes()
                    .as_slice()
            || record.retained_bytes != retained_effect_record_bytes(record)
            || record.commit_digest != effect_record_digest(record)
            || record.proof_digest != effect_proof_digest(record)
        {
            return Err(ModelError::EffectChain);
        }
        if matches!(
            &record.body,
            EffectRecordBody::DeliveryAttemptObserved(_) | EffectRecordBody::TerminalTombstone(_)
        ) && record.retained_bytes > contract.completion_reserve_bytes
        {
            return Err(ModelError::EvidenceBounds);
        }
        if record.sequence == 1 {
            if !matches!(record.body, EffectRecordBody::EffectBound) || record.append.is_some() {
                return Err(ModelError::EffectAlgebra);
            }
        } else {
            let append = record.append.as_ref().ok_or(ModelError::AppendIdentity)?;
            let immediate_predecessor = append.expected_predecessor_sequence == record.sequence - 1
                && Some(append.expected_predecessor_digest.as_slice()) == predecessor.as_deref();
            let bundled_authorization =
                matches!(&record.body, EffectRecordBody::DeliveryAttemptAuthorized(_))
                    && previous_record.is_some_and(|previous| {
                        matches!(&previous.body, EffectRecordBody::ResourceAllocated(_))
                            && previous.append.as_ref().is_some_and(|allocation_append| {
                                append.expected_predecessor_sequence
                                    == allocation_append.expected_predecessor_sequence
                                    && append.expected_predecessor_digest
                                        == allocation_append.expected_predecessor_digest
                                    && allocation_append.append_request_id
                                        == bundled_resource_allocation_append_request_id(
                                            &effect_id,
                                            &append.append_request_id,
                                        )
                            })
                    });
            if append.append_request_id.is_empty()
                || !append_request_ids.insert(append.append_request_id.clone())
                || (!immediate_predecessor && !bundled_authorization)
                || append.purpose != record.body.kind()
                || append.candidate_digest
                    != effect_candidate_digest(
                        append.expected_predecessor_sequence,
                        &append.expected_predecessor_digest,
                        &append.purpose,
                        &record.opaque_payload,
                        &record.body,
                    )
            {
                return Err(ModelError::AppendIdentity);
            }
            append_requests.push(AppendProjection {
                stream_id: record.stream_id.clone(),
                append_request_id: append.append_request_id.clone(),
                predecessor_sequence: append.expected_predecessor_sequence,
                predecessor_digest: append.expected_predecessor_digest.clone(),
                purpose: append.purpose.clone(),
                candidate_digest: append.candidate_digest.clone(),
                committed_sequence: record.sequence,
                committed_digest: record.commit_digest.clone(),
            });
        }

        match &record.body {
            EffectRecordBody::EffectBound => {
                if record.sequence != 1 {
                    return Err(ModelError::EffectAlgebra);
                }
            }
            EffectRecordBody::ResourceAllocated(link) => {
                if allocation.is_some()
                    || next_attempt_ordinal != 0
                    || saw_tombstone
                    || link.resource_ownership_ref.is_empty()
                    || link.policy_ref.is_empty()
                    || link.policy_configuration_ref.is_empty()
                    || link.resource_key_ref.is_empty()
                    || link.typed_allocation_state_ref.is_empty()
                    || link.resource_append_request_id.is_empty()
                    || link.resource_record_ref.is_empty()
                {
                    return Err(ModelError::EffectAlgebra);
                }
                allocation = Some(link.clone());
            }
            EffectRecordBody::DeliveryAttemptAuthorized(authorization) => {
                if saw_tombstone
                    || authorization.target_operation_ref.is_empty()
                    || authorization.attempt_ordinal != next_attempt_ordinal
                    || authorization.attempt_id
                        != derive_attempt_id(
                            &record.executor_binding_ref,
                            record.ledger_generation,
                            &record.effect_key,
                            &record.request_digest,
                            authorization.attempt_ordinal,
                            &authorization.target_operation_ref,
                        )
                    || attempts.contains_key(&authorization.attempt_id)
                {
                    return Err(ModelError::EffectAlgebra);
                }
                attempts.insert(
                    authorization.attempt_id.clone(),
                    (authorization.target_operation_ref.clone(), None),
                );
                next_attempt_ordinal = next_attempt_ordinal
                    .checked_add(1)
                    .ok_or(ModelError::EvidenceBounds)?;
                state = ProjectedEffectState::Authorized;
            }
            EffectRecordBody::DeliveryAttemptObserved(observation) => {
                let (_, observed_outcome) = attempts
                    .get_mut(&observation.attempt_id)
                    .ok_or(ModelError::EffectAlgebra)?;
                if observed_outcome.is_some()
                    || observation.outcome.safe_outcome_ref().is_empty()
                    || observation
                        .outcome
                        .destination_account()
                        .is_some_and(|account| account <= 0)
                {
                    return Err(ModelError::EffectAlgebra);
                }
                if let Some(account) = observation.outcome.destination_account() {
                    if destination_account.is_some_and(|existing| existing != account) {
                        return Err(ModelError::EffectAlgebra);
                    }
                    destination_account = Some(account);
                }
                *observed_outcome = Some(observation.outcome.clone());
                observation_count = observation_count
                    .checked_add(1)
                    .ok_or(ModelError::EvidenceBounds)?;
                if saw_tombstone {
                    late_observation_count = late_observation_count
                        .checked_add(1)
                        .ok_or(ModelError::EvidenceBounds)?;
                } else {
                    state = ProjectedEffectState::Observed;
                }
            }
            EffectRecordBody::TerminalTombstone(tombstone) => {
                let matching_return = attempts.get(&tombstone.terminal_attempt_id).is_some_and(
                    |(target_operation_ref, outcome)| {
                        target_operation_ref == &tombstone.external_operation_ref
                            && matches!(
                                outcome,
                                Some(AttemptOutcome::Returned {
                                    safe_outcome_ref,
                                    ..
                                }) if safe_outcome_ref == &tombstone.terminal_outcome_ref
                            )
                    },
                );
                if saw_tombstone
                    || tombstone.external_operation_ref.is_empty()
                    || tombstone.terminal_outcome_ref.is_empty()
                    || tombstone.terminal_proof_ref
                        != terminal_proof_ref(
                            &tombstone.terminal_attempt_id,
                            &tombstone.external_operation_ref,
                            &tombstone.terminal_outcome_ref,
                        )
                    || !matching_return
                {
                    return Err(ModelError::EffectAlgebra);
                }
                saw_tombstone = true;
                state = ProjectedEffectState::Tombstoned;
            }
        }

        retained_bytes = retained_bytes
            .checked_add(record.retained_bytes)
            .ok_or(ModelError::EvidenceBounds)?;
        predecessor = Some(record.commit_digest.clone());
        previous_record = Some(record);
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or(ModelError::EvidenceBounds)?;
    }

    let unmatched_attempts = i64::try_from(
        attempts
            .values()
            .filter(|(_, outcome)| outcome.is_none())
            .count(),
    )
    .map_err(|_| ModelError::EvidenceBounds)?;
    let record_count = i64::try_from(records.len()).map_err(|_| ModelError::EvidenceBounds)?;
    let (reserve_records, reserve_bytes) =
        completion_reserve_debt(unmatched_attempts, !saw_tombstone, contract)?;
    let required_record_capacity = record_count
        .checked_add(reserve_records)
        .ok_or(ModelError::EvidenceBounds)?;
    let required_byte_capacity = retained_bytes
        .checked_add(reserve_bytes)
        .ok_or(ModelError::EvidenceBounds)?;
    if next_attempt_ordinal > contract.max_attempts
        || required_record_capacity > contract.max_records
        || required_byte_capacity > contract.max_retained_bytes
    {
        return Err(ModelError::EvidenceBounds);
    }
    let allocation = allocation.as_ref();
    let frontier_record = records.last().ok_or(ModelError::EffectChain)?;
    Ok(FoldedEffect {
        projection: EffectProjection {
            effect_id,
            stream_id: first.stream_id.clone(),
            effect_key: first.effect_key.clone(),
            request_digest: first.request_digest.clone(),
            destination_identity: first.destination_identity.clone(),
            state: state.as_str().to_owned(),
            resource_ownership_ref: allocation.map(|value| value.resource_ownership_ref.clone()),
            policy_ref: allocation.map(|value| value.policy_ref.clone()),
            policy_configuration_ref: allocation
                .map(|value| value.policy_configuration_ref.clone()),
            resource_key_ref: allocation.map(|value| value.resource_key_ref.clone()),
            allocation_state_ref: allocation.map(|value| value.typed_allocation_state_ref.clone()),
            allocation_value: allocation.map(|value| value.allocation_value),
            fencing_ref: allocation.and_then(|value| value.fencing_ref.clone()),
            resource_append_request_id: allocation
                .map(|value| value.resource_append_request_id.clone()),
            resource_record_ref: allocation.map(|value| value.resource_record_ref.clone()),
            destination_account,
            authorization_count: next_attempt_ordinal,
            observation_count,
            late_observation_count,
        },
        append_requests,
        frontier: EffectFrontier {
            stream_id: frontier_record.stream_id.clone(),
            executor_binding_ref: frontier_record.executor_binding_ref.clone(),
            ledger_generation: frontier_record.ledger_generation,
            effect_key: frontier_record.effect_key.clone(),
            request_digest: frontier_record.request_digest.clone(),
            destination_identity: frontier_record.destination_identity.clone(),
            sequence: frontier_record.sequence,
            predecessor_digest: frontier_record.predecessor_digest.clone(),
            evidence_object_digest: frontier_record.evidence_object_digest.clone(),
            retained_bytes: frontier_record.retained_bytes,
            commit_digest: frontier_record.commit_digest.clone(),
            proof_digest: frontier_record.proof_digest.clone(),
        },
        record_count,
        retained_bytes,
        unmatched_attempts,
    })
}

pub(super) fn validate_bound_resource_intent(
    folded: &FoldedEffect,
    intent: Option<&BoundResourceIntent>,
) -> Result<(), ModelError> {
    let allocation_absent = folded.projection.resource_ownership_ref.is_none()
        && folded.projection.resource_key_ref.is_none()
        && folded.projection.policy_ref.is_none()
        && folded.projection.policy_configuration_ref.is_none()
        && folded.projection.allocation_state_ref.is_none()
        && folded.projection.allocation_value.is_none()
        && folded.projection.fencing_ref.is_none()
        && folded.projection.resource_append_request_id.is_none()
        && folded.projection.resource_record_ref.is_none();
    match intent {
        None if allocation_absent => Ok(()),
        Some(_) if allocation_absent && folded.projection.authorization_count == 0 => Ok(()),
        Some(intent)
            if folded.projection.resource_ownership_ref.as_deref()
                == Some(intent.resource_ownership_ref.as_str())
                && folded.projection.resource_key_ref.as_deref()
                    == Some(intent.resource_key_ref.as_str())
                && folded.projection.policy_ref.as_deref() == Some(intent.policy_ref.as_str())
                && folded.projection.policy_configuration_ref.as_deref()
                    == Some(intent.policy_configuration_ref.as_slice()) =>
        {
            Ok(())
        }
        None | Some(_) => Err(ModelError::ResourceLink),
    }
}

pub(super) fn ensure_authorization_capacity(
    folded: &FoldedEffect,
    contract: &EvidenceContract,
    next_record_bytes: i64,
) -> Result<(), ModelError> {
    if !valid_contract(contract)
        || next_record_bytes <= 0
        || folded.projection.state == ProjectedEffectState::Tombstoned.as_str()
        || folded.projection.authorization_count >= contract.max_attempts
    {
        return Err(ModelError::EvidenceBounds);
    }
    let projected_records = folded
        .record_count
        .checked_add(1)
        .ok_or(ModelError::EvidenceBounds)?;
    let unmatched_after = folded
        .unmatched_attempts
        .checked_add(1)
        .ok_or(ModelError::EvidenceBounds)?;
    let (reserve_records, reserve_bytes) =
        completion_reserve_debt(unmatched_after, true, contract)?;
    let required_record_capacity = projected_records
        .checked_add(reserve_records)
        .ok_or(ModelError::EvidenceBounds)?;
    let projected_bytes = folded
        .retained_bytes
        .checked_add(next_record_bytes)
        .ok_or(ModelError::EvidenceBounds)?;
    let required_byte_capacity = projected_bytes
        .checked_add(reserve_bytes)
        .ok_or(ModelError::EvidenceBounds)?;
    if required_record_capacity > contract.max_records
        || required_byte_capacity > contract.max_retained_bytes
    {
        return Err(ModelError::EvidenceBounds);
    }
    Ok(())
}

fn completion_reserve_debt(
    unmatched_attempts: i64,
    tombstone_absent: bool,
    contract: &EvidenceContract,
) -> Result<(i64, i64), ModelError> {
    // Debt is cumulative: every unmatched authorization needs its own observation record, and an
    // absent terminal tombstone needs one more. The contract's fixed value of two records is only
    // the structural minimum for one newly authorized attempt (one observation plus one tombstone).
    let tombstone_records = if tombstone_absent { 1 } else { 0 };
    let records = unmatched_attempts
        .checked_add(tombstone_records)
        .ok_or(ModelError::EvidenceBounds)?;
    let bytes = records
        .checked_mul(contract.completion_reserve_bytes)
        .ok_or(ModelError::EvidenceBounds)?;
    Ok((records, bytes))
}

fn valid_contract(contract: &EvidenceContract) -> bool {
    !contract.contract_ref.is_empty()
        && contract.max_attempts >= 0
        && contract.max_records > 0
        && contract.max_retained_bytes > 0
        && contract.completion_reserve_records == 2
        && contract.completion_reserve_bytes > 0
}

pub(super) fn fold_resource(records: &[ResourceRecord]) -> Result<FoldedResource, ModelError> {
    let first = records.first().ok_or(ModelError::ResourceChain)?;
    let mut expected_sequence = 1_i64;
    let mut predecessor = None;
    let mut effects = BTreeSet::new();
    let mut append_request_ids = BTreeSet::new();
    for record in records {
        if record.resource_ownership_ref != first.resource_ownership_ref
            || record.policy_ref != first.policy_ref
            || record.policy_configuration != first.policy_configuration
            || record.policy_configuration_ref != first.policy_configuration_ref
            || record.resource_key_ref != first.resource_key_ref
            || record.executor_binding_ref != first.executor_binding_ref
            || record.ledger_generation != first.ledger_generation
            || record.sequence != expected_sequence
            || record.predecessor_ref != predecessor
            || record.policy_ref != record.policy_configuration.policy_ref()
            || record.policy_configuration_ref
                != resource_policy_configuration_ref(&record.policy_configuration)
            || record.append.append_request_id.is_empty()
            || !append_request_ids.insert(record.append.append_request_id.clone())
            || record.append.expected_predecessor_sequence != record.sequence - 1
            || record.append.expected_predecessor_ref != predecessor
            || record
                .append
                .linked_effect_stream_id
                .strip_prefix(EFFECT_STREAM_PREFIX)
                .is_none_or(str::is_empty)
            || record.append.linked_effect_append_request_id.is_empty()
            || record.append.candidate_digest != resource_candidate_digest(record)
            || record.typed_allocation_state_ref
                != allocation_state_ref(
                    &record.policy_ref,
                    &record.policy_configuration_ref,
                    &record.resource_key_ref,
                    record.allocation_value,
                )
            || record.record_ref != resource_record_ref(record)
            || record.proof_digest != resource_proof_digest(record)
            || !effects.insert(record.effect_key.clone())
        {
            return Err(ModelError::ResourceChain);
        }
        match &record.policy_configuration {
            ResourcePolicyConfiguration::FiniteInventory {
                ordered_allocation_values,
            } => {
                let unique_values = ordered_allocation_values.iter().collect::<BTreeSet<_>>();
                let index =
                    usize::try_from(record.sequence - 1).map_err(|_| ModelError::ResourcePolicy)?;
                if ordered_allocation_values.is_empty()
                    || unique_values.len() != ordered_allocation_values.len()
                    || ordered_allocation_values.get(index) != Some(&record.allocation_value)
                {
                    return Err(ModelError::ResourcePolicy);
                }
            }
            ResourcePolicyConfiguration::AccountSequence { initial_value } => {
                let expected_value = initial_value
                    .checked_add(record.sequence - 1)
                    .ok_or(ModelError::ResourcePolicy)?;
                if record.allocation_value != expected_value {
                    return Err(ModelError::ResourcePolicy);
                }
            }
            ResourcePolicyConfiguration::Exclusive => {
                if record.sequence != 1 {
                    return Err(ModelError::ResourcePolicy);
                }
            }
        }
        predecessor = Some(record.record_ref.clone());
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or(ModelError::ResourceChain)?;
    }
    Ok(FoldedResource {
        head: ResourceHeadProjection {
            resource_ownership_ref: first.resource_ownership_ref.clone(),
            policy_ref: first.policy_ref.clone(),
            policy_configuration_ref: first.policy_configuration_ref.clone(),
            resource_key_ref: first.resource_key_ref.clone(),
            sequence: expected_sequence - 1,
            record_ref: predecessor.ok_or(ModelError::ResourceChain)?,
        },
        records: records.to_vec(),
    })
}

pub(super) fn validate_resource_links(
    effects: &[FoldedEffect],
    resources: &[FoldedResource],
) -> Result<(), ModelError> {
    let mut resource_by_ref = BTreeMap::new();
    for resource in resources {
        for record in &resource.records {
            if resource_by_ref
                .insert(record.record_ref.clone(), record)
                .is_some()
            {
                return Err(ModelError::ResourceLink);
            }
        }
    }
    let mut linked_records = BTreeSet::new();
    for effect in effects {
        let Some(resource_ref) = effect.projection.resource_record_ref.as_ref() else {
            continue;
        };
        let record = resource_by_ref
            .get(resource_ref)
            .ok_or(ModelError::ResourceLink)?;
        let allocation_append = effect
            .append_requests
            .iter()
            .filter(|append| append.purpose == "resource_allocated")
            .collect::<Vec<_>>();
        if record.effect_key != effect.projection.effect_key
            || Some(&record.resource_ownership_ref)
                != effect.projection.resource_ownership_ref.as_ref()
            || Some(&record.policy_ref) != effect.projection.policy_ref.as_ref()
            || Some(&record.policy_configuration_ref)
                != effect.projection.policy_configuration_ref.as_ref()
            || Some(&record.resource_key_ref) != effect.projection.resource_key_ref.as_ref()
            || Some(&record.typed_allocation_state_ref)
                != effect.projection.allocation_state_ref.as_ref()
            || Some(record.allocation_value) != effect.projection.allocation_value
            || record.fencing_ref != effect.projection.fencing_ref
            || Some(&record.append.append_request_id)
                != effect.projection.resource_append_request_id.as_ref()
            || record.append.linked_effect_stream_id != effect.projection.stream_id
            || allocation_append.len() != 1
            || allocation_append[0].append_request_id
                != record.append.linked_effect_append_request_id
            || !linked_records.insert(resource_ref.clone())
        {
            return Err(ModelError::ResourceLink);
        }
    }
    if linked_records.len() != resource_by_ref.len() {
        return Err(ModelError::ResourceLink);
    }
    Ok(())
}

fn append_preimage(append: Option<&AppendIdentity>) -> Vec<u8> {
    let Some(append) = append else {
        return vec![0];
    };
    let mut bytes = vec![1];
    push_part(&mut bytes, append.append_request_id.as_bytes());
    bytes.extend_from_slice(&append.expected_predecessor_sequence.to_be_bytes());
    push_part(&mut bytes, &append.expected_predecessor_digest);
    push_part(&mut bytes, append.purpose.as_bytes());
    push_part(&mut bytes, &append.candidate_digest);
    bytes
}

fn resource_append_preimage(append: &ResourceAppendIdentity) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_part(&mut bytes, append.append_request_id.as_bytes());
    bytes.extend_from_slice(&append.expected_predecessor_sequence.to_be_bytes());
    push_optional(&mut bytes, append.expected_predecessor_ref.as_deref());
    push_part(&mut bytes, append.linked_effect_stream_id.as_bytes());
    push_part(
        &mut bytes,
        append.linked_effect_append_request_id.as_bytes(),
    );
    push_part(&mut bytes, &append.candidate_digest);
    bytes
}

fn resource_policy_configuration_preimage(configuration: &ResourcePolicyConfiguration) -> Vec<u8> {
    let mut bytes = Vec::new();
    match configuration {
        ResourcePolicyConfiguration::FiniteInventory {
            ordered_allocation_values,
        } => {
            bytes.push(0);
            bytes.extend_from_slice(
                &u64::try_from(ordered_allocation_values.len())
                    .unwrap_or(u64::MAX)
                    .to_be_bytes(),
            );
            for value in ordered_allocation_values {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
        }
        ResourcePolicyConfiguration::AccountSequence { initial_value } => {
            bytes.push(1);
            bytes.extend_from_slice(&initial_value.to_be_bytes());
        }
        ResourcePolicyConfiguration::Exclusive => bytes.push(2),
    }
    bytes
}

fn body_preimage(body: &EffectRecordBody) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_part(&mut bytes, body.kind().as_bytes());
    match body {
        EffectRecordBody::EffectBound => {}
        EffectRecordBody::ResourceAllocated(link) => {
            push_part(&mut bytes, link.resource_ownership_ref.as_bytes());
            push_part(&mut bytes, link.policy_ref.as_bytes());
            push_part(&mut bytes, &link.policy_configuration_ref);
            push_part(&mut bytes, link.resource_key_ref.as_bytes());
            push_part(&mut bytes, &link.typed_allocation_state_ref);
            bytes.extend_from_slice(&link.allocation_value.to_be_bytes());
            push_optional(&mut bytes, link.fencing_ref.as_deref());
            push_part(&mut bytes, link.resource_append_request_id.as_bytes());
            push_part(&mut bytes, &link.resource_record_ref);
        }
        EffectRecordBody::DeliveryAttemptAuthorized(authorization) => {
            bytes.extend_from_slice(&authorization.attempt_ordinal.to_be_bytes());
            push_part(&mut bytes, &authorization.attempt_id);
            push_part(&mut bytes, authorization.target_operation_ref.as_bytes());
        }
        EffectRecordBody::DeliveryAttemptObserved(observation) => {
            push_part(&mut bytes, &observation.attempt_id);
            match &observation.outcome {
                AttemptOutcome::Returned {
                    safe_outcome_ref,
                    destination_account,
                } => {
                    bytes.push(0);
                    push_part(&mut bytes, safe_outcome_ref);
                    bytes.extend_from_slice(&destination_account.to_be_bytes());
                }
                AttemptOutcome::DidNotEnter { safe_outcome_ref } => {
                    bytes.push(1);
                    push_part(&mut bytes, safe_outcome_ref);
                }
                AttemptOutcome::Indeterminate { safe_outcome_ref } => {
                    bytes.push(2);
                    push_part(&mut bytes, safe_outcome_ref);
                }
            }
        }
        EffectRecordBody::TerminalTombstone(tombstone) => {
            push_part(&mut bytes, &tombstone.terminal_attempt_id);
            push_part(&mut bytes, tombstone.external_operation_ref.as_bytes());
            push_part(&mut bytes, &tombstone.terminal_outcome_ref);
            push_part(&mut bytes, &tombstone.terminal_proof_ref);
        }
    }
    bytes
}

fn digest_parts(parts: &[&[u8]]) -> Vec<u8> {
    let total = parts.iter().fold(0_usize, |total, part| {
        total.saturating_add(part.len()).saturating_add(8)
    });
    let mut preimage = Vec::with_capacity(total);
    for part in parts {
        push_part(&mut preimage, part);
    }
    sha256_digest_bytes(&preimage).as_bytes().to_vec()
}

fn push_part(bytes: &mut Vec<u8>, part: &[u8]) {
    bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
    bytes.extend_from_slice(part);
}

fn push_optional(bytes: &mut Vec<u8>, part: Option<&[u8]>) {
    match part {
        Some(part) => {
            bytes.push(1);
            push_part(bytes, part);
        }
        None => bytes.push(0),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    const BINDING_REF: &str = "executor-binding:test:v1";
    const GENERATION: i64 = 7;
    const DESTINATION_IDENTITY: &str = "destination:test:v1";
    const COMPLETION_RECORD_BYTES: i64 = 2_048;
    const PROTOTYPE_COMPLETION_RECORD_BYTES: i64 = 16 * 1_024;

    fn contract(max_records: i64, max_retained_bytes: i64) -> EvidenceContract {
        EvidenceContract {
            contract_ref: "evidence-contract:test:v1".to_owned(),
            max_attempts: 8,
            max_records,
            max_retained_bytes,
            completion_reserve_records: 2,
            completion_reserve_bytes: COMPLETION_RECORD_BYTES,
        }
    }

    fn generous_contract() -> EvidenceContract {
        contract(64, 1_000_000)
    }

    fn seal_effect(mut record: EffectRecord) -> EffectRecord {
        record.evidence_object_digest = sha256_digest_bytes(&record.opaque_payload)
            .as_bytes()
            .to_vec();
        record.retained_bytes = retained_effect_record_bytes(&record);
        record.commit_digest = effect_record_digest(&record);
        record.proof_digest = effect_proof_digest(&record);
        record
    }

    fn refresh_effect_candidate(record: &mut EffectRecord) {
        let append = record.append.as_mut().expect("non-genesis append");
        append.candidate_digest = effect_candidate_digest(
            append.expected_predecessor_sequence,
            &append.expected_predecessor_digest,
            &append.purpose,
            &record.opaque_payload,
            &record.body,
        );
    }

    fn bound(effect_id: &str) -> EffectRecord {
        let opaque_payload = format!("semantic-request:{effect_id}").into_bytes();
        seal_effect(EffectRecord {
            stream_id: effect_stream_id(effect_id),
            sequence: 1,
            predecessor_digest: None,
            opaque_payload: opaque_payload.clone(),
            executor_binding_ref: BINDING_REF.to_owned(),
            ledger_generation: GENERATION,
            effect_key: effect_key(BINDING_REF, GENERATION, effect_id),
            request_digest: request_digest(DESTINATION_IDENTITY, &opaque_payload),
            destination_identity: DESTINATION_IDENTITY.to_owned(),
            append: None,
            body: EffectRecordBody::EffectBound,
            evidence_object_digest: Vec::new(),
            retained_bytes: 0,
            commit_digest: Vec::new(),
            proof_digest: Vec::new(),
        })
    }

    fn append_effect(
        previous: &EffectRecord,
        append_request_id: &str,
        body: EffectRecordBody,
    ) -> EffectRecord {
        let opaque_payload = format!("opaque:{append_request_id}").into_bytes();
        let purpose = body.kind().to_owned();
        let expected_predecessor_sequence = previous.sequence;
        let expected_predecessor_digest = previous.commit_digest.clone();
        let candidate_digest = effect_candidate_digest(
            expected_predecessor_sequence,
            &expected_predecessor_digest,
            &purpose,
            &opaque_payload,
            &body,
        );
        seal_effect(EffectRecord {
            stream_id: previous.stream_id.clone(),
            sequence: previous.sequence + 1,
            predecessor_digest: Some(previous.commit_digest.clone()),
            opaque_payload,
            executor_binding_ref: previous.executor_binding_ref.clone(),
            ledger_generation: previous.ledger_generation,
            effect_key: previous.effect_key.clone(),
            request_digest: previous.request_digest.clone(),
            destination_identity: previous.destination_identity.clone(),
            append: Some(AppendIdentity {
                append_request_id: append_request_id.to_owned(),
                expected_predecessor_sequence,
                expected_predecessor_digest,
                purpose,
                candidate_digest,
            }),
            body,
            evidence_object_digest: Vec::new(),
            retained_bytes: 0,
            commit_digest: Vec::new(),
            proof_digest: Vec::new(),
        })
    }

    fn authorization(
        previous: &EffectRecord,
        append_request_id: &str,
        attempt_ordinal: i64,
        target_operation_ref: &str,
    ) -> EffectRecord {
        let attempt_id = derive_attempt_id(
            &previous.executor_binding_ref,
            previous.ledger_generation,
            &previous.effect_key,
            &previous.request_digest,
            attempt_ordinal,
            target_operation_ref,
        );
        append_effect(
            previous,
            append_request_id,
            EffectRecordBody::DeliveryAttemptAuthorized(AttemptAuthorization {
                attempt_ordinal,
                attempt_id,
                target_operation_ref: target_operation_ref.to_owned(),
            }),
        )
    }

    fn observation(
        previous: &EffectRecord,
        append_request_id: &str,
        attempt_id: Vec<u8>,
        outcome: AttemptOutcome,
    ) -> EffectRecord {
        append_effect(
            previous,
            append_request_id,
            EffectRecordBody::DeliveryAttemptObserved(AttemptObservation {
                attempt_id,
                outcome,
            }),
        )
    }

    fn tombstone(
        previous: &EffectRecord,
        append_request_id: &str,
        terminal_attempt_id: Vec<u8>,
        target_operation_ref: &str,
        terminal_outcome_ref: Vec<u8>,
    ) -> EffectRecord {
        let terminal_proof_ref = terminal_proof_ref(
            &terminal_attempt_id,
            target_operation_ref,
            &terminal_outcome_ref,
        );
        append_effect(
            previous,
            append_request_id,
            EffectRecordBody::TerminalTombstone(Tombstone {
                terminal_attempt_id,
                external_operation_ref: target_operation_ref.to_owned(),
                terminal_outcome_ref,
                terminal_proof_ref,
            }),
        )
    }

    fn seal_resource(mut record: ResourceRecord) -> ResourceRecord {
        record.append.candidate_digest = resource_candidate_digest(&record);
        record.record_ref = resource_record_ref(&record);
        record.proof_digest = resource_proof_digest(&record);
        record
    }

    fn resource_record(
        previous: Option<&ResourceRecord>,
        effect: &EffectRecord,
        configuration: ResourcePolicyConfiguration,
        allocation_value: i64,
        resource_append_request_id: &str,
        linked_effect_append_request_id: &str,
    ) -> ResourceRecord {
        let sequence = previous.map_or(1, |record| record.sequence + 1);
        let predecessor_ref = previous.map(|record| record.record_ref.clone());
        let policy_ref = configuration.policy_ref().to_owned();
        let policy_configuration_ref = resource_policy_configuration_ref(&configuration);
        let resource_key_ref = "resource-key:test".to_owned();
        seal_resource(ResourceRecord {
            resource_ownership_ref: "resource-owner:test".to_owned(),
            policy_ref: policy_ref.clone(),
            policy_configuration: configuration,
            policy_configuration_ref: policy_configuration_ref.clone(),
            resource_key_ref: resource_key_ref.clone(),
            sequence,
            predecessor_ref: predecessor_ref.clone(),
            append: ResourceAppendIdentity {
                append_request_id: resource_append_request_id.to_owned(),
                expected_predecessor_sequence: sequence - 1,
                expected_predecessor_ref: predecessor_ref,
                linked_effect_stream_id: effect.stream_id.clone(),
                linked_effect_append_request_id: linked_effect_append_request_id.to_owned(),
                candidate_digest: Vec::new(),
            },
            effect_key: effect.effect_key.clone(),
            typed_allocation_state_ref: allocation_state_ref(
                &policy_ref,
                &policy_configuration_ref,
                &resource_key_ref,
                allocation_value,
            ),
            allocation_value,
            fencing_ref: Some(b"resource-fence:test".to_vec()),
            executor_binding_ref: effect.executor_binding_ref.clone(),
            ledger_generation: effect.ledger_generation,
            record_ref: Vec::new(),
            proof_digest: Vec::new(),
        })
    }

    fn resource_link(record: &ResourceRecord) -> ResourceLink {
        ResourceLink {
            resource_ownership_ref: record.resource_ownership_ref.clone(),
            policy_ref: record.policy_ref.clone(),
            policy_configuration_ref: record.policy_configuration_ref.clone(),
            resource_key_ref: record.resource_key_ref.clone(),
            typed_allocation_state_ref: record.typed_allocation_state_ref.clone(),
            allocation_value: record.allocation_value,
            fencing_ref: record.fencing_ref.clone(),
            resource_append_request_id: record.append.append_request_id.clone(),
            resource_record_ref: record.record_ref.clone(),
        }
    }

    fn allocated_effect(
        bound_record: &EffectRecord,
        configuration: ResourcePolicyConfiguration,
        allocation_value: i64,
    ) -> (ResourceRecord, EffectRecord) {
        let resource = resource_record(
            None,
            bound_record,
            configuration,
            allocation_value,
            "resource-append:0",
            "effect-append:allocation",
        );
        let allocation = append_effect(
            bound_record,
            "effect-append:allocation",
            EffectRecordBody::ResourceAllocated(resource_link(&resource)),
        );
        (resource, allocation)
    }

    fn attempt_id(record: &EffectRecord) -> Vec<u8> {
        match &record.body {
            EffectRecordBody::DeliveryAttemptAuthorized(authorization) => {
                authorization.attempt_id.clone()
            }
            _ => panic!("expected authorization"),
        }
    }

    #[test]
    fn folds_five_immutable_variants_and_proves_the_frontier() {
        let bound_record = bound("complete");
        let (resource, allocation) = allocated_effect(
            &bound_record,
            ResourcePolicyConfiguration::FiniteInventory {
                ordered_allocation_values: vec![10_001, 10_002],
            },
            10_001,
        );
        let authorized = authorization(&allocation, "effect-append:authorize:0", 0, "operation:0");
        let returned_ref = b"returned:0".to_vec();
        let observed = observation(
            &authorized,
            "effect-append:observe:0",
            attempt_id(&authorized),
            AttemptOutcome::Returned {
                safe_outcome_ref: returned_ref.clone(),
                destination_account: 10_001,
            },
        );
        let terminal = tombstone(
            &observed,
            "effect-append:tombstone",
            attempt_id(&authorized),
            "operation:0",
            returned_ref,
        );
        let records = vec![
            bound_record,
            allocation,
            authorized,
            observed,
            terminal.clone(),
        ];

        let folded = fold_effect(&records, &generous_contract()).expect("fold effect");
        assert_eq!(folded.projection.state, "tombstoned");
        assert_eq!(folded.projection.authorization_count, 1);
        assert_eq!(folded.projection.observation_count, 1);
        assert_eq!(folded.projection.late_observation_count, 0);
        assert_eq!(folded.projection.destination_account, Some(10_001));
        assert_eq!(folded.unmatched_attempts, 0);
        assert_eq!(folded.append_requests.len(), 4);

        assert_eq!(folded.frontier.stream_id, terminal.stream_id);
        assert_eq!(
            folded.frontier.executor_binding_ref,
            terminal.executor_binding_ref
        );
        assert_eq!(
            folded.frontier.ledger_generation,
            terminal.ledger_generation
        );
        assert_eq!(folded.frontier.effect_key, terminal.effect_key);
        assert_eq!(folded.frontier.request_digest, terminal.request_digest);
        assert_eq!(
            folded.frontier.destination_identity,
            terminal.destination_identity
        );
        assert_eq!(folded.frontier.sequence, terminal.sequence);
        assert_eq!(
            folded.frontier.predecessor_digest,
            terminal.predecessor_digest
        );
        assert_eq!(
            folded.frontier.evidence_object_digest,
            terminal.evidence_object_digest
        );
        assert_eq!(folded.frontier.retained_bytes, terminal.retained_bytes);
        assert_eq!(folded.frontier.commit_digest, terminal.commit_digest);
        assert_eq!(folded.frontier.proof_digest, terminal.proof_digest);

        let folded_resource = fold_resource(std::slice::from_ref(&resource))
            .expect("fold permanently retained resource");
        assert_eq!(folded_resource.records, vec![resource.clone()]);
        assert_eq!(folded_resource.head.sequence, 1);
        assert_eq!(
            folded_resource.head.policy_configuration_ref,
            resource.policy_configuration_ref
        );
        validate_resource_links(&[folded], &[folded_resource])
            .expect("effect and resource atomic links");
        assert!(
            resource_stream_id(&resource.resource_ownership_ref, &resource.resource_key_ref)
                .starts_with(RESOURCE_STREAM_PREFIX)
        );
    }

    #[test]
    fn resealed_wrong_terminal_proof_is_rejected() {
        let bound_record = bound("wrong-terminal-proof");
        let (_, allocation) =
            allocated_effect(&bound_record, ResourcePolicyConfiguration::Exclusive, 1);
        let authorized = authorization(&allocation, "effect-append:authorize", 0, "operation:0");
        let returned_ref = b"returned:wrong-proof".to_vec();
        let observed = observation(
            &authorized,
            "effect-append:observe",
            attempt_id(&authorized),
            AttemptOutcome::Returned {
                safe_outcome_ref: returned_ref.clone(),
                destination_account: 77,
            },
        );
        let mut wrong_terminal = tombstone(
            &observed,
            "effect-append:tombstone",
            attempt_id(&authorized),
            "operation:0",
            returned_ref,
        );
        match &mut wrong_terminal.body {
            EffectRecordBody::TerminalTombstone(tombstone) => {
                tombstone.terminal_proof_ref = vec![0; 32];
            }
            _ => unreachable!("terminal helper returned another variant"),
        }
        refresh_effect_candidate(&mut wrong_terminal);
        let wrong_terminal = seal_effect(wrong_terminal);
        assert!(matches!(
            fold_effect(
                &[
                    bound_record,
                    allocation,
                    authorized,
                    observed,
                    wrong_terminal,
                ],
                &generous_contract()
            ),
            Err(ModelError::EffectAlgebra)
        ));
    }

    #[test]
    fn bound_resource_intent_closes_allocation_and_authorization() {
        let bound_record = bound("bound-intent");
        let configuration = ResourcePolicyConfiguration::Exclusive;
        let (_, allocation) = allocated_effect(&bound_record, configuration.clone(), 1);
        let allocated = fold_effect(&[bound_record.clone(), allocation], &generous_contract())
            .expect("fold allocated effect");
        let exact_intent = BoundResourceIntent {
            resource_ownership_ref: "resource-owner:test".to_owned(),
            resource_key_ref: "resource-key:test".to_owned(),
            policy_ref: configuration.policy_ref().to_owned(),
            policy_configuration_ref: resource_policy_configuration_ref(&configuration),
        };
        validate_bound_resource_intent(&allocated, Some(&exact_intent))
            .expect("exact bound intent");
        assert!(matches!(
            validate_bound_resource_intent(&allocated, None),
            Err(ModelError::ResourceLink)
        ));

        let mut bind_a_allocate_b = exact_intent.clone();
        bind_a_allocate_b.resource_key_ref = "resource-key:A".to_owned();
        assert!(matches!(
            validate_bound_resource_intent(&allocated, Some(&bind_a_allocate_b)),
            Err(ModelError::ResourceLink)
        ));

        let pending = fold_effect(std::slice::from_ref(&bound_record), &generous_contract())
            .expect("fold pending bound intent");
        validate_bound_resource_intent(&pending, Some(&exact_intent))
            .expect("bound intent may await allocation before authorization");

        let authorized = authorization(
            &bound_record,
            "effect-append:authorize-without-allocation",
            0,
            "operation:bound-intent",
        );
        let authorized_without_allocation =
            fold_effect(&[bound_record, authorized], &generous_contract())
                .expect("fold structurally valid authorization");
        assert!(matches!(
            validate_bound_resource_intent(&authorized_without_allocation, Some(&exact_intent)),
            Err(ModelError::ResourceLink)
        ));
    }

    #[test]
    fn tombstone_allows_one_late_observation_per_pre_tombstone_attempt() {
        let bound_record = bound("late-observation");
        let (_, allocation) =
            allocated_effect(&bound_record, ResourcePolicyConfiguration::Exclusive, 1);
        let first_authorized =
            authorization(&allocation, "effect-append:authorize:0", 0, "operation:0");
        let returned_ref = b"returned:0".to_vec();
        let first_observed = observation(
            &first_authorized,
            "effect-append:observe:0",
            attempt_id(&first_authorized),
            AttemptOutcome::Returned {
                safe_outcome_ref: returned_ref.clone(),
                destination_account: 44,
            },
        );
        let second_authorized = authorization(
            &first_observed,
            "effect-append:authorize:1",
            1,
            "operation:1",
        );
        let third_authorized = authorization(
            &second_authorized,
            "effect-append:authorize:2",
            2,
            "operation:2",
        );
        let terminal = tombstone(
            &third_authorized,
            "effect-append:tombstone",
            attempt_id(&first_authorized),
            "operation:0",
            returned_ref,
        );
        let first_late_observed = observation(
            &terminal,
            "effect-append:late-observe:1",
            attempt_id(&second_authorized),
            AttemptOutcome::DidNotEnter {
                safe_outcome_ref: b"did-not-enter:1".to_vec(),
            },
        );
        let second_late_observed = observation(
            &first_late_observed,
            "effect-append:late-observe:2",
            attempt_id(&third_authorized),
            AttemptOutcome::Indeterminate {
                safe_outcome_ref: b"indeterminate:2".to_vec(),
            },
        );
        let records = vec![
            bound_record.clone(),
            allocation.clone(),
            first_authorized.clone(),
            first_observed.clone(),
            second_authorized.clone(),
            third_authorized.clone(),
            terminal.clone(),
            first_late_observed.clone(),
            second_late_observed.clone(),
        ];

        let folded =
            fold_effect(&records, &generous_contract()).expect("fold two late observations");
        assert_eq!(folded.projection.state, "tombstoned");
        assert_eq!(folded.projection.observation_count, 3);
        assert_eq!(folded.projection.late_observation_count, 2);
        assert_eq!(folded.projection.destination_account, Some(44));
        assert_eq!(folded.unmatched_attempts, 0);

        let mut one_late_records = records[..7].to_vec();
        one_late_records.push(first_late_observed.clone());
        let one_late_fold =
            fold_effect(&one_late_records, &generous_contract()).expect("one late observation");
        assert_eq!(one_late_fold.projection.state, "tombstoned");
        assert_eq!(one_late_fold.projection.late_observation_count, 1);
        assert_eq!(one_late_fold.unmatched_attempts, 1);

        let duplicate_late = observation(
            &second_late_observed,
            "effect-append:duplicate-late-observe",
            attempt_id(&second_authorized),
            AttemptOutcome::Indeterminate {
                safe_outcome_ref: b"indeterminate:1".to_vec(),
            },
        );
        let mut duplicate_late_records = records.clone();
        duplicate_late_records.push(duplicate_late);
        assert!(matches!(
            fold_effect(&duplicate_late_records, &generous_contract()),
            Err(ModelError::EffectAlgebra)
        ));

        let terminal_records = vec![
            bound_record.clone(),
            allocation.clone(),
            first_authorized.clone(),
            first_observed.clone(),
            second_authorized.clone(),
            third_authorized.clone(),
            terminal.clone(),
        ];
        let terminal_fold =
            fold_effect(&terminal_records, &generous_contract()).expect("terminal with debt");
        assert_eq!(terminal_fold.unmatched_attempts, 2);
        let exact_terminal_contract = contract(
            terminal_fold.record_count + 2,
            terminal_fold.retained_bytes + 2 * COMPLETION_RECORD_BYTES,
        );
        fold_effect(&terminal_records, &exact_terminal_contract)
            .expect("exact loaded late-observation reserve");
        assert!(matches!(
            fold_effect(
                &terminal_records,
                &contract(
                    exact_terminal_contract.max_records - 1,
                    exact_terminal_contract.max_retained_bytes
                )
            ),
            Err(ModelError::EvidenceBounds)
        ));
        assert!(matches!(
            fold_effect(
                &terminal_records,
                &contract(
                    exact_terminal_contract.max_records,
                    exact_terminal_contract.max_retained_bytes - 1
                )
            ),
            Err(ModelError::EvidenceBounds)
        ));

        let post_terminal_authorization =
            authorization(&terminal, "effect-append:authorize:3", 3, "operation:3");
        let mut post_terminal_records = terminal_records.clone();
        post_terminal_records.push(post_terminal_authorization);
        assert!(matches!(
            fold_effect(&post_terminal_records, &generous_contract()),
            Err(ModelError::EffectAlgebra)
        ));

        let wrong_terminal = tombstone(
            &third_authorized,
            "effect-append:wrong-tombstone",
            attempt_id(&second_authorized),
            "operation:1",
            b"did-not-return".to_vec(),
        );
        let mut wrong_terminal_records = terminal_records[..6].to_vec();
        wrong_terminal_records.push(wrong_terminal);
        assert!(matches!(
            fold_effect(&wrong_terminal_records, &generous_contract()),
            Err(ModelError::EffectAlgebra)
        ));

        let indeterminate = observation(
            &third_authorized,
            "effect-append:indeterminate",
            attempt_id(&second_authorized),
            AttemptOutcome::Indeterminate {
                safe_outcome_ref: b"indeterminate:1".to_vec(),
            },
        );
        let mut indeterminate_records = terminal_records[..6].to_vec();
        indeterminate_records.push(indeterminate);
        let indeterminate_fold =
            fold_effect(&indeterminate_records, &generous_contract()).expect("closed outcome");
        assert_eq!(indeterminate_fold.projection.state, "observed");
        assert_eq!(indeterminate_fold.projection.destination_account, Some(44));
    }

    #[test]
    fn resource_policy_configuration_and_atomic_append_links_are_authenticated() {
        let first_effect = bound("resource-first");
        let second_effect = bound("resource-second");
        let finite_configuration = ResourcePolicyConfiguration::FiniteInventory {
            ordered_allocation_values: vec![10_009, 10_003],
        };
        let first_finite = resource_record(
            None,
            &first_effect,
            finite_configuration.clone(),
            10_009,
            "resource-append:0",
            "effect-append:allocation:0",
        );
        let second_finite = resource_record(
            Some(&first_finite),
            &second_effect,
            finite_configuration.clone(),
            10_003,
            "resource-append:1",
            "effect-append:allocation:1",
        );
        let folded_finite = fold_resource(&[first_finite.clone(), second_finite.clone()])
            .expect("configured finite order");
        assert_eq!(folded_finite.records.len(), 2);
        assert_eq!(folded_finite.head.sequence, 2);

        let exhausted = resource_record(
            Some(&second_finite),
            &bound("resource-third"),
            finite_configuration,
            10_011,
            "resource-append:2",
            "effect-append:allocation:2",
        );
        assert!(matches!(
            fold_resource(&[first_finite.clone(), second_finite.clone(), exhausted]),
            Err(ModelError::ResourcePolicy)
        ));

        let duplicate_item_configuration = ResourcePolicyConfiguration::FiniteInventory {
            ordered_allocation_values: vec![10_009, 10_009],
        };
        let duplicate_item = resource_record(
            None,
            &first_effect,
            duplicate_item_configuration,
            10_009,
            "resource-append:duplicate-item",
            "effect-append:duplicate-item",
        );
        assert!(matches!(
            fold_resource(&[duplicate_item]),
            Err(ModelError::ResourcePolicy)
        ));

        let account_configuration =
            ResourcePolicyConfiguration::AccountSequence { initial_value: 41 };
        let first_account = resource_record(
            None,
            &first_effect,
            account_configuration.clone(),
            41,
            "resource-append:account:0",
            "effect-append:account:0",
        );
        let second_account = resource_record(
            Some(&first_account),
            &second_effect,
            account_configuration,
            42,
            "resource-append:account:1",
            "effect-append:account:1",
        );
        fold_resource(&[first_account.clone(), second_account.clone()])
            .expect("configured account sequence");

        let changed_initial = resource_record(
            Some(&first_account),
            &second_effect,
            ResourcePolicyConfiguration::AccountSequence { initial_value: 42 },
            43,
            "resource-append:changed-initial",
            "effect-append:changed-initial",
        );
        assert!(matches!(
            fold_resource(&[first_account.clone(), changed_initial]),
            Err(ModelError::ResourceChain)
        ));

        let first_exclusive = resource_record(
            None,
            &first_effect,
            ResourcePolicyConfiguration::Exclusive,
            1,
            "resource-append:exclusive:0",
            "effect-append:exclusive:0",
        );
        let second_exclusive = resource_record(
            Some(&first_exclusive),
            &second_effect,
            ResourcePolicyConfiguration::Exclusive,
            1,
            "resource-append:exclusive:1",
            "effect-append:exclusive:1",
        );
        assert!(matches!(
            fold_resource(&[first_exclusive, second_exclusive]),
            Err(ModelError::ResourcePolicy)
        ));

        let mut duplicate_append = second_account.clone();
        duplicate_append.append.append_request_id = first_account.append.append_request_id.clone();
        let duplicate_append = seal_resource(duplicate_append);
        assert!(matches!(
            fold_resource(&[first_account, duplicate_append]),
            Err(ModelError::ResourceChain)
        ));

        assert_ne!(
            resource_policy_configuration_ref(&ResourcePolicyConfiguration::AccountSequence {
                initial_value: 41
            }),
            resource_policy_configuration_ref(&ResourcePolicyConfiguration::AccountSequence {
                initial_value: 42
            })
        );

        let atomic_bound = bound("atomic-link");
        let wrong_link = resource_record(
            None,
            &atomic_bound,
            ResourcePolicyConfiguration::Exclusive,
            1,
            "resource-append:atomic",
            "effect-append:wrong",
        );
        let allocation = append_effect(
            &atomic_bound,
            "effect-append:allocation",
            EffectRecordBody::ResourceAllocated(resource_link(&wrong_link)),
        );
        let folded_effect = fold_effect(&[atomic_bound, allocation], &generous_contract())
            .expect("fold allocation");
        let folded_wrong_link =
            fold_resource(&[wrong_link]).expect("resource record remains internally authentic");
        assert!(matches!(
            validate_resource_links(&[folded_effect], &[folded_wrong_link]),
            Err(ModelError::ResourceLink)
        ));
    }

    #[test]
    fn append_frontier_and_dense_attempt_hostility_is_rejected() {
        let bound_record = bound("hostile");
        let (_, allocation) =
            allocated_effect(&bound_record, ResourcePolicyConfiguration::Exclusive, 1);

        let mut missing_append = allocation.clone();
        missing_append.append = None;
        let missing_append = seal_effect(missing_append);
        assert!(matches!(
            fold_effect(
                &[bound_record.clone(), missing_append],
                &generous_contract()
            ),
            Err(ModelError::AppendIdentity)
        ));

        let mut wrong_predecessor_identity = allocation.clone();
        {
            let append = wrong_predecessor_identity
                .append
                .as_mut()
                .expect("allocation append");
            append.expected_predecessor_sequence = 0;
            append.expected_predecessor_digest.clear();
        }
        refresh_effect_candidate(&mut wrong_predecessor_identity);
        let wrong_predecessor_identity = seal_effect(wrong_predecessor_identity);
        assert!(matches!(
            fold_effect(
                &[bound_record.clone(), wrong_predecessor_identity],
                &generous_contract()
            ),
            Err(ModelError::AppendIdentity)
        ));

        let mut wrong_purpose = allocation.clone();
        wrong_purpose
            .append
            .as_mut()
            .expect("allocation append")
            .purpose = "delivery_attempt_authorized".to_owned();
        refresh_effect_candidate(&mut wrong_purpose);
        let wrong_purpose = seal_effect(wrong_purpose);
        assert!(matches!(
            fold_effect(&[bound_record.clone(), wrong_purpose], &generous_contract()),
            Err(ModelError::AppendIdentity)
        ));

        let mut wrong_candidate = allocation.clone();
        wrong_candidate
            .append
            .as_mut()
            .expect("allocation append")
            .candidate_digest
            .push(0);
        let wrong_candidate = seal_effect(wrong_candidate);
        assert!(matches!(
            fold_effect(
                &[bound_record.clone(), wrong_candidate],
                &generous_contract()
            ),
            Err(ModelError::AppendIdentity)
        ));

        let duplicate_append_id =
            authorization(&allocation, "effect-append:allocation", 0, "operation:0");
        assert!(matches!(
            fold_effect(
                &[
                    bound_record.clone(),
                    allocation.clone(),
                    duplicate_append_id
                ],
                &generous_contract()
            ),
            Err(ModelError::AppendIdentity)
        ));

        let mut wrong_chain_predecessor = allocation.clone();
        wrong_chain_predecessor.predecessor_digest = Some(b"wrong-head".to_vec());
        let wrong_chain_predecessor = seal_effect(wrong_chain_predecessor);
        assert!(matches!(
            fold_effect(
                &[bound_record.clone(), wrong_chain_predecessor],
                &generous_contract()
            ),
            Err(ModelError::EffectChain)
        ));

        let mut wrong_proof = allocation.clone();
        wrong_proof.proof_digest.push(0);
        assert!(matches!(
            fold_effect(&[bound_record.clone(), wrong_proof], &generous_contract()),
            Err(ModelError::EffectChain)
        ));

        let mut wrong_retained_bytes = allocation.clone();
        wrong_retained_bytes.retained_bytes += 1;
        wrong_retained_bytes.commit_digest = effect_record_digest(&wrong_retained_bytes);
        wrong_retained_bytes.proof_digest = effect_proof_digest(&wrong_retained_bytes);
        assert!(matches!(
            fold_effect(
                &[bound_record.clone(), wrong_retained_bytes],
                &generous_contract()
            ),
            Err(ModelError::EffectChain)
        ));

        let mut historical_authorization =
            authorization(&allocation, "effect-append:historical", 0, "operation:0");
        {
            let append = historical_authorization
                .append
                .as_mut()
                .expect("authorization append");
            append.expected_predecessor_sequence = bound_record.sequence;
            append.expected_predecessor_digest = bound_record.commit_digest.clone();
        }
        refresh_effect_candidate(&mut historical_authorization);
        let historical_authorization = seal_effect(historical_authorization);
        assert!(matches!(
            fold_effect(
                &[
                    bound_record.clone(),
                    allocation.clone(),
                    historical_authorization,
                ],
                &generous_contract(),
            ),
            Err(ModelError::AppendIdentity)
        ));

        let bundled_authorization_id = "effect-append:bundled-authorization";
        let mut bundled_allocation = allocation.clone();
        bundled_allocation
            .append
            .as_mut()
            .expect("allocation append")
            .append_request_id =
            bundled_resource_allocation_append_request_id("hostile", bundled_authorization_id);
        let bundled_allocation = seal_effect(bundled_allocation);
        let mut bundled_authorization = authorization(
            &bundled_allocation,
            bundled_authorization_id,
            0,
            "operation:0",
        );
        {
            let append = bundled_authorization
                .append
                .as_mut()
                .expect("authorization append");
            append.expected_predecessor_sequence = bound_record.sequence;
            append.expected_predecessor_digest = bound_record.commit_digest.clone();
        }
        refresh_effect_candidate(&mut bundled_authorization);
        let bundled_authorization = seal_effect(bundled_authorization);
        fold_effect(
            &[
                bound_record.clone(),
                bundled_allocation,
                bundled_authorization,
            ],
            &generous_contract(),
        )
        .expect("closed allocation-plus-authorization bundle");

        let gap = authorization(&allocation, "effect-append:gap", 1, "operation:1");
        assert!(matches!(
            fold_effect(
                &[bound_record.clone(), allocation.clone(), gap],
                &generous_contract()
            ),
            Err(ModelError::EffectAlgebra)
        ));

        let mut wrong_attempt_id =
            authorization(&allocation, "effect-append:wrong-attempt", 0, "operation:0");
        match &mut wrong_attempt_id.body {
            EffectRecordBody::DeliveryAttemptAuthorized(authorization) => {
                authorization.attempt_id.push(0);
            }
            _ => unreachable!("authorization helper returned another variant"),
        }
        refresh_effect_candidate(&mut wrong_attempt_id);
        let wrong_attempt_id = seal_effect(wrong_attempt_id);
        assert!(matches!(
            fold_effect(
                &[bound_record.clone(), allocation.clone(), wrong_attempt_id],
                &generous_contract()
            ),
            Err(ModelError::EffectAlgebra)
        ));
    }

    #[test]
    fn no_resource_effect_is_authorizable_and_carries_completion_debt() {
        let bound_record = bound("no-resource");
        let authorized =
            authorization(&bound_record, "effect-append:no-resource", 0, "operation:0");
        let records = vec![bound_record, authorized];
        let folded =
            fold_effect(&records, &generous_contract()).expect("resource allocation is optional");
        assert_eq!(folded.projection.state, "authorized");
        assert_eq!(folded.unmatched_attempts, 1);
        assert_eq!(folded.projection.policy_ref, None);
        assert_eq!(folded.projection.resource_record_ref, None);

        let exact_contract = contract(
            folded.record_count + 2,
            folded.retained_bytes + 2 * COMPLETION_RECORD_BYTES,
        );
        fold_effect(&records, &exact_contract).expect("exact loaded completion debt");
        assert!(matches!(
            fold_effect(
                &records,
                &contract(
                    exact_contract.max_records - 1,
                    exact_contract.max_retained_bytes
                )
            ),
            Err(ModelError::EvidenceBounds)
        ));
    }

    #[test]
    fn completion_reserve_debt_is_cumulative_beyond_the_two_record_minimum() {
        let contract = generous_contract();
        assert_eq!(
            completion_reserve_debt(1, true, &contract).expect("one attempt plus tombstone"),
            (2, 2 * COMPLETION_RECORD_BYTES)
        );
        assert_eq!(
            completion_reserve_debt(2, true, &contract).expect("two attempts plus tombstone"),
            (3, 3 * COMPLETION_RECORD_BYTES)
        );
        assert_eq!(
            completion_reserve_debt(2, false, &contract).expect("two late observations"),
            (2, 2 * COMPLETION_RECORD_BYTES)
        );
    }

    #[test]
    fn effect_id_utf8_byte_bound_closes_completion_record_capacity() {
        let effect_id = "é".repeat(MAX_EFFECT_ID_UTF8_BYTES / "é".len());
        assert_eq!(effect_id.len(), MAX_EFFECT_ID_UTF8_BYTES);
        validate_effect_id(&effect_id).expect("exact UTF-8 byte bound");

        let bound_record = bound(&effect_id);
        let target_operation_ref = format!("destination-operation:{effect_id}");
        let authorized = authorization(
            &bound_record,
            &format!("authorization:{effect_id}:1"),
            0,
            &target_operation_ref,
        );
        let attempt_id = attempt_id(&authorized);
        let returned_ref = vec![0x5a; 32];
        let mut observed = observation(
            &authorized,
            &format!("observation:{effect_id}:0"),
            attempt_id.clone(),
            AttemptOutcome::Returned {
                safe_outcome_ref: returned_ref.clone(),
                destination_account: 44,
            },
        );
        observed.opaque_payload = 44_i64.to_be_bytes().to_vec();
        refresh_effect_candidate(&mut observed);
        let observed = seal_effect(observed);
        let mut terminal = tombstone(
            &observed,
            &format!("tombstone:{effect_id}"),
            attempt_id,
            &target_operation_ref,
            returned_ref,
        );
        terminal.opaque_payload.clear();
        refresh_effect_candidate(&mut terminal);
        let terminal = seal_effect(terminal);

        assert!(
            observed.retained_bytes <= PROTOTYPE_COMPLETION_RECORD_BYTES,
            "bounded observation is {} bytes",
            observed.retained_bytes
        );
        assert!(
            terminal.retained_bytes <= PROTOTYPE_COMPLETION_RECORD_BYTES,
            "bounded tombstone is {} bytes",
            terminal.retained_bytes
        );
        let mut prototype_contract = generous_contract();
        prototype_contract.completion_reserve_bytes = PROTOTYPE_COMPLETION_RECORD_BYTES;
        let pending = fold_effect(std::slice::from_ref(&bound_record), &prototype_contract)
            .expect("fold maximum-length identifier");
        ensure_authorization_capacity(&pending, &prototype_contract, authorized.retained_bytes)
            .expect("authorization preserves bounded completion capacity");
        fold_effect(
            &[bound_record, authorized, observed, terminal],
            &prototype_contract,
        )
        .expect("maximum-length identifier completes within reserve");

        let oversized = format!("{effect_id}x");
        assert_eq!(oversized.len(), MAX_EFFECT_ID_UTF8_BYTES + 1);
        assert!(matches!(
            validate_effect_id(&oversized),
            Err(ModelError::EffectIdentity)
        ));
        assert!(matches!(
            fold_effect(&[bound(&oversized)], &prototype_contract),
            Err(ModelError::EffectIdentity)
        ));
    }

    #[test]
    fn multi_unmatched_attempt_reserves_are_exact_at_record_and_byte_boundaries() {
        let bound_record = bound("bounds");
        let (_, allocation) =
            allocated_effect(&bound_record, ResourcePolicyConfiguration::Exclusive, 1);
        let first_authorized =
            authorization(&allocation, "effect-append:authorize:0", 0, "operation:0");
        let first_records = vec![
            bound_record.clone(),
            allocation.clone(),
            first_authorized.clone(),
        ];
        let folded_once =
            fold_effect(&first_records, &generous_contract()).expect("first unmatched attempt");
        assert_eq!(folded_once.unmatched_attempts, 1);

        let second_authorized = authorization(
            &first_authorized,
            "effect-append:authorize:1",
            1,
            "operation:1",
        );
        let reserve_record_count = 3;
        let reserve_bytes = reserve_record_count * COMPLETION_RECORD_BYTES;
        let exact_max_records = folded_once.record_count + 1 + reserve_record_count;
        let exact_max_bytes =
            folded_once.retained_bytes + second_authorized.retained_bytes + reserve_bytes;
        let exact_contract = contract(exact_max_records, exact_max_bytes);
        ensure_authorization_capacity(
            &folded_once,
            &exact_contract,
            second_authorized.retained_bytes,
        )
        .expect("exact multi-unmatched reserve");

        let one_record_short = contract(exact_max_records - 1, exact_max_bytes);
        assert!(matches!(
            ensure_authorization_capacity(
                &folded_once,
                &one_record_short,
                second_authorized.retained_bytes
            ),
            Err(ModelError::EvidenceBounds)
        ));

        let one_byte_short = contract(exact_max_records, exact_max_bytes - 1);
        assert!(matches!(
            ensure_authorization_capacity(
                &folded_once,
                &one_byte_short,
                second_authorized.retained_bytes
            ),
            Err(ModelError::EvidenceBounds)
        ));

        let mut two_attempt_records = first_records;
        two_attempt_records.push(second_authorized);
        let folded_twice =
            fold_effect(&two_attempt_records, &exact_contract).expect("two unmatched attempts");
        assert_eq!(folded_twice.unmatched_attempts, 2);
        assert!(matches!(
            fold_effect(
                &two_attempt_records,
                &contract(exact_max_records - 1, exact_max_bytes)
            ),
            Err(ModelError::EvidenceBounds)
        ));
        assert!(matches!(
            fold_effect(
                &two_attempt_records,
                &contract(exact_max_records, exact_max_bytes - 1)
            ),
            Err(ModelError::EvidenceBounds)
        ));

        let retained_total = folded_twice.retained_bytes;
        let record_total = folded_twice.record_count;
        assert!(matches!(
            fold_effect(
                &two_attempt_records,
                &contract(record_total - 1, retained_total)
            ),
            Err(ModelError::EvidenceBounds)
        ));
        assert!(matches!(
            fold_effect(
                &two_attempt_records,
                &contract(record_total, retained_total - 1)
            ),
            Err(ModelError::EvidenceBounds)
        ));

        let mut attempts_exhausted = exact_contract;
        attempts_exhausted.max_attempts = 1;
        assert!(matches!(
            ensure_authorization_capacity(
                &folded_once,
                &attempts_exhausted,
                two_attempt_records
                    .last()
                    .expect("second authorization")
                    .retained_bytes
            ),
            Err(ModelError::EvidenceBounds)
        ));

        let mut invalid_reserve = generous_contract();
        invalid_reserve.completion_reserve_records = 1;
        assert!(matches!(
            fold_effect(&two_attempt_records, &invalid_reserve),
            Err(ModelError::EvidenceBounds)
        ));
        invalid_reserve.completion_reserve_records = 3;
        assert!(matches!(
            fold_effect(&two_attempt_records, &invalid_reserve),
            Err(ModelError::EvidenceBounds)
        ));
    }
}
