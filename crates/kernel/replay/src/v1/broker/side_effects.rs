use super::*;

impl ReplayBroker {
    pub(super) fn side_effect_intent_evidence(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<SideEffectIntentReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let intent = self.intents.get(&request.pair_id).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::SideEffectMissing,
                format!("missing side-effect intent {}", request.pair_id),
            )
        })?;
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

    pub(super) fn required_side_effect_record<'a, T>(
        &'a self,
        records: &'a BTreeMap<SideEffectKey, T>,
        request: &SideEffectEvidenceReplayRequest,
        label: &'static str,
    ) -> Result<&'a T> {
        records
            .get(&(request.pair_id.clone(), request.invocation_epoch))
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::SideEffectMissing,
                    format!("missing side-effect {label} {}", request.pair_id),
                )
            })
    }

    pub(super) fn optional_side_effect_record<'a, T>(
        &'a self,
        records: &'a BTreeMap<SideEffectKey, T>,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Option<&'a T> {
        records.get(&(request.pair_id.clone(), request.invocation_epoch))
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
        let Some(submission) = self.optional_side_effect_record(&self.submissions, request) else {
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
        let Some(prepared) = self.optional_side_effect_record(&self.prepared_invocations, request)
        else {
            return Ok(None);
        };
        self.verify_prepared_invocation_artifact(prepared).map(Some)
    }

    pub(super) fn side_effect_receipt_for(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<Option<ReceiptReplayEvidence>> {
        let Some(receipt) = self.optional_side_effect_record(&self.receipts, request) else {
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
            .certified_spec
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
            .certified_spec
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
        let intent = self.intents.get(&request.pair_id).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::SideEffectMissing,
                format!("missing side-effect intent {}", request.pair_id),
            )
        })?;
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

    pub(super) fn verify_side_effect_event_against_intent(
        &self,
        pair_id: &SideEffectPairId,
        ledger_key: &events::SideEffectLedgerKey,
        invocation_epoch: u32,
        pair_role: events::SideEffectPairRole,
        node_id: &NodeId,
        attempt_id: &AttemptId,
    ) -> Result<()> {
        let intent = self.side_effect_intent_for_event(pair_id, ledger_key, invocation_epoch)?;
        match pair_role {
            events::SideEffectPairRole::Submit => {
                verify_side_effect_submit_claim_identity(
                    intent,
                    node_id,
                    attempt_id,
                    "side-effect event does not match persisted intent",
                )?;
            }
            events::SideEffectPairRole::Verify => {
                let (verify_node, verify) = self.side_effect_verify_node_for_pair(pair_id)?;
                if verify.submit_node_id != intent.node_id || verify_node.node_id != *node_id {
                    return Err(side_effect_mismatch(
                        "side-effect verify event does not match certified pair",
                    ));
                }
                self.node(node_id)?;
            }
        }
        Ok(())
    }

    pub(super) fn verify_side_effect_ambiguity_against_intent(
        &self,
        pair_id: &SideEffectPairId,
        ledger_key: &events::SideEffectLedgerKey,
        invocation_epoch: u32,
        pair_role: events::SideEffectPairRole,
        node_id: &NodeId,
        attempt_id: &AttemptId,
    ) -> Result<()> {
        self.verify_side_effect_event_against_intent(
            pair_id,
            ledger_key,
            invocation_epoch,
            pair_role,
            node_id,
            attempt_id,
        )
    }

    pub(super) fn side_effect_intent_for_event(
        &self,
        pair_id: &SideEffectPairId,
        ledger_key: &events::SideEffectLedgerKey,
        invocation_epoch: u32,
    ) -> Result<&side_effect::IntentPersisted> {
        let intent = self.intents.get(pair_id).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::SideEffectMissing,
                format!("missing side-effect intent {pair_id}"),
            )
        })?;
        if intent.ledger_key != *ledger_key
            || intent.pair_id != *pair_id
            || intent.invocation_epoch != invocation_epoch
        {
            return Err(side_effect_mismatch(
                "side-effect event does not match persisted intent",
            ));
        }
        Ok(intent)
    }

    pub(super) fn side_effect_contract_node_for_pair_event(
        &self,
        pair_id: &SideEffectPairId,
        pair_role: events::SideEffectPairRole,
        node_id: &NodeId,
    ) -> Result<&NodeId> {
        let intent = self.intents.get(pair_id).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::SideEffectMissing,
                format!("missing side-effect intent {pair_id}"),
            )
        })?;
        match pair_role {
            events::SideEffectPairRole::Submit => {
                if intent.node_id != *node_id {
                    return Err(side_effect_mismatch(
                        "side-effect event does not match persisted intent",
                    ));
                }
                Ok(&intent.node_id)
            }
            events::SideEffectPairRole::Verify => {
                let (verify_node, verify) = self.side_effect_verify_node_for_pair(pair_id)?;
                if verify.submit_node_id != intent.node_id || verify_node.node_id != *node_id {
                    return Err(side_effect_mismatch(
                        "side-effect verify event does not match certified pair",
                    ));
                }
                Ok(&intent.node_id)
            }
        }
    }

    pub(super) fn side_effect_verify_node_for_pair(
        &self,
        pair_id: &SideEffectPairId,
    ) -> Result<(&spec::NodeSpec, &spec::SideEffectVerifyNodeSpec)> {
        let pair = self
            .certified_spec
            .spec
            .side_effect_verify_pair_for_pair_id(pair_id)
            .map_err(certified_spec_error)?;
        Ok((pair.verify_node, pair.verify))
    }
}
