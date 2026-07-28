use super::*;

impl ReplayBroker<'_> {
    /// Returns the certified spec used as replay authority.
    pub fn certified_spec(&self) -> &HashedSpecEnvelope {
        store::current_lifecycle::read(self.view)
            .certified_spec()
            .envelope()
    }

    /// Returns the store-owned admitted-root reader bound to this replay broker.
    pub fn admission(&self) -> Result<store::current_lifecycle::CurrentAdmissionRef<'_>> {
        store::current_lifecycle::read(self.view)
            .admission()
            .map_err(ReplayError::from)
    }

    /// Returns retained artifact evidence for a direct artifact replay request.
    pub fn artifact(&self, request: &ArtifactReplayRequest) -> Result<StoredArtifactEvidenceRef> {
        self.verify_artifact(ArtifactEvidenceExpectation {
            artifact_id: &request.artifact_id,
            evidence_hash: &request.evidence_hash,
            digest: &request.digest,
            schema_id: request.schema_id.as_ref(),
            semantic_type_id: request.semantic_type_id.as_ref(),
            role: request.role,
            producer_node_id: request.producer_node_id.as_ref(),
            producer_seed_id: request.producer_seed_id.as_ref(),
        })
    }

    /// Returns retained artifact evidence and bytes for an explicit event requirement.
    pub fn retained_artifact(
        &self,
        requirement: &store::EventArtifactRequirement,
    ) -> Result<ArtifactReplayEvidence> {
        let object = self.object_for_requirement(requirement)?;
        Ok(ArtifactReplayEvidence {
            artifact_bytes: object.bytes().to_vec(),
            artifact: object.evidence().clone(),
        })
    }

    /// Returns retained produced-cell frames whose certified node/cell/event match a predicate.
    pub fn produced_cell_frames_matching<F>(
        &self,
        mut matches_cell: F,
    ) -> Result<Vec<ProducedCellReplayFrame>>
    where
        F: FnMut(&spec::NodeSpec, &spec::CellSpec, &events::CellProduced) -> Result<bool>,
    {
        let mut frames = Vec::new();
        for record in self.produced_cell_records()? {
            let produced = record.payload;
            let node = self.node(&produced.node_id)?;
            let cell = self.cell(&produced.cell_id)?;
            if !matches_cell(node, cell, produced)? {
                continue;
            }
            let requirement = store::EventArtifactRequirement {
                source: store::EventArtifactReferenceSource::StateOutput,
                artifact_id: produced.artifact_id.clone(),
                evidence_hash: produced.evidence_hash.clone(),
                digest: Some(produced.content_digest.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(produced.schema_id.clone()),
                semantic_type_id: Some(produced.semantic_type_id.clone()),
                producer_node_id: Some(produced.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::StateOutput),
            };
            let artifact = self.object_for_requirement(&requirement)?;
            frames.push(ProducedCellReplayFrame {
                node: node.clone(),
                cell: cell.clone(),
                produced: produced.clone(),
                artifact_bytes: artifact.bytes().to_vec(),
                artifact: artifact.evidence().clone(),
            });
        }
        Ok(frames)
    }

    /// Returns side-effect replay frames whose intent matches a domain predicate.
    ///
    /// This exposes broker-validated recorded evidence only. Domain verifiers remain
    /// responsible for deciding how many frames are valid and which phases are required.
    pub fn side_effect_replay_frames_matching<F>(
        &self,
        mut matches_intent: F,
    ) -> Result<Vec<SideEffectReplayFrame<'_>>>
    where
        F: FnMut(&side_effect::IntentPersisted) -> Result<bool>,
    {
        let mut frames = Vec::new();
        for intent in self.side_effect_intents()? {
            if !matches_intent(intent)? {
                continue;
            }
            let records =
                self.side_effect_records_for_pair(&intent.pair_id, intent.invocation_epoch)?;
            let certified_context = self.certified_side_effect_context_for_pair(&intent.pair_id)?;
            frames.push(SideEffectReplayFrame {
                intent,
                certified_context,
                prepared: records.prepared,
                submission: records.submission,
                submission_unknown: records.submission_unknown,
                not_submitted: records.not_submitted,
                receipt: records.receipt,
                confirmation: records.confirmation,
                ambiguity: records.ambiguity,
            });
        }
        Ok(frames)
    }

    /// Returns observed side-effect submission evidence from replay records only.
    pub fn side_effect_submission(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<SubmissionReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let submission = self
            .side_effect_records_for_request(request)?
            .submission
            .ok_or_else(|| missing_side_effect_record(request, "submission"))?;
        let artifact = self.verify_requested_side_effect_artifact(request, submission)?;
        Ok(SubmissionReplayEvidence {
            submission: submission.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        })
    }

    /// Returns retained submission-unknown evidence from replay records only.
    pub fn side_effect_submission_unknown(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<SubmissionUnknownReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let unknown = self
            .side_effect_records_for_request(request)?
            .submission_unknown
            .ok_or_else(|| missing_side_effect_record(request, "submission-unknown evidence"))?;
        let artifact = self.verify_requested_side_effect_artifact(request, unknown)?;
        Ok(SubmissionUnknownReplayEvidence {
            unknown: unknown.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        })
    }

    /// Returns prepared side-effect invocation evidence from replay records only.
    pub fn side_effect_prepared_invocation(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<PreparedInvocationReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let prepared = self
            .side_effect_records_for_request(request)?
            .prepared
            .ok_or_else(|| missing_side_effect_record(request, "prepared invocation"))?;
        self.verify_prepared_invocation_artifact(prepared)
    }

    /// Returns side-effect not-submitted proof evidence from replay records only.
    pub fn side_effect_not_submitted(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<NotSubmittedReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let proof = self
            .side_effect_records_for_request(request)?
            .not_submitted
            .ok_or_else(|| missing_side_effect_record(request, "not-submitted proof"))?;
        Ok(NotSubmittedReplayEvidence {
            proof: proof.clone(),
            artifact: self.verify_requested_side_effect_artifact(request, proof)?,
        })
    }

    /// Verifies submission evidence with an evidence-only replay verifier contract.
    pub fn verify_side_effect_submission<V>(
        &self,
        request: &SideEffectEvidenceReplayRequest,
        verifier: &V,
    ) -> Result<SubmissionReplayEvidence>
    where
        V: SideEffectReplayVerifier + ?Sized,
    {
        let submission = self.side_effect_submission(request)?;
        let input = SideEffectSubmissionReplayInput {
            intent: self.side_effect_intent_evidence(request)?,
            certified_context: request.certified_context.clone(),
            prepared_invocation: self.side_effect_prepared_invocation_for(request)?,
            submission: submission.clone(),
        };
        verifier.verify_submission(&input)?;
        Ok(submission)
    }

    /// Returns observed side-effect receipt evidence from replay records only.
    pub fn side_effect_receipt(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<ReceiptReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let receipt = self
            .side_effect_records_for_request(request)?
            .receipt
            .ok_or_else(|| missing_side_effect_record(request, "receipt"))?;
        let artifact = self.verify_requested_side_effect_artifact(request, receipt)?;
        Ok(ReceiptReplayEvidence {
            receipt: receipt.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        })
    }

    /// Returns observed side-effect confirmation evidence from replay records only.
    pub fn side_effect_confirmation(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<ConfirmationReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let confirmation = self
            .side_effect_records_for_request(request)?
            .confirmation
            .ok_or_else(|| missing_side_effect_record(request, "confirmation"))?;
        let artifact = self.verify_requested_side_effect_artifact(request, confirmation)?;
        Ok(ConfirmationReplayEvidence {
            confirmation: confirmation.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        })
    }

    /// Returns side-effect ambiguity evidence from replay records only.
    pub fn side_effect_ambiguity(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<AmbiguityReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let ambiguity = self
            .side_effect_records_for_request(request)?
            .ambiguity
            .ok_or_else(|| missing_side_effect_record(request, "ambiguity"))?;
        let artifact = self.verify_requested_side_effect_artifact(request, ambiguity)?;
        Ok(AmbiguityReplayEvidence {
            ambiguity: ambiguity.clone(),
            artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
            artifact,
        })
    }

    /// Verifies receipt evidence with an evidence-only replay verifier contract.
    pub fn verify_side_effect_receipt<V>(
        &self,
        request: &SideEffectEvidenceReplayRequest,
        verifier: &V,
    ) -> Result<ReceiptReplayEvidence>
    where
        V: SideEffectReplayVerifier + ?Sized,
    {
        let mut verifier_request = request.clone();
        verifier_request.replay_verifier_id = Some(verifier.verifier_id().clone());
        let receipt = self.side_effect_receipt(&verifier_request)?;
        let input = SideEffectReceiptReplayInput {
            intent: self.side_effect_intent_evidence(request)?,
            certified_context: request.certified_context.clone(),
            prepared_invocation: self.side_effect_prepared_invocation_for(request)?,
            submission: self.side_effect_submission_for(request)?,
            receipt: receipt.clone(),
        };
        verifier.verify_receipt(&input)?;
        Ok(receipt)
    }

    /// Verifies confirmation evidence with an evidence-only replay verifier contract.
    pub fn verify_side_effect_confirmation<V>(
        &self,
        request: &SideEffectEvidenceReplayRequest,
        verifier: &V,
    ) -> Result<ConfirmationReplayEvidence>
    where
        V: SideEffectReplayVerifier + ?Sized,
    {
        let mut verifier_request = request.clone();
        verifier_request.replay_verifier_id = Some(verifier.verifier_id().clone());
        let confirmation = self.side_effect_confirmation(&verifier_request)?;
        let input = SideEffectConfirmationReplayInput {
            intent: self.side_effect_intent_evidence(request)?,
            certified_context: request.certified_context.clone(),
            prepared_invocation: self.side_effect_prepared_invocation_for(request)?,
            submission: self.side_effect_submission_for(request)?,
            receipt: self.side_effect_receipt_for(request)?,
            verification: self.side_effect_verification_for_pair(&request.pair_id)?,
            confirmation: confirmation.clone(),
        };
        verifier.verify_confirmation(&input)?;
        Ok(confirmation)
    }

    /// Rejects any attempt to obtain live capability access during replay.
    pub fn reject_live_capability_request(&self) -> Result<()> {
        Err(ReplayError::new(
            ReplayErrorKind::LiveCapabilityRequest,
            "typed replay brokers do not construct live capabilities",
        ))
    }
}

fn missing_side_effect_record(
    request: &SideEffectEvidenceReplayRequest,
    label: &'static str,
) -> ReplayError {
    ReplayError::new(
        ReplayErrorKind::SideEffectMissing,
        format!("missing side-effect {label} {}", request.pair_id),
    )
}
