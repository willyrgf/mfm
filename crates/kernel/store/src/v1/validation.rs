use super::*;

#[path = "validation_artifacts.rs"]
mod artifact_validation;
pub use self::artifact_validation::validate_artifact_requirement_against_evidence;
use self::artifact_validation::validate_required_artifact_requirement;
pub(super) use self::artifact_validation::verify_retained_artifact_bytes;

#[path = "event_keys.rs"]
mod event_keys;
pub(super) use self::event_keys::{
    derive_event_id, derive_logical_key, is_unique_logical_key, payload_spec_hash,
    unique_logical_key_rewrite_allowed,
};

#[path = "terminal_payloads.rs"]
mod terminal_payloads;
use self::terminal_payloads::{
    is_attempt_terminal_commit_payload, is_attempt_terminal_payload, is_retention_commit_payload,
    is_retention_payload, is_run_completed_payload, is_saga_terminal_commit_payload,
    is_side_effect_payload, is_side_effect_progress_commit_payload,
    is_side_effect_terminal_commit_payload, is_side_effect_terminal_disposition_payload,
    validate_attempt_terminal_resource_lane_release_batch,
    validate_side_effect_terminal_resource_lane_release_batch,
};
pub(super) use self::terminal_payloads::{
    request_contains_manual_resolution, request_contains_saga_terminal_outcome,
};

pub(super) fn validate_unique_artifact_evidence(
    field: &'static str,
    artifacts: &[ArtifactEvidenceRef],
) -> Result<()> {
    let mut by_artifact = BTreeMap::<ArtifactAuthorityKey, &ArtifactEvidenceRef>::new();
    for artifact in artifacts {
        let key = artifact_authority_key(artifact)?;
        if let Some(existing) = by_artifact.insert(key, artifact) {
            if existing != artifact {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: artifact.artifact_id.clone(),
                    field,
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_required_artifacts_cover_payload_references(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    for payload in &request.payloads {
        for requirement in event_artifact_requirements(payload) {
            let Some(evidence) = request.required_artifacts.iter().find(|evidence| {
                evidence.artifact_id == requirement.artifact_id
                    && evidence.evidence_hash().ok().as_ref() == Some(&requirement.evidence_hash)
            }) else {
                return Err(invalid_prepared_commit_purpose(
                    purpose,
                    format!(
                        "missing required artifact evidence for {}",
                        requirement.artifact_id
                    ),
                ));
            };
            validate_required_artifact_requirement(purpose, &requirement, evidence)?;
        }
    }
    Ok(())
}

pub(super) fn reject_store_materialized_resource_lane_payloads(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    if request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::ResourceLaneClaimed(_)
                | KernelEventPayload::ResourceLaneReleased(_)
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            purpose,
            "prepared commits must use resource-lane intents, not store-filled lane events",
        ));
    }
    Ok(())
}

pub(super) fn validate_payload_public_diagnostics(payloads: &[KernelEventPayload]) -> Result<()> {
    for payload in payloads {
        match payload {
            KernelEventPayload::PublicOutputRenderFailed(payload) => payload.error.validate()?,
            KernelEventPayload::StateAttemptFailed(payload) => payload.error.validate()?,
            KernelEventPayload::SideEffectFailed(payload) => payload.error.validate()?,
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn validate_run_start_commit(request: &CommitRequest) -> Result<()> {
    if request.payloads.len() != 1
        || !matches!(
            request.payloads.first(),
            Some(KernelEventPayload::RunAdmitted(_))
        )
    {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "run-admission commits must contain exactly one RunAdmitted payload",
        ));
    }
    let Some(KernelEventPayload::RunAdmitted(payload)) = request.payloads.first() else {
        unreachable!("run-admission payload shape was checked above");
    };
    let authority = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                RunAdmission::NAME,
                "run admission requires certified run store authority",
            )
        })?;
    if authority.run_id() != request.run_id() || authority.spec_hash() != &payload.spec_hash {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "certified run store authority does not match RunAdmitted",
        ));
    }
    validate_run_admitted_identity_for_request(request.run_id(), payload)?;
    if request.preconditions.required_run_state != RequiredRunState::Absent {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "run admission requires absent-run precondition",
        ));
    }
    Ok(())
}

