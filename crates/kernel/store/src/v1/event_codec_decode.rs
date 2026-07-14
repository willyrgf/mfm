use super::*;

/// Parses a typed kernel event payload from its store canonical JSON shape.
pub fn payload_from_json_value(json: &serde_json::Value) -> Result<KernelEventPayload> {
    match required_str(json, "variant")? {
        "RunAdmitted" => Ok(KernelEventPayload::RunAdmitted(Box::new(
            events::RunAdmitted {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                identity_material: parse_run_identity_material(required_obj(
                    json,
                    "identity_material",
                )?)?,
                entry_point: parse_entry_point_launch_evidence(required_obj(json, "entry_point")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                spec_artifact: parse_run_artifact(required_obj(json, "spec_artifact")?)?,
                certificate_artifact: parse_run_artifact(required_obj(
                    json,
                    "certificate_artifact",
                )?)?,
                config_artifacts: parse_vec(json, "config_artifacts", parse_run_artifact)?,
                fact_descriptor_artifacts: parse_vec(
                    json,
                    "fact_descriptor_artifacts",
                    parse_run_artifact,
                )?,
                spec_version: parse_identity(required_str(json, "spec_version")?)?,
                lowering_version: parse_identity(required_str(json, "lowering_version")?)?,
                public_output_schema_id: parse_identity(required_str(
                    json,
                    "public_output_schema_id",
                )?)?,
                saga_policy_digest: parse_identity(required_str(json, "saga_policy_digest")?)?,
                descriptor_identities: parse_vec(json, "descriptor_identities", |item| {
                    parse_descriptor_identity(item)
                })?,
                runner_executables: parse_vec(json, "runner_executables", parse_executable)?,
                adapter_executables: parse_vec(json, "adapter_executables", parse_executable)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
                admitted_binding_digest: parse_identity(required_str(
                    json,
                    "admitted_binding_digest",
                )?)?,
                seed_cells: parse_vec(json, "seed_cells", parse_seed_cell_ref)?,
            },
        ))),
        "StateAttemptStarted" => Ok(KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                attempt_no: required_u32(json, "attempt_no")?,
                state_kind: parse_identity(required_str(json, "state_kind")?)?,
                state_version: parse_identity(required_str(json, "state_version")?)?,
            },
        )),
        "FactRecorded" => Ok(KernelEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            claim: parse_fact_claim(required_obj(json, "claim")?)?,
        })),
        "ArtifactReferenced" => Ok(KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: optional_str(json, "node_id")?
                    .map(parse_identity)
                    .transpose()?,
                attempt_id: optional_str(json, "attempt_id")?
                    .map(parse_identity)
                    .transpose()?,
                artifact_ref: parse_event_artifact(required_obj(json, "artifact_ref")?)?,
            },
        )),
        "CellProduced" => Ok(KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            cell_id: parse_identity(required_str(json, "cell_id")?)?,
            scope_id: parse_identity(required_str(json, "scope_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
            context: parse_cell_context(required_obj(json, "context")?)?,
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
            evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
            producer_state_kind: optional_str(json, "producer_state_kind")?
                .map(parse_identity)
                .transpose()?,
            producer_state_version: optional_str(json, "producer_state_version")?
                .map(parse_identity)
                .transpose()?,
        })),
        "CellSkipped" => Ok(KernelEventPayload::CellSkipped(events::CellSkipped {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            cell_id: parse_identity(required_str(json, "cell_id")?)?,
            scope_id: parse_identity(required_str(json, "scope_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
            context: parse_cell_context(required_obj(json, "context")?)?,
            skip_reason: parse_skip_reason(required_obj(json, "skip_reason")?)?,
        })),
        "SideEffectIntentPersisted" => Ok(KernelEventPayload::SideEffectIntentPersisted(
            side_effect::IntentPersisted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                scope_id: parse_identity(required_str(json, "scope_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                intent_schema_id: parse_identity(required_str(json, "intent_schema_id")?)?,
                intent_hash: parse_identity(required_str(json, "intent_hash")?)?,
                intent_artifact_id: parse_identity(required_str(json, "intent_artifact_id")?)?,
                intent_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "intent_artifact_evidence_hash",
                )?)?,
                idempotency_input_schema_id: parse_identity(required_str(
                    json,
                    "idempotency_input_schema_id",
                )?)?,
                idempotency_input_hash: parse_identity(required_str(
                    json,
                    "idempotency_input_hash",
                )?)?,
                idempotency_key: events::IdempotencyKeyRef::new(required_str(
                    json,
                    "idempotency_key",
                )?)?,
                capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
                capability_version: parse_identity(required_str(json, "capability_version")?)?,
                adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
                adapter_version: parse_identity(required_str(json, "adapter_version")?)?,
            },
        )),
        "SideEffectClaimed" => Ok(KernelEventPayload::SideEffectClaimed(
            side_effect::Claimed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "SideEffectClaimTakenOver" => Ok(KernelEventPayload::SideEffectClaimTakenOver(
            side_effect::ClaimTakenOver {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                previous_claim_owner: events::RunnerInvocationId::new(required_str(
                    json,
                    "previous_claim_owner",
                )?)?,
                new_claim_owner: events::RunnerInvocationId::new(required_str(
                    json,
                    "new_claim_owner",
                )?)?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                previous_claim_generation: required_u32(json, "previous_claim_generation")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "ResourceLaneClaimed" => Ok(KernelEventPayload::ResourceLaneClaimed(
            events::ResourceLaneClaimed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                resource_key: parse_resource_key_evidence(required_obj(json, "resource_key")?)?,
                requirement_digest: parse_identity(required_str(json, "requirement_digest")?)?,
                resolved_by_capability_impl: events::RunnerFactoryId::new(required_str(
                    json,
                    "resolved_by_capability_impl",
                )?)?,
                claim_id: events::ResourceLaneClaimId::new(required_str(json, "claim_id")?)?,
                claim_fencing_token: required_u64(json, "claim_fencing_token")?,
                lane_transition_seq: required_u64(json, "lane_transition_seq")?,
            },
        )),
        "SideEffectInvocationPrepared" => Ok(KernelEventPayload::SideEffectInvocationPrepared(
            side_effect::InvocationPrepared {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
                resource_key: optional_obj(json, "resource_key")?
                    .map(parse_resource_key_evidence)
                    .transpose()?,
                prepared_artifact_id: optional_str(json, "prepared_artifact_id")?
                    .map(parse_identity)
                    .transpose()?,
                prepared_hash: optional_str(json, "prepared_hash")?
                    .map(parse_identity)
                    .transpose()?,
                prepared_artifact_evidence_hash: optional_str(
                    json,
                    "prepared_artifact_evidence_hash",
                )?
                .map(parse_identity)
                .transpose()?,
            },
        )),
        "SideEffectInvocationStarted" => Ok(KernelEventPayload::SideEffectInvocationStarted(
            side_effect::InvocationStarted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "SideEffectNotSubmittedProven" => Ok(KernelEventPayload::SideEffectNotSubmittedProven(
            side_effect::NotSubmittedProven {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                proof_schema_id: parse_identity(required_str(json, "proof_schema_id")?)?,
                proof_hash: parse_identity(required_str(json, "proof_hash")?)?,
                proof_artifact_id: parse_identity(required_str(json, "proof_artifact_id")?)?,
                proof_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "proof_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectSubmissionObserved" => Ok(KernelEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                submission_schema_id: parse_identity(required_str(json, "submission_schema_id")?)?,
                submission_hash: parse_identity(required_str(json, "submission_hash")?)?,
                submission_artifact_id: parse_identity(required_str(
                    json,
                    "submission_artifact_id",
                )?)?,
                submission_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "submission_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectSubmissionUnknown" => Ok(KernelEventPayload::SideEffectSubmissionUnknown(
            side_effect::SubmissionUnknown {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectReceiptObserved" => Ok(KernelEventPayload::SideEffectReceiptObserved(
            side_effect::ReceiptObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                receipt_schema_id: parse_identity(required_str(json, "receipt_schema_id")?)?,
                receipt_hash: parse_identity(required_str(json, "receipt_hash")?)?,
                receipt_artifact_id: parse_identity(required_str(json, "receipt_artifact_id")?)?,
                receipt_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "receipt_artifact_evidence_hash",
                )?)?,
                replay_verifier_id: events::ReplayVerifierId::new(required_str(
                    json,
                    "replay_verifier_id",
                )?)?,
                resource_touched_set: optional_obj(json, "resource_touched_set")?
                    .map(parse_resource_touched_set_evidence)
                    .transpose()?,
            },
        )),
        "SideEffectConfirmationObserved" => Ok(KernelEventPayload::SideEffectConfirmationObserved(
            side_effect::ConfirmationObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                confirmation_schema_id: parse_identity(required_str(
                    json,
                    "confirmation_schema_id",
                )?)?,
                confirmation_hash: parse_identity(required_str(json, "confirmation_hash")?)?,
                confirmation_artifact_id: parse_identity(required_str(
                    json,
                    "confirmation_artifact_id",
                )?)?,
                confirmation_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "confirmation_artifact_evidence_hash",
                )?)?,
                replay_verifier_id: events::ReplayVerifierId::new(required_str(
                    json,
                    "replay_verifier_id",
                )?)?,
                resource_touched_set: optional_obj(json, "resource_touched_set")?
                    .map(parse_resource_touched_set_evidence)
                    .transpose()?,
            },
        )),
        "SideEffectAmbiguous" => Ok(KernelEventPayload::SideEffectAmbiguous(
            side_effect::Ambiguous {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                ambiguity_code: events::AmbiguityCode::new(required_str(json, "ambiguity_code")?)?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectFailed" => Ok(KernelEventPayload::SideEffectFailed(side_effect::Failed {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
            ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                json,
                "ledger_purpose",
            )?)?,
            pair_id: parse_identity(required_str(json, "pair_id")?)?,
            pair_role: parse_side_effect_pair_role(json, "pair_role")?,
            invocation_epoch: required_u32(json, "invocation_epoch")?,
            failure_phase: parse_failure_phase(required_str(json, "failure_phase")?)?,
            retryable: required_bool(json, "retryable")?,
            error: parse_error_info(required_obj(json, "error")?)?,
        })),
        "ResourceLaneReleased" => Ok(KernelEventPayload::ResourceLaneReleased(
            events::ResourceLaneReleased {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_id: events::ResourceLaneClaimId::new(required_str(json, "claim_id")?)?,
                release_id: events::ResourceLaneReleaseId::new(required_str(json, "release_id")?)?,
                claim_fencing_token: required_u64(json, "claim_fencing_token")?,
                release_authority: parse_resource_lane_release_authority(required_str(
                    json,
                    "release_authority",
                )?)?,
                release_reason: events::ResourceLaneReleaseReason::new(required_str(
                    json,
                    "release_reason",
                )?)?,
                lane_transition_seq: required_u64(json, "lane_transition_seq")?,
            },
        )),
        "PublicOutputProduced" => Ok(KernelEventPayload::PublicOutputProduced(
            events::PublicOutputProduced {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                receipt_cell_id: parse_identity(required_str(json, "receipt_cell_id")?)?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                output_spec_digest: parse_identity(required_str(json, "output_spec_digest")?)?,
                cells: parse_vec(json, "cells", parse_named_cell_ref)?,
                rendered_digest: parse_identity(required_str(json, "rendered_digest")?)?,
                rendered_artifact_id: optional_str(json, "rendered_artifact_id")?
                    .map(parse_identity)
                    .transpose()?,
                rendered_artifact_evidence_hash: optional_str(
                    json,
                    "rendered_artifact_evidence_hash",
                )?
                .map(parse_identity)
                .transpose()?,
                renderer_descriptor_id: parse_identity(required_str(
                    json,
                    "renderer_descriptor_id",
                )?)?,
            },
        )),
        "PublicOutputRenderFailed" => Ok(KernelEventPayload::PublicOutputRenderFailed(
            events::PublicOutputRenderFailed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                renderer_descriptor_id: parse_identity(required_str(
                    json,
                    "renderer_descriptor_id",
                )?)?,
                error: parse_error_info(required_obj(json, "error")?)?,
            },
        )),
        "StateAttemptCompleted" => Ok(KernelEventPayload::StateAttemptCompleted(
            events::StateAttemptCompleted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                output_cell_id: parse_identity(required_str(json, "output_cell_id")?)?,
            },
        )),
        "StateAttemptInterrupted" => Ok(KernelEventPayload::StateAttemptInterrupted(
            events::StateAttemptInterrupted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            },
        )),
        "StateAttemptFailed" => Ok(KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                retryable: required_bool(json, "retryable")?,
                error: parse_error_info(required_obj(json, "error")?)?,
            },
        )),
        "ManualResolutionRecorded" => Ok(KernelEventPayload::ManualResolutionRecorded(
            events::ManualResolutionRecorded {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                outcome: parse_manual_resolution_outcome(required_str(json, "outcome")?)?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
                authorization_schema_id: parse_identity(required_str(
                    json,
                    "authorization_schema_id",
                )?)?,
                authorization_hash: parse_identity(required_str(json, "authorization_hash")?)?,
                authorization_artifact_id: parse_identity(required_str(
                    json,
                    "authorization_artifact_id",
                )?)?,
                authorization_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "authorization_artifact_evidence_hash",
                )?)?,
                note: optional_obj(json, "note")?
                    .map(parse_manual_resolution_note)
                    .transpose()?,
            },
        )),
        "RunCompleted" => Ok(KernelEventPayload::RunCompleted(events::RunCompleted {
            run_id: parse_identity(required_str(json, "run_id")?)?,
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            outcome: parse_run_completion_outcome(required_obj(json, "outcome")?)?,
        })),
        "RetentionRefsAppended" => Ok(KernelEventPayload::RetentionRefsAppended(
            events::RetentionRefsAppended {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                refs: parse_vec(json, "refs", parse_retention_ref)?,
                reason: parse_retention_reason(required_str(json, "reason")?)?,
            },
        )),
        "RetentionManifestProjected" => Ok(KernelEventPayload::RetentionManifestProjected(
            events::RetentionManifestProjected {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                manifest_seq: required_u64(json, "manifest_seq")?,
                manifest_digest: parse_identity(required_str(json, "manifest_digest")?)?,
                previous_manifest_digest: optional_str(json, "previous_manifest_digest")?
                    .map(parse_identity)
                    .transpose()?,
                manifest_artifact_id: parse_identity(required_str(json, "manifest_artifact_id")?)?,
                manifest_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "manifest_artifact_evidence_hash",
                )?)?,
            },
        )),
        other => Err(StoreError::Event(format!(
            "unknown event payload variant {other}"
        ))),
    }
}

