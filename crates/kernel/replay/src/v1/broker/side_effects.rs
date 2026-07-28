use super::*;

impl ReplayBroker<'_> {
    /// Returns retained side-effect intent evidence from replay records only.
    pub fn side_effect_intent_evidence(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<SideEffectIntentReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let intent = self.side_effect_intent(&request.pair_id)?;
        let artifact = self.verify_artifact(ArtifactEvidenceExpectation {
            artifact_id: &intent.intent_artifact_id,
            evidence_hash: &intent.intent_artifact_evidence_hash,
            digest: &intent.intent_hash,
            schema_id: Some(&intent.intent_schema_id),
            semantic_type_id: None,
            role: ArtifactRole::SideEffectIntent,
            producer_node_id: Some(&intent.node_id),
            producer_seed_id: None,
        })?;
        Ok(SideEffectIntentReplayEvidence {
            intent: intent.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        })
    }

    pub(super) fn verify_requested_side_effect_artifact<T>(
        &self,
        request: &SideEffectEvidenceReplayRequest,
        evidence: &T,
    ) -> Result<StoredArtifactEvidenceRef>
    where
        T: SideEffectReplayArtifact,
    {
        if let Some(recorded) = evidence.replay_verifier_id() {
            verify_replay_verifier(request.replay_verifier_id.as_ref(), recorded)?;
        }
        if evidence.evidence_schema_id() != &request.evidence_schema_id
            || evidence.evidence_hash() != &request.evidence_hash
        {
            return Err(side_effect_mismatch(evidence.mismatch_message()));
        }
        self.verify_side_effect_artifact(evidence)
    }

    pub(super) fn verify_side_effect_artifact<T>(
        &self,
        evidence: &T,
    ) -> Result<StoredArtifactEvidenceRef>
    where
        T: SideEffectReplayArtifact,
    {
        self.verify_artifact(ArtifactEvidenceExpectation {
            artifact_id: evidence.artifact_id(),
            evidence_hash: evidence.artifact_evidence_hash(),
            digest: evidence.evidence_hash(),
            schema_id: Some(evidence.evidence_schema_id()),
            semantic_type_id: None,
            role: evidence.artifact_role(),
            producer_node_id: Some(evidence.producer_node_id()),
            producer_seed_id: None,
        })
    }

    pub(super) fn verify_prepared_invocation_artifact(
        &self,
        prepared: &side_effect::InvocationPrepared,
    ) -> Result<PreparedInvocationReplayEvidence> {
        let artifact = self.verify_artifact(ArtifactEvidenceExpectation {
            artifact_id: &prepared.prepared_artifact_id,
            evidence_hash: &prepared.prepared_artifact_evidence_hash,
            digest: &prepared.prepared_hash,
            schema_id: Some(&prepared.prepared_schema_id),
            semantic_type_id: None,
            role: ArtifactRole::PreparedInvocation,
            producer_node_id: Some(&prepared.node_id),
            producer_seed_id: None,
        })?;
        Ok(PreparedInvocationReplayEvidence {
            prepared: prepared.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        })
    }

    pub(super) fn side_effect_submission_for(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<Option<SubmissionReplayEvidence>> {
        let Some(submission) = self.side_effect_records_for_request(request)?.submission else {
            return Ok(None);
        };
        let artifact = self.verify_side_effect_artifact(submission)?;
        Ok(Some(SubmissionReplayEvidence {
            submission: submission.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        }))
    }

    pub(super) fn side_effect_prepared_invocation_for(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<Option<PreparedInvocationReplayEvidence>> {
        let Some(prepared) = self.side_effect_records_for_request(request)?.prepared else {
            return Ok(None);
        };
        self.verify_prepared_invocation_artifact(prepared).map(Some)
    }

    pub(super) fn side_effect_receipt_for(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<Option<ReceiptReplayEvidence>> {
        let Some(receipt) = self.side_effect_records_for_request(request)?.receipt else {
            return Ok(None);
        };
        let artifact = self.verify_side_effect_artifact(receipt)?;
        Ok(Some(ReceiptReplayEvidence {
            receipt: receipt.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        }))
    }

    pub(super) fn side_effect_verification_for_pair(
        &self,
        pair_id: &SideEffectPairId,
    ) -> Result<spec::SideEffectVerificationSpec> {
        let pair = self
            .certified_spec()
            .spec
            .side_effect_verify_pair_for_pair_id(pair_id)
            .map_err(certified_spec_error)?;
        Ok(pair.submit_contract.verification.clone())
    }

    pub(super) fn certified_side_effect_context_for_pair(
        &self,
        pair_id: &SideEffectPairId,
    ) -> Result<CertifiedSideEffectContext> {
        let pair = self
            .certified_spec()
            .spec
            .side_effect_verify_pair_for_pair_id(pair_id)
            .map_err(certified_spec_error)?;
        let output_cell = self.cell(pair.submit_output_cell)?;
        Ok(CertifiedSideEffectContext {
            node_context: pair.submit_node.context.clone(),
            output_context: output_cell.context.clone(),
        })
    }

    pub(super) fn verify_side_effect_intent(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<()> {
        let intent = self.side_effect_intent(&request.pair_id)?;
        if intent.invocation_epoch != request.invocation_epoch {
            return Err(side_effect_mismatch(
                "side-effect intent does not match replay request",
            ));
        }
        let certified_context = self.certified_side_effect_context_for_pair(&request.pair_id)?;
        if request.certified_context != certified_context {
            return Err(side_effect_mismatch(
                "side-effect replay request does not match certified node context",
            ));
        }
        Ok(())
    }
}
