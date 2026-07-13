use super::*;

impl ReplayBroker {
    /// Returns the certified spec used as replay authority.
    pub fn certified_spec(&self) -> &HashedSpecEnvelope {
        &self.certified_spec
    }

    /// Returns the run-start payload bound to this replay broker.
    pub fn run_admitted(&self) -> &events::RunAdmitted {
        &self.run_id
    }

    /// Returns the broker-owned verified history events used for replay evidence.
    pub fn events(&self) -> &[KernelEventEnvelope] {
        &self.stream
    }

    /// Returns the projection rebuilt from the authoritative run stream.
    pub fn projection_snapshot(&self) -> &ProjectionSnapshot {
        &self.projection
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
        let evidence = self
            .artifacts
            .get(&(
                requirement.artifact_id.clone(),
                requirement.evidence_hash.clone(),
            ))
            .cloned()
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!(
                        "missing replay-authorized artifact evidence for {}",
                        requirement.artifact_id
                    ),
                )
            })?;
        store::validate_artifact_requirement_against_evidence(requirement, &evidence).map_err(
            |error| artifact_requirement_replay_error(error, "artifact evidence mismatch"),
        )?;
        let artifact_bytes = self.artifact_bytes_for_evidence(&evidence)?.to_vec();
        Ok(ArtifactReplayEvidence {
            artifact_bytes,
            artifact: evidence,
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
        for envelope in &self.stream {
            let KernelEventPayload::CellProduced(produced) = envelope.payload() else {
                continue;
            };
            let node = self.node(&produced.node_id)?;
            let cell = self.cell(&produced.cell_id)?;
            if !matches_cell(node, cell, produced)? {
                continue;
            }
            let artifact = self.verify_artifact(ArtifactEvidenceExpectation {
                artifact_id: &produced.artifact_id,
                evidence_hash: &produced.evidence_hash,
                digest: &produced.content_digest,
                schema_id: Some(&produced.schema_id),
                semantic_type_id: Some(&produced.semantic_type_id),
                role: ArtifactRole::StateOutput,
                producer_node_id: Some(&produced.node_id),
                producer_seed_id: None,
            })?;
            frames.push(ProducedCellReplayFrame {
                node: node.clone(),
                cell: cell.clone(),
                produced: produced.clone(),
                artifact_bytes: self.artifact_bytes_for_evidence(&artifact)?.to_vec(),
                artifact,
            });
        }
        Ok(frames)
    }

    /// Returns a recorded fact from replay evidence only.
    pub fn recorded_fact(&self, request: &FactReplayRequest) -> Result<RecordedFactReplay> {
        self.verify_node_capability(
            &request.node_id,
            &request.capability_kind,
            &request.capability_version,
        )?;
        self.verify_node_adapter(
            &request.node_id,
            &request.adapter_kind,
            &request.adapter_version,
        )?;
        let fact = self.facts.get(&request.fact_claim_id).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::FactMissing,
                format!(
                    "missing recorded fact claim {}:{}:{}",
                    request.fact_claim_id.source_run_id(),
                    request.fact_claim_id.source_seq(),
                    request.fact_claim_id.source_ordinal()
                ),
            )
        })?;
        let producer = fact.claim.producer();
        let fact_request = fact.claim.request();
        let response = fact.claim.response();
        if fact.node_id != request.node_id
            || fact.attempt_id != request.attempt_id
            || producer.capability_kind() != &request.capability_kind
            || producer.capability_version() != &request.capability_version
            || producer.adapter_kind() != &request.adapter_kind
            || producer.adapter_version() != &request.adapter_version
            || fact_request.is_none_or(|evidence| {
                evidence.request_schema_id() != &request.request_schema_id
                    || evidence.request_hash() != &request.request_hash
            })
            || response.response_schema_id() != &request.response_schema_id
        {
            return Err(ReplayError::new(
                ReplayErrorKind::FactMismatch,
                format!(
                    "recorded fact claim {}:{}:{} does not match replay request",
                    request.fact_claim_id.source_run_id(),
                    request.fact_claim_id.source_seq(),
                    request.fact_claim_id.source_ordinal()
                ),
            ));
        }

        Ok(RecordedFactReplay {
            fact_claim_id: request.fact_claim_id.clone(),
            fact: fact.clone(),
            artifact: self.verify_artifact(ArtifactEvidenceExpectation {
                artifact_id: response.artifact_id(),
                evidence_hash: response.artifact_evidence_hash(),
                digest: response.response_hash(),
                schema_id: Some(response.response_schema_id()),
                semantic_type_id: None,
                role: ArtifactRole::FactResponse,
                producer_node_id: Some(&fact.node_id),
                producer_seed_id: None,
            })?,
        })
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
        for intent in self.intents.values() {
            if !matches_intent(intent)? {
                continue;
            }
            let key = (intent.pair_id.clone(), intent.invocation_epoch);
            let certified_context = self.certified_side_effect_context_for_pair(&intent.pair_id)?;
            frames.push(SideEffectReplayFrame {
                intent,
                certified_context,
                prepared: self.prepared_invocations.get(&key),
                submission: self.submissions.get(&key),
                not_submitted: self.not_submitted.get(&key),
                receipt: self.receipts.get(&key),
                confirmation: self.confirmations.get(&key),
                ambiguity: self.ambiguities.get(&key),
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
        let submission =
            self.required_side_effect_record(&self.submissions, request, "submission")?;
        let artifact = self.verify_requested_side_effect_artifact(request, submission)?;
        Ok(SubmissionReplayEvidence {
            submission: submission.clone(),
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
        let prepared = self.required_side_effect_record(
            &self.prepared_invocations,
            request,
            "prepared invocation",
        )?;
        self.verify_prepared_invocation_artifact(prepared)
    }

    /// Returns side-effect not-submitted proof evidence from replay records only.
    pub fn side_effect_not_submitted(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<NotSubmittedReplayEvidence> {
        self.verify_side_effect_intent(request)?;
        let proof =
            self.required_side_effect_record(&self.not_submitted, request, "not-submitted proof")?;
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
        let receipt = self.required_side_effect_record(&self.receipts, request, "receipt")?;
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
        let confirmation =
            self.required_side_effect_record(&self.confirmations, request, "confirmation")?;
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
        let ambiguity =
            self.required_side_effect_record(&self.ambiguities, request, "ambiguity")?;
        Ok(AmbiguityReplayEvidence {
            ambiguity: ambiguity.clone(),
            artifact: self.verify_requested_side_effect_artifact(request, ambiguity)?,
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
