use super::*;

type ArtifactReferenceKey = (ArtifactId, ContentDigest);

/// Compact explicit-artifact-reference frontier for strict successor validation.
#[derive(Debug)]
pub(super) struct ArtifactHistoryFold {
    referenced: BTreeSet<ArtifactReferenceKey>,
}

impl ArtifactHistoryFold {
    pub(super) fn new() -> Self {
        Self {
            referenced: BTreeSet::new(),
        }
    }

    pub(super) fn apply_suffix(
        &mut self,
        history: &JournalHistory<'_>,
        suffix_start: usize,
    ) -> Result<()> {
        for batch in history.suffix_batches(suffix_start)? {
            self.apply_batch(&batch)?;
        }
        Ok(())
    }

    fn apply_batch(&mut self, batch: &JournalBatch<'_>) -> Result<()> {
        for event in batch.records() {
            let events::KernelEventPayload::ArtifactReferenced(reference) = event.payload() else {
                continue;
            };
            let key = (
                reference.artifact_ref.artifact_id.clone(),
                reference.artifact_ref.evidence_hash.clone(),
            );
            if !self.referenced.insert(key) {
                return Err(invalid_history(format!(
                    "artifact reference {} repeats existing committed authority",
                    reference.artifact_ref.artifact_id
                )));
            }

            if is_attempt_staged_role(reference.artifact_ref.role) {
                if reference.node_id.is_none() || reference.attempt_id.is_none() {
                    return Err(invalid_history(format!(
                        "staged artifact reference {} must be scoped to a producer attempt",
                        reference.artifact_ref.artifact_id
                    )));
                }
                if !artifact_reference_matches_same_batch_payload(batch, reference) {
                    return Err(invalid_history(format!(
                        "staged artifact reference {} is not bound to a same-batch typed payload",
                        reference.artifact_ref.artifact_id
                    )));
                }
            } else if reference.artifact_ref.role != events::ArtifactRole::TypedConfig {
                return Err(invalid_history(format!(
                    "unsupported artifact reference role {} for {}",
                    reference.artifact_ref.role.as_str(),
                    reference.artifact_ref.artifact_id
                )));
            }
        }
        Ok(())
    }
}

fn is_attempt_staged_role(role: events::ArtifactRole) -> bool {
    matches!(
        role.contract().staging,
        events::ArtifactStagingClass::AttemptStateOutput
            | events::ArtifactStagingClass::AttemptFactResponse
            | events::ArtifactStagingClass::AttemptExternalReadEvidence
            | events::ArtifactStagingClass::AttemptFactQueryEvidence
            | events::ArtifactStagingClass::AttemptPublicOutput
            | events::ArtifactStagingClass::AttemptRedactedDiagnostic
    )
}

fn artifact_reference_matches_same_batch_payload(
    batch: &JournalBatch<'_>,
    reference: &events::ArtifactReferenced,
) -> bool {
    batch
        .records()
        .iter()
        .any(|event| payload_matches_reference(event.payload(), reference))
}

fn payload_matches_reference(
    payload: &events::KernelEventPayload,
    reference: &events::ArtifactReferenced,
) -> bool {
    let (Some(reference_node_id), Some(reference_attempt_id)) =
        (&reference.node_id, &reference.attempt_id)
    else {
        return false;
    };
    match payload {
        events::KernelEventPayload::CellProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::StateOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.artifact_id == reference.artifact_ref.artifact_id
                && payload.content_digest == reference.artifact_ref.content_digest
                && payload.evidence_hash == reference.artifact_ref.evidence_hash
                && payload.schema_id == reference.artifact_ref.schema_id
                && reference.artifact_ref.semantic_type_id.as_ref()
                    == Some(&payload.semantic_type_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => {
            let response = payload.claim.response();
            reference.artifact_ref.role == events::ArtifactRole::FactResponse
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && response.artifact_id() == &reference.artifact_ref.artifact_id
                && response.response_hash() == &reference.artifact_ref.content_digest
                && response.artifact_evidence_hash() == &reference.artifact_ref.evidence_hash
                && response.response_schema_id() == &reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::PublicOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.rendered_artifact_id.as_ref()
                    == Some(&reference.artifact_ref.artifact_id)
                && payload.rendered_digest == reference.artifact_ref.content_digest
                && payload.rendered_artifact_evidence_hash.as_ref()
                    == Some(&reference.artifact_ref.evidence_hash)
                && payload.public_schema_id == reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::RedactedDiagnostic
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload
                    .error
                    .diagnostic_ref
                    .as_ref()
                    .is_some_and(|diagnostic| event_artifact_refs_match(diagnostic, reference))
        }
        events::KernelEventPayload::StateAttemptFailed(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::RedactedDiagnostic
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload
                    .error
                    .diagnostic_ref
                    .as_ref()
                    .is_some_and(|diagnostic| event_artifact_refs_match(diagnostic, reference))
        }
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            matches!(
                reference.artifact_ref.role,
                events::ArtifactRole::FactQueryEvidence
                    | events::ArtifactRole::ExternalReadEvidence
            ) && payload.artifact_ref.role == reference.artifact_ref.role
                && payload.node_id.as_ref() == Some(reference_node_id)
                && payload.attempt_id.as_ref() == Some(reference_attempt_id)
                && event_artifact_refs_match(&payload.artifact_ref, reference)
        }
        _ => false,
    }
}