fn parse_descriptor_identity(json: &serde_json::Value) -> Result<DescriptorIdentity> {
    match required_str(json, "descriptor_family")? {
        "state" => Ok(DescriptorIdentity::State(Box::new(
            StateDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                name: required_str(json, "name")?.to_owned(),
                state_kind: parse_identity(required_str(json, "state_kind")?)?,
                state_version: parse_identity(required_str(json, "state_version")?)?,
                context: parse_state_context_descriptor(required_obj(json, "context")?)?,
                input_context: parse_state_input_context_contract(required_obj(
                    json,
                    "input_context",
                )?)?,
                output_context: parse_state_output_context_contract(required_obj(
                    json,
                    "output_context",
                )?)?,
                config_schema_id: parse_identity(required_str(json, "config_schema_id")?)?,
                input_schema_id: parse_identity(required_str(json, "input_schema_id")?)?,
                output_schema_id: parse_identity(required_str(json, "output_schema_id")?)?,
                output_semantic_type_id: parse_identity(required_str(
                    json,
                    "output_semantic_type_id",
                )?)?,
                effect_kind: parse_identity(required_str(json, "effect_kind")?)?,
                effect_class: required_str(json, "effect_class")?.to_owned(),
                effect_name: required_str(json, "effect_name")?.to_owned(),
                effect_version: parse_identity(required_str(json, "effect_version")?)?,
                capabilities: parse_capability_set(required_obj(json, "capabilities")?)?,
                emitted_fact_descriptors: parse_vec(
                    json,
                    "emitted_fact_descriptors",
                    parse_fact_descriptor_ref,
                )?,
                runner: required_str(json, "runner")?.to_owned(),
                side_effect_contract_digest: optional_str(json, "side_effect_contract_digest")?
                    .map(parse_identity)
                    .transpose()?,
            },
        ))),
        "operation" => Ok(DescriptorIdentity::Operation(Box::new(
            OperationDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                name: required_str(json, "name")?.to_owned(),
                operation_kind: parse_identity(required_str(json, "operation_kind")?)?,
                operation_version: parse_identity(required_str(json, "operation_version")?)?,
                config_schema_id: parse_identity(required_str(json, "config_schema_id")?)?,
                input_schema_id: parse_identity(required_str(json, "input_schema_id")?)?,
                output_schema_id: parse_identity(required_str(json, "output_schema_id")?)?,
                expansion_abi: required_str(json, "expansion_abi")?.to_owned(),
            },
        ))),
        "renderer" => Ok(DescriptorIdentity::Renderer(Box::new(
            RendererDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                renderer_kind: RendererKind::new(required_str(json, "renderer_kind")?)
                    .map_err(|error| StoreError::Identity(error.to_string()))?,
                renderer_version: RendererVersion::new(required_str(json, "renderer_version")?)
                    .map_err(|error| StoreError::Identity(error.to_string()))?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            },
        ))),
        other => Err(StoreError::Identity(format!(
            "unknown descriptor identity family {other}"
        ))),
    }
}

