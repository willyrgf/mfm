use super::*;

macro_rules! with_side_effect_payload_mut {
    ($payload:expr, $inner:ident, $body:block) => {
        match $payload {
            KernelEventPayload::SideEffectIntentPersisted($inner) => $body,
            KernelEventPayload::SideEffectClaimed($inner) => $body,
            KernelEventPayload::SideEffectClaimTakenOver($inner) => $body,
            KernelEventPayload::ResourceLaneClaimed($inner) => $body,
            KernelEventPayload::ResourceLaneClaimIntent($inner) => $body,
            KernelEventPayload::ResourceLaneReleased($inner) => $body,
            KernelEventPayload::ResourceLaneReleaseIntent($inner) => $body,
            KernelEventPayload::SideEffectInvocationPrepared($inner) => $body,
            KernelEventPayload::SideEffectInvocationStarted($inner) => $body,
            KernelEventPayload::SideEffectNotSubmittedProven($inner) => $body,
            KernelEventPayload::SideEffectSubmissionObserved($inner) => $body,
            KernelEventPayload::SideEffectSubmissionUnknown($inner) => $body,
            KernelEventPayload::SideEffectReceiptObserved($inner) => $body,
            KernelEventPayload::SideEffectConfirmationObserved($inner) => $body,
            KernelEventPayload::SideEffectAmbiguous($inner) => $body,
            KernelEventPayload::SideEffectFailed($inner) => $body,
            _ => unreachable!("payload is not side-effect evidence"),
        }
    };
}

pub(super) fn set_remediation_purpose(
    payload: &mut KernelEventPayload,
    ledger_key: events::SideEffectLedgerKey,
) {
    set_side_effect_ledger(payload, ledger_key, remediation_ledger_purpose());
    with_side_effect_payload_mut!(payload, inner, {
        inner.pair_id = remediation_pair_id();
    });
    macro_rules! set_remediation_emitter {
        ($inner:ident) => {
            match $inner.pair_role {
                events::SideEffectPairRole::Submit => {
                    $inner.node_id = remediation_submit_node_id();
                    $inner.attempt_id = remediation_submit_attempt_id();
                }
                events::SideEffectPairRole::Verify => {
                    $inner.node_id = remediation_verify_node_id();
                    $inner.attempt_id = remediation_verify_attempt_id();
                }
            }
        };
    }
    match payload {
        KernelEventPayload::SideEffectIntentPersisted(inner) => {
            set_remediation_emitter!(inner);
            inner.scope_id = scope_id(181);
            inner.intent_artifact_evidence_hash = intent_artifact_ref_for_node(
                inner.intent_artifact_id.clone(),
                inner.intent_hash.clone(),
                inner.node_id.clone(),
            )
            .evidence_hash()
            .expect("remediation intent evidence hash");
        }
        KernelEventPayload::SideEffectClaimed(inner) => set_remediation_emitter!(inner),
        KernelEventPayload::SideEffectClaimTakenOver(inner) => set_remediation_emitter!(inner),
        KernelEventPayload::ResourceLaneClaimed(inner) => set_remediation_emitter!(inner),
        KernelEventPayload::ResourceLaneClaimIntent(inner) => set_remediation_emitter!(inner),
        KernelEventPayload::SideEffectInvocationPrepared(inner) => set_remediation_emitter!(inner),
        KernelEventPayload::SideEffectInvocationStarted(inner) => set_remediation_emitter!(inner),
        KernelEventPayload::SideEffectNotSubmittedProven(inner) => {
            set_remediation_emitter!(inner);
            inner.proof_artifact_evidence_hash = remediation_side_effect_evidence(
                inner.proof_artifact_id.clone(),
                inner.proof_hash.clone(),
                inner.proof_schema_id.clone(),
                ArtifactRole::NotSubmittedProof,
            )
            .evidence_hash()
            .expect("remediation proof evidence hash");
        }
        KernelEventPayload::SideEffectSubmissionObserved(inner) => {
            set_remediation_emitter!(inner);
            inner.submission_artifact_evidence_hash = remediation_side_effect_evidence(
                inner.submission_artifact_id.clone(),
                inner.submission_hash.clone(),
                inner.submission_schema_id.clone(),
                ArtifactRole::Submission,
            )
            .evidence_hash()
            .expect("remediation submission evidence hash");
        }
        KernelEventPayload::SideEffectSubmissionUnknown(inner) => {
            set_remediation_emitter!(inner);
            inner.evidence_artifact_evidence_hash = remediation_side_effect_evidence(
                inner.evidence_artifact_id.clone(),
                inner.evidence_hash.clone(),
                inner.evidence_schema_id.clone(),
                ArtifactRole::SubmissionUnknownEvidence,
            )
            .evidence_hash()
            .expect("remediation submission-unknown evidence hash");
        }
        KernelEventPayload::SideEffectReceiptObserved(inner) => {
            set_remediation_emitter!(inner);
            inner.receipt_artifact_evidence_hash = remediation_side_effect_evidence(
                inner.receipt_artifact_id.clone(),
                inner.receipt_hash.clone(),
                inner.receipt_schema_id.clone(),
                ArtifactRole::Receipt,
            )
            .evidence_hash()
            .expect("remediation receipt evidence hash");
        }
        KernelEventPayload::SideEffectConfirmationObserved(inner) => {
            set_remediation_emitter!(inner);
            inner.confirmation_artifact_evidence_hash = remediation_side_effect_evidence(
                inner.confirmation_artifact_id.clone(),
                inner.confirmation_hash.clone(),
                inner.confirmation_schema_id.clone(),
                ArtifactRole::Confirmation,
            )
            .evidence_hash()
            .expect("remediation confirmation evidence hash");
        }
        KernelEventPayload::SideEffectAmbiguous(inner) => {
            set_remediation_emitter!(inner);
            inner.evidence_artifact_evidence_hash = remediation_side_effect_evidence(
                inner.evidence_artifact_id.clone(),
                inner.evidence_hash.clone(),
                inner.evidence_schema_id.clone(),
                ArtifactRole::AmbiguityEvidence,
            )
            .evidence_hash()
            .expect("remediation ambiguity evidence hash");
        }
        KernelEventPayload::SideEffectFailed(inner) => set_remediation_emitter!(inner),
        KernelEventPayload::ResourceLaneReleased(_)
        | KernelEventPayload::ResourceLaneReleaseIntent(_) => {}
        _ => unreachable!("payload is not side-effect evidence"),
    }
}