fn event_artifact_refs_match(
    evidence: &events::ArtifactEvidenceRef,
    reference: &events::ArtifactReferenced,
) -> bool {
    evidence == &reference.artifact_ref
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_output_reference() -> events::ArtifactReferenced {
        events::ArtifactReferenced {
            spec_hash: test_support::fixed_spec_hash_for_test(0x41),
            node_id: Some(test_support::fixed_node_id_for_test(0x42)),
            attempt_id: Some(test_support::fixed_attempt_id_for_test(0x43)),
            artifact_ref: events::ArtifactEvidenceRef {
                artifact_id: test_support::fixed_artifact_id_for_test(0x44),
                role: events::ArtifactRole::StateOutput,
                schema_id: test_support::fixed_schema_id_for_test("state-output", 0x45),
                semantic_type_id: Some(test_support::fixed_semantic_type_id_for_test(
                    "state-output",
                    0x46,
                )),
                content_digest: test_support::fixed_content_digest_for_test(0x47),
                evidence_hash: test_support::fixed_content_digest_for_test(0x48),
                byte_len: 32,
                media_type: test_support::media_type_for_test("application/json"),
            },
        }
    }

    fn matching_cell(reference: &events::ArtifactReferenced) -> events::CellProduced {
        events::CellProduced {
            spec_hash: reference.spec_hash.clone(),
            node_id: reference.node_id.clone().expect("test node"),
            cell_id: test_support::fixed_cell_id_for_test(0x49),
            scope_id: test_support::fixed_scope_id_for_test(0x4a),
            attempt_id: reference.attempt_id.clone().expect("test attempt"),
            semantic_type_id: reference
                .artifact_ref
                .semantic_type_id
                .clone()
                .expect("test semantic type"),
            schema_id: reference.artifact_ref.schema_id.clone(),
            value_lineage: spec::ValueLineageRef {
                lineage_digest: test_support::fixed_content_digest_for_test(0x4b),
            },
            context: spec::CellContextSpec::no_context(),
            artifact_id: reference.artifact_ref.artifact_id.clone(),
            content_digest: reference.artifact_ref.content_digest.clone(),
            evidence_hash: reference.artifact_ref.evidence_hash.clone(),
            producer_state_kind: None,
            producer_state_version: None,
        }
    }

    #[test]
    fn state_output_reference_requires_exact_typed_payload() {
        let reference = state_output_reference();
        let matching = events::KernelEventPayload::CellProduced(matching_cell(&reference));
        assert!(payload_matches_reference(&matching, &reference));

        let mut wrong_attempt = matching_cell(&reference);
        wrong_attempt.attempt_id = test_support::fixed_attempt_id_for_test(0x4c);
        assert!(!payload_matches_reference(
            &events::KernelEventPayload::CellProduced(wrong_attempt),
            &reference,
        ));
        assert!(!payload_matches_reference(
            &events::KernelEventPayload::ArtifactReferenced(reference.clone()),
            &reference,
        ));
    }

    #[test]
    fn only_attempt_staged_roles_require_typed_payload_binding() {
        let staged = [
            events::ArtifactRole::StateOutput,
            events::ArtifactRole::FactResponse,
            events::ArtifactRole::ExternalReadEvidence,
            events::ArtifactRole::FactQueryEvidence,
            events::ArtifactRole::PublicOutput,
            events::ArtifactRole::RedactedDiagnostic,
        ];
        for role in events::ArtifactRole::ALL {
            assert_eq!(
                is_attempt_staged_role(*role),
                staged.contains(role),
                "unexpected staging classification for {}",
                role.as_str()
            );
        }
    }
}