fn parse_state_context_descriptor(
    json: &serde_json::Value,
) -> Result<spec::StateContextDescriptorSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateContextDescriptorSpec::no_context()),
        "required" => Ok(spec::StateContextDescriptorSpec::Required(Box::new(
            spec::StateContextDescriptorRequirementSpec {
                context_descriptor_id: parse_identity(required_str(
                    json,
                    "context_descriptor_id",
                )?)?,
                schema_id: parse_identity(required_str(json, "schema_id")?)?,
                semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            },
        ))),
        kind => Err(StoreError::Event(format!(
            "unknown state context descriptor kind {kind}"
        ))),
    }
}

fn parse_context_producer(json: &serde_json::Value) -> Result<spec::ContextProducerSpec> {
    Ok(spec::ContextProducerSpec {
        producer_descriptor_ids: required_array(json, "producer_descriptor_ids")?
            .iter()
            .map(|value| {
                let raw = value.as_str().ok_or_else(|| {
                    StoreError::Event("producer_descriptor_ids entries must be strings".to_owned())
                })?;
                parse_identity(raw).map_err(StoreError::from)
            })
            .collect::<Result<Vec<_>>>()?,
        seed_producers_allowed: required_bool(json, "seed_producers_allowed")?,
    })
}

fn parse_state_input_context_contract(
    json: &serde_json::Value,
) -> Result<spec::StateInputContextContractSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateInputContextContractSpec::no_context()),
        "required" => Ok(spec::StateInputContextContractSpec::Required {
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required_obj(json, "producer")?)?),
        }),
        kind => Err(StoreError::Event(format!(
            "unknown state input context contract kind {kind}"
        ))),
    }
}