pub(super) fn set_payload_spec_hash(payload: &mut KernelEventPayload, spec_hash: &SpecHash) {
    match payload {
        KernelEventPayload::RunAdmitted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::StateAttemptStarted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::FactRecorded(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ArtifactReferenced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::CellProduced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::CellSkipped(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            payload.spec_hash = spec_hash.clone()
        }
        KernelEventPayload::SideEffectClaimed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            payload.spec_hash = spec_hash.clone()
        }
        KernelEventPayload::ResourceLaneClaimed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneClaimIntent(payload) => {
            payload.spec_hash = spec_hash.clone()
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectInvocationStarted(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectReceiptObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectAmbiguous(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectFailed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneReleased(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneReleaseIntent(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::PublicOutputProduced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::StateAttemptCompleted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::StateAttemptFailed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::RunCompleted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::RetentionRefsAppended(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::RetentionManifestProjected(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
    }
}

pub(super) fn certify_payloads_for_policy(
    run_id: &RunId,
    policy: SagaPolicySpec,
    payloads: &mut [KernelEventPayload],
) -> CommitPreconditions {
    let spec = saga_authority_spec(policy);
    let spec_hash = spec.spec_hash().expect("saga authority spec hash");
    for payload in payloads {
        set_payload_spec_hash(payload, &spec_hash);
    }
    CommitPreconditions {
        certified_run_authority: Some(
            CertifiedRunStoreAuthority::from_spec(run_id.clone(), &spec)
                .expect("certified run authority"),
        ),
        ..CommitPreconditions::default()
    }
}

pub(super) fn set_side_effect_ledger(
    payload: &mut KernelEventPayload,
    ledger_key: events::SideEffectLedgerKey,
    purpose: events::SideEffectLedgerPurpose,
) {
    with_side_effect_payload_mut!(payload, inner, {
        inner.ledger_key = ledger_key.clone();
        inner.ledger_purpose = purpose.clone();
    });
}

pub(super) fn set_side_effect_node_attempt(
    payload: &mut KernelEventPayload,
    node_id: NodeId,
    attempt_id: AttemptId,
) {
    match payload {
        KernelEventPayload::SideEffectIntentPersisted(inner) => {
            inner.node_id = node_id.clone();
            inner.attempt_id = attempt_id;
            // Keep the event's exact evidence key aligned with store producer identity.
            inner.intent_artifact_evidence_hash = intent_artifact_ref_for_node(
                inner.intent_artifact_id.clone(),
                inner.intent_hash.clone(),
                node_id,
            )
            .evidence_hash()
            .expect("intent evidence hash after node assignment");
        }
        KernelEventPayload::SideEffectClaimed(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectClaimTakenOver(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::ResourceLaneClaimed(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::ResourceLaneClaimIntent(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectInvocationPrepared(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectInvocationStarted(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectNotSubmittedProven(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectSubmissionObserved(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectSubmissionUnknown(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectReceiptObserved(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectConfirmationObserved(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectAmbiguous(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        KernelEventPayload::SideEffectFailed(inner) => {
            inner.node_id = node_id;
            inner.attempt_id = attempt_id;
        }
        _ => unreachable!("payload does not carry side-effect emitter attribution"),
    }
}

pub(super) fn set_attempt_failure_node_attempt(
    payload: &mut KernelEventPayload,
    node_id: NodeId,
    attempt_id: AttemptId,
) {
    let KernelEventPayload::StateAttemptFailed(payload) = payload else {
        unreachable!("payload is not attempt failure evidence");
    };
    payload.node_id = node_id;
    payload.attempt_id = attempt_id;
}