fn validate_run_admitted_identity_for_request(
    run_id: &RunId,
    payload: &events::RunAdmitted,
) -> Result<()> {
    if payload.run_id != *run_id {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted run id does not match commit run id",
        ));
    }
    if payload.identity_material.certified_spec_hash != payload.spec_hash {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted identity material spec hash does not match event spec hash",
        ));
    }
    let derived = payload.identity_material.derive_run_id().map_err(|_| {
        invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted identity material is invalid",
        )
    })?;
    if derived != payload.run_id {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted run id does not match identity material",
        ));
    }
    Ok(())
}

pub(super) fn validate_state_attempt_started_commit(request: &CommitRequest) -> Result<()> {
    if request.payloads.len() != 1
        || !matches!(
            request.payloads.first(),
            Some(KernelEventPayload::StateAttemptStarted(_))
        )
    {
        return Err(invalid_prepared_commit_purpose(
            StateAttemptStarted::NAME,
            "state-attempt-start commits must contain exactly one StateAttemptStarted payload",
        ));
    }
    if request.preconditions.required_run_state != RequiredRunState::NotCompleted {
        return Err(invalid_prepared_commit_purpose(
            StateAttemptStarted::NAME,
            "state-attempt-start requires not-completed run precondition",
        ));
    }
    Ok(())
}

pub(super) fn validate_attempt_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        AttemptTerminal::NAME,
        request,
        is_attempt_terminal_commit_payload,
        "attempt-terminal commits cannot contain non-attempt-terminal payloads",
    )?;
    require_purpose_payload(
        AttemptTerminal::NAME,
        request,
        is_attempt_terminal_payload,
        "missing attempt-terminal payload",
    )?;
    validate_attempt_terminal_resource_lane_release_batch(request)?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

pub(super) fn validate_side_effect_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SideEffectTerminal::NAME,
        request,
        is_side_effect_terminal_commit_payload,
        "side-effect terminal commits cannot contain non-side-effect-terminal payloads",
    )?;
    require_purpose_payload(
        SideEffectTerminal::NAME,
        request,
        is_side_effect_terminal_disposition_payload,
        "missing side-effect terminal payload",
    )?;
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SideEffectTerminal::NAME,
            "side-effect terminal commits require certified run authority",
        ));
    }
    validate_side_effect_terminal_resource_lane_release_batch(request)?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

pub(super) fn validate_side_effect_progress_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SideEffectProgress::NAME,
        request,
        is_side_effect_progress_commit_payload,
        "side-effect progress commits cannot contain non-side-effect-progress payloads",
    )?;
    require_purpose_payload(
        SideEffectProgress::NAME,
        request,
        is_side_effect_payload,
        "missing side-effect payload",
    )?;
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SideEffectProgress::NAME,
            "side-effect progress commits require certified run authority",
        ));
    }
    Ok(())
}

pub(super) fn validate_retention_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        Retention::NAME,
        request,
        is_retention_commit_payload,
        "retention commits cannot contain non-retention payloads",
    )?;
    require_purpose_payload(
        Retention::NAME,
        request,
        is_retention_payload,
        "missing retention payload",
    )?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

fn validate_manual_resolution_commit(request: &CommitRequest) -> Result<()> {
    let manual_resolution_count = request
        .payloads
        .iter()
        .filter(|payload| matches!(payload, KernelEventPayload::ManualResolutionRecorded(_)))
        .count();
    if manual_resolution_count != 1 {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution commits must contain exactly one ManualResolutionRecorded payload",
        ));
    }
    if !matches!(
        request.payloads.last(),
        Some(KernelEventPayload::ManualResolutionRecorded(_))
    ) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution release intents must precede ManualResolutionRecorded",
        ));
    }
    if request.payloads.iter().any(|payload| {
        !matches!(
            payload,
            KernelEventPayload::ManualResolutionRecorded(_)
                | KernelEventPayload::ResourceLaneReleaseIntent(_)
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution commits may only contain resource lane release intents and ManualResolutionRecorded",
        ));
    }
    if request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
                release_authority,
                ..
            }) if *release_authority != events::ResourceLaneReleaseAuthority::ManualResolution
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution release intents require manual resolution release authority",
        ));
    }
    if request.preconditions.required_run_state != RequiredRunState::NotCompleted {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires not-completed run precondition",
        ));
    }
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires certified run authority",
        ));
    }
    Ok(())
}

