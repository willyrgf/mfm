use super::*;

impl ReplayBroker {
    pub(super) fn verify_fact_query_evidence_reference(
        &mut self,
        payload: &events::ArtifactReferenced,
    ) -> Result<()> {
        let bytes = self.artifact_bytes_by_key(&(
            payload.artifact_ref.artifact_id.clone(),
            payload.artifact_ref.evidence_hash.clone(),
        ))?;
        let evidence =
            mfm_facts::parse_canonical_fact_query_evidence_bytes(bytes).map_err(|error| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    format!("fact query evidence artifact is invalid: {error}"),
                )
            })?;
        mfm_facts::validate_fact_query_evidence(&evidence).map_err(|error| {
            ReplayError::new(
                ReplayErrorKind::CertifiedEvidenceMismatch,
                format!("fact query evidence is structurally invalid: {error}"),
            )
        })?;
        let plan = evidence.plan();
        self.verify_fact_query_descriptor_resolution(plan.resolved_descriptor())?;
        for fact_ref in evidence.receipt().returned_refs() {
            self.verify_fact_query_returned_ref(fact_ref, plan)?;
        }
        Ok(())
    }

    fn verify_fact_query_descriptor_resolution(
        &mut self,
        descriptor_hash: &ContentDigest,
    ) -> Result<()> {
        let mut candidates = BTreeMap::new();
        if let Some(descriptor) = self.projection.fact_descriptor(descriptor_hash) {
            let requirement = store::EventArtifactRequirement {
                source: store::EventArtifactReferenceSource::FactDescriptor,
                artifact_id: descriptor.descriptor_artifact_id.clone(),
                evidence_hash: descriptor
                    .descriptor_artifact_evidence
                    .evidence_hash()
                    .map_err(ReplayError::from)?,
                digest: Some(descriptor_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(mfm_facts::fact_descriptor_schema_id().map_err(|error| {
                    ReplayError::new(
                        ReplayErrorKind::CertifiedEvidenceMismatch,
                        error.to_string(),
                    )
                })?),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::FactDescriptor),
            };
            let evidence =
                exact_retained_artifact_for_requirement(&self.retained_artifacts, &requirement)?;
            insert_fact_descriptor_candidate(&mut candidates, evidence)?;
        }
        for envelope in &self.stream {
            let KernelEventPayload::RetentionRefsAppended(payload) = envelope.payload() else {
                continue;
            };
            for reference in &payload.refs {
                if reference.role != ArtifactRole::FactDescriptor
                    || reference.content_digest != *descriptor_hash
                {
                    continue;
                }
                let requirement = store::EventArtifactRequirement {
                    source: store::EventArtifactReferenceSource::RetentionRef,
                    artifact_id: reference.artifact_id.clone(),
                    evidence_hash: reference.evidence_hash.clone(),
                    digest: Some(reference.content_digest.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: None,
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::FactDescriptor),
                };
                let evidence = exact_retained_artifact_for_requirement(
                    &self.retained_artifacts,
                    &requirement,
                )?;
                insert_fact_descriptor_candidate(&mut candidates, evidence)?;
            }
        }
        let evidence = match candidates.len() {
            0 => {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!(
                        "missing exact retained fact descriptor artifact for {descriptor_hash}"
                    ),
                ));
            }
            1 => candidates
                .into_values()
                .next()
                .expect("one descriptor candidate"),
            _ => {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!("fact descriptor {descriptor_hash} has conflicting retained evidence"),
                ));
            }
        };
        let key = replay_artifact_authority_key(&evidence)?;
        let bytes = self.artifact_bytes_by_key(&key)?;
        let parsed = mfm_facts::parse_canonical_fact_descriptor_bytes(bytes).map_err(|error| {
            ReplayError::new(
                ReplayErrorKind::CertifiedEvidenceMismatch,
                format!("fact descriptor artifact is invalid: {error}"),
            )
        })?;
        let parsed_hash = mfm_facts::fact_descriptor_hash(&parsed).map_err(|error| {
            ReplayError::new(
                ReplayErrorKind::CertifiedEvidenceMismatch,
                format!("fact descriptor artifact hash is invalid: {error}"),
            )
        })?;
        if &parsed_hash != descriptor_hash {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!("fact descriptor artifact does not match {descriptor_hash}"),
            ));
        }
        self.insert_authorized_artifact(evidence)
    }

    fn verify_fact_query_returned_ref(
        &self,
        fact_ref: &mfm_facts::InternalFactRef,
        plan: &mfm_facts::CanonicalFactQueryPlan,
    ) -> Result<()> {
        if fact_ref.fact_descriptor_hash() != plan.resolved_descriptor() {
            return Err(ReplayError::new(
                ReplayErrorKind::FactMismatch,
                "returned fact ref does not match the query descriptor",
            ));
        }
        let (envelope, fact) = self.fact_event_for_claim(fact_ref.fact_claim_id())?;
        let claim = &fact.claim;
        let response = claim.response();
        let source_ref = mfm_facts::InternalFactRef::from_claim(
            fact_ref.fact_claim_id().clone(),
            envelope.event_id().clone(),
            fact_ref.recorded_at().to_owned(),
            fact.node_id.clone(),
            claim,
        )
        .map_err(|error| ReplayError::new(ReplayErrorKind::FactMismatch, error.to_string()))?;
        if &source_ref != fact_ref {
            return Err(ReplayError::new(
                ReplayErrorKind::FactMismatch,
                format!(
                    "returned fact ref {}:{}:{} does not match retained source fact",
                    fact_ref.fact_claim_id().source_run_id(),
                    fact_ref.fact_claim_id().source_seq(),
                    fact_ref.fact_claim_id().source_ordinal()
                ),
            ));
        }
        let artifact = self.verify_artifact(ArtifactEvidenceExpectation {
            artifact_id: response.artifact_id(),
            evidence_hash: response.artifact_evidence_hash(),
            digest: response.response_hash(),
            schema_id: Some(response.response_schema_id()),
            semantic_type_id: None,
            role: ArtifactRole::FactResponse,
            producer_node_id: Some(&fact.node_id),
            producer_seed_id: None,
        })?;
        let evidence_hash = artifact.evidence_hash().map_err(|error| {
            ReplayError::new(ReplayErrorKind::ArtifactMismatch, error.to_string())
        })?;
        if &evidence_hash != response.artifact_evidence_hash() {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                "returned fact response artifact evidence hash does not match source fact",
            ));
        }
        Ok(())
    }

    fn fact_event_for_claim(
        &self,
        fact_claim_id: &mfm_facts::FactClaimId,
    ) -> Result<(&KernelEventEnvelope, &events::FactRecorded)> {
        let fact = self.facts.get(fact_claim_id).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::FactMissing,
                format!(
                    "missing source fact for returned ref {}:{}:{}",
                    fact_claim_id.source_run_id(),
                    fact_claim_id.source_seq(),
                    fact_claim_id.source_ordinal()
                ),
            )
        })?;
        let envelope = self.fact_events.get(fact_claim_id).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::FactMissing,
                format!(
                    "missing source fact event for returned ref {}:{}:{}",
                    fact_claim_id.source_run_id(),
                    fact_claim_id.source_seq(),
                    fact_claim_id.source_ordinal()
                ),
            )
        })?;
        Ok((envelope, fact))
    }
}
