use super::*;

impl ReplayBroker<'_> {
    pub(crate) fn verify_fact_query_evidence_reference(
        &self,
        payload: &events::ArtifactReferenced,
    ) -> Result<mfm_facts::FactQueryEvidence> {
        let requirement = store::artifact_referenced_artifact_requirement(payload);
        let object = self.object_for_requirement(&requirement)?;
        let bytes = object.bytes();
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
        Ok(evidence)
    }

    fn verify_fact_query_descriptor_resolution(
        &self,
        descriptor_hash: &ContentDigest,
    ) -> Result<()> {
        let mut candidates = Vec::new();
        for descriptor in self.admission()?.fact_descriptor_artifacts() {
            if descriptor.content_digest != *descriptor_hash {
                continue;
            }
            let requirement = store::run_artifact_requirement(
                store::EventArtifactReferenceSource::FactDescriptor,
                descriptor,
                ArtifactRole::FactDescriptor,
            );
            let object = self.object_for_requirement(&requirement)?;
            insert_fact_descriptor_candidate(&mut candidates, object.evidence(), object.bytes())?;
        }
        for reference in self.retention_references()? {
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
            let object = self.object_for_requirement(&requirement)?;
            insert_fact_descriptor_candidate(&mut candidates, object.evidence(), object.bytes())?;
        }
        let (_evidence, bytes) = match candidates.len() {
            0 => {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!(
                        "missing exact retained fact descriptor artifact for {descriptor_hash}"
                    ),
                ));
            }
            1 => candidates.pop().ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!(
                        "missing exact retained fact descriptor artifact for {descriptor_hash}"
                    ),
                )
            })?,
            _ => {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!("fact descriptor {descriptor_hash} has conflicting retained evidence"),
                ));
            }
        };
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
        Ok(())
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
        let record = self.fact_event_for_claim(fact_ref.fact_claim_id())?;
        let fact = record.payload();
        let claim = &fact.claim;
        let response = claim.response();
        let source_ref = mfm_facts::InternalFactRef::from_claim(
            fact_ref.fact_claim_id().clone(),
            record.event_id().clone(),
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
        let requirement = store::fact_response_artifact_requirement(fact_ref);
        let artifact = self.object_for_requirement(&requirement)?;
        let evidence_hash = artifact.evidence().evidence_hash().map_err(|error| {
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
    ) -> Result<ReplayFactEventRef<'_>> {
        self.fact_record_for_claim(fact_claim_id)?.ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::FactMissing,
                format!(
                    "missing source fact event for returned ref {}:{}:{}",
                    fact_claim_id.source_run_id(),
                    fact_claim_id.source_seq(),
                    fact_claim_id.source_ordinal()
                ),
            )
        })
    }
}