pub(super) fn validate_manual_resolution_commit_with_proof(
    request: &CommitRequest,
    proof: &VerifiedManualResolutionForPrefix,
) -> Result<()> {
    validate_manual_resolution_commit(request)?;
    let manual_resolutions = request
        .payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::ManualResolutionRecorded(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if manual_resolutions.len() != 1 {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires exactly one ManualResolutionRecorded payload",
        ));
    }
    let payload = manual_resolutions[0];
    let token = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                ManualResolution::NAME,
                "manual resolution requires certified run authority",
            )
        })?;
    let prefix = proof.prefix();
    if prefix.run_id() != request.run_id()
        || prefix.run_id() != &payload.run_id
        || prefix.run_id() != token.run_id()
    {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof run id does not match manual resolution request",
        ));
    }
    if prefix.spec_hash() != &payload.spec_hash || prefix.spec_hash() != token.spec_hash() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof spec hash does not match manual resolution request",
        ));
    }
    if prefix.expected_next_seq() != request.expected_next_seq().as_u64() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof prefix expected_next_seq does not match manual resolution request",
        ));
    }
    if proof.outcome() != payload.outcome {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "ManualResolutionRecorded outcome does not match manual proof",
        ));
    }
    if proof.evidence().schema_id != payload.evidence_schema_id
        || proof.evidence().content_hash != payload.evidence_hash
        || proof.evidence().artifact_id != payload.evidence_artifact_id
        || proof.authorization().schema_id != payload.authorization_schema_id
        || proof.authorization().content_hash != payload.authorization_hash
        || proof.authorization().artifact_id != payload.authorization_artifact_id
    {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "ManualResolutionRecorded artifact refs do not match manual proof",
        ));
    }
    let block_reason = manual_block_reason_from_auth(prefix.manual_block_reason());
    let certified_manual_policy = manual_policy_for_block_reason(token.saga_policy(), block_reason)
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                ManualResolution::NAME,
                "certified run authority policy does not permit manual proof block reason",
            )
        })?;
    if certified_manual_policy != prefix.manual_policy() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof policy does not match certified run authority",
        ));
    }
    Ok(())
}

fn validate_saga_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SagaTerminal::NAME,
        request,
        is_saga_terminal_commit_payload,
        "saga terminal commits cannot contain non-terminal payloads",
    )?;
    if request
        .payloads
        .iter()
        .filter(|payload| is_run_completed_payload(payload))
        .count()
        != 1
    {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution commits must contain exactly one RunCompleted payload",
        ));
    }
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution requires certified run authority",
        ));
    }
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

pub(super) fn validate_saga_terminal_commit_with_proof(
    request: &CommitRequest,
    proof: &SagaTerminalProof,
) -> Result<()> {
    validate_saga_terminal_commit(request)?;
    let completed = request
        .payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if completed.len() != 1 {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution requires exactly one RunCompleted payload",
        ));
    }
    let payload = completed[0];
    let token = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                SagaTerminal::NAME,
                "saga terminal resolution requires certified run authority",
            )
        })?;
    if proof.run_id() != request.run_id()
        || proof.run_id() != &payload.run_id
        || proof.run_id() != token.run_id()
    {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof run id does not match terminal request",
        ));
    }
    if proof.prefix_next_seq() != request.expected_next_seq() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof prefix does not match terminal request expected next sequence",
        ));
    }
    if proof.saga_policy_digest() != token.saga_policy_digest() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof saga policy digest does not match certified run authority",
        ));
    }
    let proof_outcome = proof.outcome();
    if payload.outcome != proof_outcome {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "RunCompleted outcome does not match SagaTerminalProof",
        ));
    }
    if matches!(payload.outcome, events::RunCompletionOutcome::Completed(_)) {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "completed public-output terminal must use CompleteRun authority",
        ));
    }
    if let Some(spec_hash) = proof.manual_spec_hash() {
        if spec_hash != &payload.spec_hash {
            return Err(invalid_prepared_commit_purpose(
                SagaTerminal::NAME,
                "manual proof spec hash does not match RunCompleted payload",
            ));
        }
    }
    Ok(())
}