fn parse_state_output_context_contract(
    json: &serde_json::Value,
) -> Result<spec::StateOutputContextContractSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateOutputContextContractSpec::no_context()),
        "produces" => Ok(spec::StateOutputContextContractSpec::Produces {
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
        }),
        kind => Err(StoreError::Event(format!(
            "unknown state output context contract kind {kind}"
        ))),
    }
}

fn parse_cell_context(json: &serde_json::Value) -> Result<spec::CellContextSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::CellContextSpec::no_context()),
        "bound" => Ok(spec::CellContextSpec::Bound {
            context_ref: parse_identity(required_str(json, "context_ref")?)?,
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required_obj(json, "producer")?)?),
        }),
        kind => Err(StoreError::Event(format!(
            "unknown cell context kind {kind}"
        ))),
    }
}

fn parse_capability_set(json: &serde_json::Value) -> Result<CapabilitySetDescriptor> {
    let capabilities = required_array(json, "capabilities")?
        .iter()
        .map(parse_capability)
        .collect::<Result<Vec<_>>>()?;
    Ok(CapabilitySetDescriptor::new(capabilities)?)
}

fn parse_capability(json: &serde_json::Value) -> Result<CapabilityDescriptor> {
    Ok(CapabilityDescriptor::new(
        parse_identity::<CapabilityKind>(required_str(json, "kind")?)?,
        parse_identity::<CapabilityVersion>(required_str(json, "version")?)?,
        parse_capability_role(required_str(json, "role")?)?,
        required_str(json, "name")?.to_owned(),
    )?)
}