fn require_purpose_payload(
    purpose: &'static str,
    request: &CommitRequest,
    predicate: impl Fn(&KernelEventPayload) -> bool,
    message: &'static str,
) -> Result<()> {
    if request.payloads.iter().any(predicate) {
        Ok(())
    } else {
        Err(invalid_prepared_commit_purpose(purpose, message))
    }
}

fn reject_wrong_purpose_payloads(
    purpose: &'static str,
    request: &CommitRequest,
    predicate: impl Fn(&KernelEventPayload) -> bool,
    message: &'static str,
) -> Result<()> {
    if let Some(payload) = request.payloads.iter().find(|payload| !predicate(payload)) {
        Err(invalid_prepared_commit_purpose(
            purpose,
            format!("{message}: {:?}", payload.event_schema_id()),
        ))
    } else {
        Ok(())
    }
}

pub(super) fn invalid_prepared_commit_purpose(
    purpose: &'static str,
    message: impl Into<String>,
) -> StoreError {
    StoreError::InvalidPreparedCommitPurpose {
        purpose,
        message: message.into(),
    }
}

pub(super) fn validate_payload_run_and_spec(
    run_id: &RunId,
    payloads: &[KernelEventPayload],
) -> Result<()> {
    let Some(first) = payloads.first() else {
        return Err(StoreError::EmptyCommit);
    };
    let expected_spec_hash = payload_spec_hash(first);
    for payload in payloads {
        if let Some(payload_run_id) = payload_run_id(payload) {
            if payload_run_id != *run_id {
                return Err(StoreError::PayloadRunMismatch {
                    expected: Box::new(run_id.clone()),
                    actual: Box::new(payload_run_id),
                });
            }
        }
        let actual_spec_hash = payload_spec_hash(payload);
        if actual_spec_hash != expected_spec_hash {
            return Err(StoreError::PayloadSpecHashMismatch {
                expected: Box::new(expected_spec_hash),
                actual: Box::new(actual_spec_hash),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_terminal_attempt_cell_pairs(payloads: &[KernelEventPayload]) -> Result<()> {
    let mut completions = BTreeSet::new();
    let mut terminal_cells = BTreeSet::new();
    let mut public_outputs = BTreeSet::new();

    for payload in payloads {
        match payload {
            KernelEventPayload::StateAttemptCompleted(payload) => {
                completions.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.output_cell_id.clone(),
                ));
            }
            KernelEventPayload::CellProduced(payload) => {
                terminal_cells.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            KernelEventPayload::CellSkipped(payload) => {
                terminal_cells.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            KernelEventPayload::PublicOutputProduced(payload) => {
                public_outputs.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.receipt_cell_id.clone(),
                ));
            }
            _ => {}
        }
    }

    for (node_id, attempt_id, output_cell_id) in &completions {
        if !terminal_cells.contains(&(node_id.clone(), attempt_id.clone(), output_cell_id.clone()))
        {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{node_id}:{attempt_id}"),
                message: "attempt completion requires matching terminal cell in same commit"
                    .to_owned(),
            });
        }
    }
    for (node_id, attempt_id, cell_id) in &terminal_cells {
        if !completions.contains(&(node_id.clone(), attempt_id.clone(), cell_id.clone())) {
            return Err(StoreError::ProjectionConflict {
                key: format!("cell:{cell_id}:terminal"),
                message: "terminal cell requires matching attempt completion in same commit"
                    .to_owned(),
            });
        }
    }
    for (node_id, attempt_id, receipt_cell_id) in &public_outputs {
        let terminal = (node_id.clone(), attempt_id.clone(), receipt_cell_id.clone());
        if !terminal_cells.contains(&terminal) || !completions.contains(&terminal) {
            return Err(StoreError::ProjectionConflict {
                key: format!("public_output:{node_id}:{attempt_id}"),
                message: "public output requires matching render receipt terminal in same commit"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_supported_stream_model(events: &[KernelEventEnvelope]) -> Result<()> {
    let mut started_attempts = BTreeMap::<(NodeId, AttemptId), StreamSeq>::new();
    for event in events {
        if let KernelEventPayload::StateAttemptStarted(payload) = event.payload() {
            started_attempts.insert(
                (payload.node_id.clone(), payload.attempt_id.clone()),
                event.seq(),
            );
        }
    }
    for event in events {
        for (node_id, attempt_id) in payload_required_started_attempts(event.payload()) {
            let key = (node_id.clone(), attempt_id.clone());
            let Some(start_seq) = started_attempts.get(&key) else {
                return Err(StoreError::ProjectionConflict {
                    key: format!("stream_model:{node_id}:{attempt_id}"),
                    message: "invalid run stream model: attempt-bound payload is not preceded by a StateAttemptStarted commit".to_owned(),
                });
            };
            if *start_seq >= event.seq() {
                return Err(StoreError::ProjectionConflict {
                    key: format!("stream_model:{node_id}:{attempt_id}"),
                    message: "invalid run stream model: StateAttemptStarted must be committed before attempt-bound terminal payloads".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn payload_required_started_attempts(payload: &KernelEventPayload) -> Vec<(NodeId, AttemptId)> {
    match payload {
        KernelEventPayload::StateAttemptCompleted(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::CellProduced(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::CellSkipped(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::FactRecorded(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::PublicOutputProduced(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        payload => payload
            .side_effect_ref()
            .map(|side_effect| vec![(side_effect.node_id.clone(), side_effect.attempt_id.clone())])
            .unwrap_or_default(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalSideEffectEvidencePair {
    node_id: NodeId,
    attempt_id: AttemptId,
    retryable: bool,
    kind: TerminalSideEffectEvidenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalSideEffectEvidenceKind {
    Ambiguous,
    Failed,
}

impl TerminalSideEffectEvidencePair {
    fn ambiguous(payload: &side_effect::Ambiguous) -> Self {
        Self {
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            retryable: false,
            kind: TerminalSideEffectEvidenceKind::Ambiguous,
        }
    }

    fn failed(payload: &side_effect::Failed) -> Self {
        Self {
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            retryable: payload.retryable,
            kind: TerminalSideEffectEvidenceKind::Failed,
        }
    }

    fn node_attempt_key(&self) -> (NodeId, AttemptId) {
        (self.node_id.clone(), self.attempt_id.clone())
    }

    fn label(&self) -> &'static str {
        match self.kind {
            TerminalSideEffectEvidenceKind::Ambiguous => "ambiguity",
            TerminalSideEffectEvidenceKind::Failed => "failure",
        }
    }
}

pub(super) fn validate_terminal_side_effect_evidence_pairs(
    payloads: &[KernelEventPayload],
) -> Result<()> {
    let mut side_effect_terminals = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for payload in payloads {
        match payload {
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                insert_terminal_side_effect_pair(
                    &mut side_effect_terminals,
                    TerminalSideEffectEvidencePair::ambiguous(payload),
                )?;
            }
            KernelEventPayload::SideEffectFailed(payload) => {
                insert_terminal_side_effect_pair(
                    &mut side_effect_terminals,
                    TerminalSideEffectEvidencePair::failed(payload),
                )?;
            }
            KernelEventPayload::StateAttemptFailed(payload) => {
                attempt_failures.insert(
                    (payload.node_id.clone(), payload.attempt_id.clone()),
                    payload.retryable,
                );
            }
            _ => {}
        }
    }
    for pair in side_effect_terminals.values() {
        let key = pair.node_attempt_key();
        match attempt_failures.get(&key) {
            Some(attempt_retryable) if *attempt_retryable == pair.retryable => {}
            Some(_) => {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "sidefx:{}:{}:{}",
                        pair.node_id,
                        pair.attempt_id,
                        pair.label()
                    ),
                    message: format!(
                        "side-effect {} retryability must match attempt failure",
                        pair.label()
                    ),
                });
            }
            None => {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "sidefx:{}:{}:{}",
                        pair.node_id,
                        pair.attempt_id,
                        pair.label()
                    ),
                    message: format!(
                        "side-effect {} requires matching StateAttemptFailed in same commit",
                        pair.label()
                    ),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_side_effect_attempt_failures_have_terminal_evidence(
    projections: &ProjectionSnapshot,
    payloads: &[KernelEventPayload],
) -> Result<()> {
    let mut current_side_effect_authority = BTreeSet::new();
    let mut terminal_side_effect_evidence = BTreeSet::new();
    let mut attempt_failures = Vec::new();
    for payload in payloads {
        if let Some(side_effect) = payload.side_effect_ref() {
            let key = (side_effect.node_id.clone(), side_effect.attempt_id.clone());
            current_side_effect_authority.insert(key.clone());
            if matches!(
                side_effect.kind,
                events::SideEffectEventKind::Ambiguous | events::SideEffectEventKind::Failed
            ) {
                terminal_side_effect_evidence.insert(key);
            }
        }
        if let KernelEventPayload::StateAttemptFailed(payload) = payload {
            attempt_failures.push((payload.node_id.clone(), payload.attempt_id.clone()));
        }
    }

    for (node_id, attempt_id) in attempt_failures {
        let key = (node_id.clone(), attempt_id.clone());
        if terminal_side_effect_evidence.contains(&key) {
            continue;
        }
        let prior_side_effect = projections.side_effects().find_map(|(_, projection)| {
            if projection.intent.node_id == node_id && projection.intent.attempt_id == attempt_id {
                Some(projection)
            } else {
                None
            }
        });
        if prior_side_effect
            .map(|projection| projection.ledger_state().map(|state| state.is_confirmed()))
            .transpose()?
            .unwrap_or(false)
        {
            continue;
        }
        if prior_side_effect.is_some() || current_side_effect_authority.contains(&key) {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{node_id}:{attempt_id}"),
                message:
                    "side-effect attempt failure requires terminal side-effect evidence in the same commit"
                        .to_owned(),
            });
        }
    }
    Ok(())
}

fn insert_terminal_side_effect_pair(
    pairs: &mut BTreeMap<(NodeId, AttemptId), TerminalSideEffectEvidencePair>,
    pair: TerminalSideEffectEvidencePair,
) -> Result<()> {
    let key = pair.node_attempt_key();
    if pairs.insert(key.clone(), pair).is_some() {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{}:{}:terminal", key.0, key.1),
            message: "terminal side-effect evidence must be unique per node attempt in one commit"
                .to_owned(),
        });
    }
    Ok(())
}

pub(super) fn validate_retention_manifest_pairs(payloads: &[KernelEventPayload]) -> Result<()> {
    let retained_manifest_refs = payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::RetentionRefsAppended(payload) => Some(&payload.refs),
            _ => None,
        })
        .flatten()
        .filter(|retention_ref| retention_ref.role == ArtifactRole::RetentionManifest)
        .map(|retention_ref| {
            (
                retention_ref.artifact_id.clone(),
                retention_ref.content_digest.clone(),
            )
        })
        .collect::<BTreeSet<_>>();

    for payload in payloads {
        let KernelEventPayload::RetentionManifestProjected(payload) = payload else {
            continue;
        };
        if !retained_manifest_refs.contains(&(
            payload.manifest_artifact_id.clone(),
            payload.manifest_digest.clone(),
        )) {
            return Err(StoreError::ProjectionConflict {
                key: format!(
                    "retention:{}:manifest:{}",
                    payload.run_id, payload.manifest_seq
                ),
                message:
                    "retention manifest projection requires matching retention ref in same commit"
                        .to_owned(),
            });
        }
    }
    Ok(())
}

fn payload_run_id(payload: &KernelEventPayload) -> Option<RunId> {
    payload.run_id().cloned()
}