fn parse_executable(json: &serde_json::Value) -> Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id: events::RunnerFactoryId::new(required_str(json, "factory_id")?)?,
        cargo_package_digest: parse_identity(required_str(json, "cargo_package_digest")?)?,
        binary_digest: parse_identity(required_str(json, "binary_digest")?)?,
        nix_derivation_hash: optional_str(json, "nix_derivation_hash")?
            .map(events::NixDerivationHash::new)
            .transpose()?,
        nix_output_hash: optional_str(json, "nix_output_hash")?
            .map(events::NixOutputHash::new)
            .transpose()?,
    })
}

fn parse_seed_cell_ref(json: &serde_json::Value) -> Result<events::SeedCellRef> {
    Ok(events::SeedCellRef {
        seed_id: parse_identity(required_str(json, "seed_id")?)?,
        cell_id: parse_identity(required_str(json, "cell_id")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        digest: parse_identity(required_str(json, "digest")?)?,
        seed_artifact: parse_event_artifact(required_obj(json, "seed_artifact")?)?,
    })
}

fn parse_named_cell_ref(json: &serde_json::Value) -> Result<events::NamedTypedCellRef> {
    Ok(events::NamedTypedCellRef {
        public_field_path: PublicFieldPath::new(required_str(json, "public_field_path")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
        cell_id: parse_identity(required_str(json, "cell_id")?)?,
        producer: parse_cell_producer(required_obj(json, "producer")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
    })
}

fn parse_cell_producer(json: &serde_json::Value) -> Result<CellProducer> {
    match required_str(json, "kind")? {
        "node" => Ok(CellProducer::Node(parse_identity(required_str(
            json, "node_id",
        )?)?)),
        "seed" => Ok(CellProducer::Seed(parse_identity(required_str(
            json, "seed_id",
        )?)?)),
        other => Err(StoreError::Identity(format!(
            "unknown cell producer {other}"
        ))),
    }
}

fn parse_value_lineage(json: &serde_json::Value) -> Result<ValueLineageRef> {
    Ok(ValueLineageRef {
        lineage_digest: parse_identity(required_str(json, "lineage_digest")?)?,
    })
}

/// Parses an artifact evidence reference from canonical JSON.
pub fn parse_event_artifact(json: &serde_json::Value) -> CodecResult<events::ArtifactEvidenceRef> {
    Ok(events::ArtifactEvidenceRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        semantic_type_id: optional_str(json, "semantic_type_id")?
            .map(parse_identity)
            .transpose()?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        byte_len: required_u64(json, "byte_len")?,
        media_type: MediaType::new(required_str(json, "media_type")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
    })
}

fn parse_fact_claim(json: &serde_json::Value) -> Result<mfm_facts::FactClaim> {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: parse_fact_visibility(required_obj(json, "visibility")?)?,
        fact_kind: mfm_facts::FactKind::new(required_str(json, "fact_kind")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
        fact_descriptor_hash: parse_identity(required_str(json, "fact_descriptor_hash")?)?,
        subject: parse_fact_subject_evidence(required_obj(json, "subject")?)?,
        observed_at: optional_str(json, "observed_at")?.map(str::to_owned),
        request: optional_obj(json, "request")?
            .map(parse_fact_request_evidence)
            .transpose()?,
        response: parse_fact_response_evidence(required_obj(json, "response")?)?,
        producer: parse_fact_producer_provenance(required_obj(json, "producer")?)?,
    })
    .map_err(|error| StoreError::Identity(error.to_string()))
}

fn parse_fact_visibility(json: &serde_json::Value) -> Result<mfm_facts::FactVisibility> {
    match required_str(json, "kind")? {
        "run_private" => Ok(mfm_facts::FactVisibility::RunPrivate),
        "indexed" => Ok(mfm_facts::FactVisibility::Indexed {
            audience: parse_fact_audience(required_str(json, "audience")?)?,
            scope: parse_fact_visibility_scope(required_str(json, "scope")?)?,
        }),
        other => Err(StoreError::Identity(format!(
            "unknown fact visibility {other}"
        ))),
    }
}

fn parse_fact_audience(value: &str) -> Result<mfm_facts::FactAudience> {
    value
        .parse::<mfm_facts::FactAudience>()
        .map_err(|_| StoreError::Identity(format!("unknown fact audience {value}")))
}

fn parse_fact_visibility_scope(value: &str) -> Result<mfm_facts::FactVisibilityScope> {
    value
        .parse::<mfm_facts::FactVisibilityScope>()
        .map_err(|_| StoreError::Identity(format!("unknown fact visibility scope {value}")))
}

fn parse_fact_subject_evidence(json: &serde_json::Value) -> Result<mfm_facts::FactSubjectEvidence> {
    mfm_facts::FactSubjectEvidence::new(
        parse_identity(required_str(json, "fact_subject_namespace_hash")?)?,
        PlainCanonicalJsonBytes::from_canonical_json_slice(
            required_str(json, "subject_material")?.as_bytes(),
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?,
        parse_identity(required_str(json, "subject_material_hash")?)?,
        mfm_facts::FactKey::from_digest(parse_identity(required_str(json, "fact_key")?)?),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))
}

fn parse_fact_request_evidence(json: &serde_json::Value) -> Result<mfm_facts::FactRequestEvidence> {
    Ok(mfm_facts::FactRequestEvidence::new(
        parse_identity(required_str(json, "request_schema_id")?)?,
        parse_identity(required_str(json, "request_hash")?)?,
    ))
}

fn parse_fact_response_evidence(
    json: &serde_json::Value,
) -> Result<mfm_facts::FactResponseEvidence> {
    Ok(mfm_facts::FactResponseEvidence::new(
        parse_identity(required_str(json, "response_schema_id")?)?,
        parse_identity(required_str(json, "response_hash")?)?,
        parse_identity(required_str(json, "artifact_id")?)?,
        parse_identity(required_str(json, "artifact_evidence_hash")?)?,
    ))
}

fn parse_fact_producer_provenance(
    json: &serde_json::Value,
) -> Result<mfm_facts::FactProducerProvenance> {
    Ok(mfm_facts::FactProducerProvenance::new(
        parse_identity(required_str(json, "capability_kind")?)?,
        required_str(json, "capability_version")?.parse()?,
        parse_identity(required_str(json, "adapter_kind")?)?,
        required_str(json, "adapter_version")?.parse()?,
    ))
}

fn parse_run_artifact(json: &serde_json::Value) -> Result<events::RunArtifactEvidenceRef> {
    Ok(events::RunArtifactEvidenceRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        schema_id: optional_str(json, "schema_id")?
            .map(parse_identity)
            .transpose()?,
        semantic_type_id: optional_str(json, "semantic_type_id")?
            .map(parse_identity)
            .transpose()?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        byte_len: required_u64(json, "byte_len")?,
        media_type: MediaType::new(required_str(json, "media_type")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
    })
}

fn parse_entry_point_launch_evidence(
    json: &serde_json::Value,
) -> Result<events::EntryPointLaunchEvidence> {
    Ok(events::EntryPointLaunchEvidence {
        resolved_op_id: events::EntryPointOpId::new(required_str(json, "resolved_op_id")?)?,
        entry_point_registry_digest: parse_identity(required_str(
            json,
            "entry_point_registry_digest",
        )?)?,
    })
}

fn parse_run_identity_material(json: &serde_json::Value) -> Result<events::RunIdentityMaterialV1> {
    Ok(events::RunIdentityMaterialV1 {
        certified_spec_hash: parse_identity(required_str(json, "certified_spec_hash")?)?,
        store_scope_id: StoreScopeId::new(required_str(json, "store_scope_id")?)?,
        invocation_key_digest: parse_identity(required_str(json, "invocation_key_digest")?)?,
    })
}

/// Parses a cell skip reason from canonical JSON.
pub fn parse_skip_reason(json: &serde_json::Value) -> CodecResult<events::SkipReason> {
    Ok(events::SkipReason {
        code: events::ErrorCode::new(required_str(json, "code")?)?,
        safe_message: required_str(json, "safe_message")?.to_owned(),
    })
}

/// Parses a side-effect ledger purpose from canonical JSON.
pub fn parse_side_effect_ledger_purpose(
    json: &serde_json::Value,
) -> CodecResult<events::SideEffectLedgerPurpose> {
    match required_str(json, "kind")? {
        "forward" => Ok(events::SideEffectLedgerPurpose::Forward),
        "remediation" => Ok(events::SideEffectLedgerPurpose::Remediation {
            forward_pair_id: parse_identity(required_str(json, "forward_pair_id")?)?,
        }),
        other => Err(CodecError::Identity(format!(
            "unknown side-effect ledger purpose {other}"
        ))),
    }
}

fn parse_side_effect_pair_role(
    json: &serde_json::Value,
    field: &'static str,
) -> CodecResult<events::SideEffectPairRole> {
    let value = required_str(json, field)?;
    events::SideEffectPairRole::parse(value)
        .ok_or_else(|| CodecError::Identity(format!("unknown side-effect pair role {value}")))
}

/// Parses exclusive resource-key evidence from canonical JSON.
pub fn parse_resource_key_evidence(
    json: &serde_json::Value,
) -> CodecResult<events::ResourceKeyEvidence> {
    Ok(events::ResourceKeyEvidence {
        namespace: ResourceNamespace::new(required_str(json, "namespace")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
        key_schema_id: parse_identity(required_str(json, "key_schema_id")?)?,
        key: events::ResourceKey::new(required_str(json, "key")?)?,
    })
}

/// Parses touched-set resource evidence from canonical JSON.
pub fn parse_resource_touched_set_evidence(
    json: &serde_json::Value,
) -> CodecResult<events::ResourceTouchedSetEvidence> {
    Ok(events::ResourceTouchedSetEvidence {
        namespace: ResourceNamespace::new(required_str(json, "namespace")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
        evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
        evidence_artifact_evidence_hash: parse_identity(required_str(
            json,
            "evidence_artifact_evidence_hash",
        )?)?,
    })
}

/// Parses a manual resolution outcome tag.
pub fn parse_manual_resolution_outcome(
    value: &str,
) -> CodecResult<events::ManualResolutionOutcome> {
    match value {
        "confirm_remediated" => Ok(events::ManualResolutionOutcome::ConfirmRemediated),
        "fail_without_acdc_claim" => Ok(events::ManualResolutionOutcome::FailWithoutAcdcClaim),
        other => Err(CodecError::Identity(format!(
            "unknown manual resolution outcome {other}"
        ))),
    }
}

/// Parses a manual resolution note from canonical JSON.
pub fn parse_manual_resolution_note(
    json: &serde_json::Value,
) -> CodecResult<events::ManualResolutionNote> {
    Ok(events::ManualResolutionNote::new(required_str(
        json, "text",
    )?)?)
}

/// Parses structured error info from canonical JSON.
pub fn parse_error_info(json: &serde_json::Value) -> CodecResult<events::MfmErrorInfo> {
    let public_details = optional_obj(json, "public_details")?
        .map(|details| {
            Ok::<events::RedactedJson, CodecError>(events::RedactedJson::new(parse_identity(
                required_str(details, "content_digest")?,
            )?))
        })
        .transpose()?;
    let diagnostic_ref = optional_obj(json, "diagnostic_ref")?
        .map(parse_event_artifact)
        .transpose()?;
    let mut error = events::MfmErrorInfo::new(
        events::ErrorCode::new(required_str(json, "code")?)?,
        parse_error_category(required_str(json, "category")?)?,
        required_bool(json, "retryable")?,
        required_str(json, "safe_message")?,
    )?;
    if let Some(public_details) = public_details {
        error = error.with_public_details(public_details)?;
    }
    if let Some(diagnostic_ref) = diagnostic_ref {
        error = error.with_diagnostic_ref(diagnostic_ref)?;
    }
    Ok(error)
}

/// Parses a run completion outcome from canonical JSON.
pub fn parse_run_completion_outcome(
    json: &serde_json::Value,
) -> CodecResult<events::RunCompletionOutcome> {
    match required_str(json, "kind")? {
        "completed" => {
            let evidence = required_obj(json, "public_output")?;
            Ok(events::RunCompletionOutcome::Completed(Box::new(
                events::PublicOutputCompletionEvidence {
                    public_output_schema_id: parse_identity(required_str(
                        evidence,
                        "public_output_schema_id",
                    )?)?,
                    public_output_event_id: parse_identity(required_str(
                        evidence,
                        "public_output_event_id",
                    )?)?,
                },
            )))
        }
        "compensated" => Ok(events::RunCompletionOutcome::Compensated),
        "manually_resolved" => Ok(events::RunCompletionOutcome::ManuallyResolved),
        "failed_without_acdc_claim" => Ok(events::RunCompletionOutcome::FailedWithoutAcdcClaim),
        other => Err(CodecError::Identity(format!(
            "unknown run completion outcome {other}"
        ))),
    }
}

fn parse_fact_descriptor_ref(json: &serde_json::Value) -> Result<spec::FactDescriptorRef> {
    Ok(spec::FactDescriptorRef {
        descriptor_hash: parse_identity(required_str(json, "descriptor_hash")?)?,
    })
}

fn parse_retention_ref(json: &serde_json::Value) -> Result<events::RetentionRef> {
    Ok(events::RetentionRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
    })
}

fn decode_artifact_role_tag(value: &str) -> CodecResult<ArtifactRole> {
    ArtifactRole::parse(value)
        .ok_or_else(|| CodecError::Identity(format!("unknown artifact role {value}")))
}

fn parse_capability_role(value: &str) -> Result<CapabilityRole> {
    match value {
        "read_external" => Ok(CapabilityRole::ReadExternal),
        "managed_platform_write" => Ok(CapabilityRole::ManagedPlatformWrite),
        "support" => Ok(CapabilityRole::Support),
        "external_mutation_authority" => Ok(CapabilityRole::ExternalMutationAuthority),
        other => Err(StoreError::Identity(format!(
            "unknown capability role {other}"
        ))),
    }
}

/// Parses an error category tag.
pub fn parse_error_category(value: &str) -> CodecResult<events::ErrorCategory> {
    match value {
        "planning" => Ok(events::ErrorCategory::Planning),
        "validation" => Ok(events::ErrorCategory::Validation),
        "capability" => Ok(events::ErrorCategory::Capability),
        "side_effect" => Ok(events::ErrorCategory::SideEffect),
        "runtime" => Ok(events::ErrorCategory::Runtime),
        "storage" => Ok(events::ErrorCategory::Storage),
        "cancelled" => Ok(events::ErrorCategory::Cancelled),
        other => Err(CodecError::Identity(format!(
            "unknown error category {other}"
        ))),
    }
}

/// Parses a side-effect failure phase tag.
pub fn parse_failure_phase(value: &str) -> CodecResult<side_effect::FailurePhase> {
    match value {
        "before_invocation_started" => Ok(side_effect::FailurePhase::BeforeInvocationStarted),
        "after_not_submitted_proven" => Ok(side_effect::FailurePhase::AfterNotSubmittedProven),
        other => Err(CodecError::Identity(format!(
            "unknown failure phase {other}"
        ))),
    }
}

fn parse_retention_reason(value: &str) -> Result<events::RetentionReason> {
    match value {
        "run_admitted" => Ok(events::RetentionReason::RunAdmitted),
        "runtime_evidence" => Ok(events::RetentionReason::RuntimeEvidence),
        "public_output" => Ok(events::RetentionReason::PublicOutput),
        "manifest_projection" => Ok(events::RetentionReason::ManifestProjection),
        other => Err(StoreError::Identity(format!(
            "unknown retention reason {other}"
        ))),
    }
}

fn parse_resource_lane_release_authority(
    value: &str,
) -> Result<events::ResourceLaneReleaseAuthority> {
    events::ResourceLaneReleaseAuthority::parse(value).map_err(|error| {
        StoreError::Identity(format!("invalid resource lane release authority: {error}"))
    })
}

pub(super) fn parse_vec<T>(
    json: &serde_json::Value,
    field: &'static str,
    parser: impl Fn(&serde_json::Value) -> Result<T>,
) -> Result<Vec<T>> {
    required_array(json, field)?.iter().map(parser).collect()
}

fn required_array<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> Result<&'a [serde_json::Value]> {
    json.get(field)
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| StoreError::Event(format!("missing array field {field}")))
}
