use super::*;

impl ReplayBroker {
    pub(super) fn authorize_event_artifacts(&mut self, payload: &KernelEventPayload) -> Result<()> {
        for requirement in payload.artifact_requirements() {
            if self.should_skip_event_artifact_requirement(&requirement)? {
                self.authorize_skipped_event_artifact_requirement(&requirement)?;
                continue;
            }
            self.authorize_event_artifact_requirement(&requirement)?;
        }
        Ok(())
    }

    fn should_skip_event_artifact_requirement(
        &self,
        requirement: &store::EventArtifactRequirement,
    ) -> Result<bool> {
        if requirement.source == store::EventArtifactReferenceSource::PublicOutputCell {
            return Ok(true);
        }
        if !requirement.source.is_terminal_lifecycle_receipt_candidate() {
            return Ok(false);
        }
        if requirement.artifact_role != Some(ArtifactRole::StateOutput) {
            return Ok(false);
        }
        let Some(node_id) = requirement.producer_node_id.as_ref() else {
            return Ok(false);
        };
        let node = self.node(node_id)?;
        if !is_terminal_lifecycle_node(node) {
            return Ok(false);
        }
        if requirement.source == store::EventArtifactReferenceSource::ArtifactReferenced {
            let cell = self.cell(&node.output_cell)?;
            if requirement.schema_id.as_ref() != Some(&cell.schema_id)
                || requirement.semantic_type_id.as_ref() != Some(&cell.semantic_type_id)
            {
                return Ok(false);
            }
        }
        let Some(digest) = requirement.digest.as_ref() else {
            return Ok(false);
        };
        self.is_terminal_lifecycle_receipt_artifact(
            node_id,
            &requirement.artifact_id,
            digest,
            &requirement.evidence_hash,
        )
    }

    fn authorize_skipped_event_artifact_requirement(
        &mut self,
        requirement: &store::EventArtifactRequirement,
    ) -> Result<StoredArtifactEvidenceRef> {
        self.validate_event_artifact_requirement(requirement, "skipped artifact evidence mismatch")
    }

    fn authorize_event_artifact_requirement(
        &mut self,
        requirement: &store::EventArtifactRequirement,
    ) -> Result<StoredArtifactEvidenceRef> {
        if requirement.digest.is_none() {
            return Err(ReplayError::new(
                ReplayErrorKind::InvalidRunStream,
                format!(
                    "artifact requirement for {} does not carry a digest",
                    requirement.artifact_id
                ),
            ));
        }
        if requirement.source == store::EventArtifactReferenceSource::RetentionRef
            && requirement.artifact_role.is_none()
        {
            return Err(ReplayError::new(
                ReplayErrorKind::InvalidRunStream,
                format!(
                    "retention requirement for {} does not carry an artifact role",
                    requirement.artifact_id
                ),
            ));
        }
        if requirement.artifact_role.is_none() && requirement.schema_id.is_none() {
            return Err(ReplayError::new(
                ReplayErrorKind::InvalidRunStream,
                format!(
                    "schema-only requirement for {} does not carry a schema id",
                    requirement.artifact_id
                ),
            ));
        }
        self.validate_event_artifact_requirement(requirement, "artifact evidence mismatch")
    }

    fn validate_event_artifact_requirement(
        &mut self,
        requirement: &store::EventArtifactRequirement,
        _mismatch_context: &'static str,
    ) -> Result<StoredArtifactEvidenceRef> {
        if requirement.digest.is_none() {
            return Err(ReplayError::new(
                ReplayErrorKind::InvalidRunStream,
                format!(
                    "artifact requirement for {} does not carry a digest",
                    requirement.artifact_id
                ),
            ));
        }
        let evidence =
            exact_retained_artifact_for_requirement(&self.retained_artifacts, requirement)?;
        let evidence = evidence.clone();
        self.insert_authorized_artifact(evidence.clone())?;
        Ok(evidence)
    }

    pub(super) fn authorize_artifact(
        &mut self,
        expected: ArtifactEvidenceExpectation<'_>,
    ) -> Result<StoredArtifactEvidenceRef> {
        let evidence = self
            .retained_artifacts
            .get(&(expected.artifact_id.clone(), expected.evidence_hash.clone()))
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!(
                        "missing retained artifact evidence for {}",
                        expected.artifact_id
                    ),
                )
            })?;
        verify_artifact_expectation(evidence, expected)?;
        let evidence = evidence.clone();
        self.insert_authorized_artifact(evidence.clone())?;
        Ok(evidence)
    }

    pub(super) fn insert_authorized_artifact(
        &mut self,
        evidence: StoredArtifactEvidenceRef,
    ) -> Result<()> {
        let key = replay_artifact_authority_key(&evidence)?;
        if let Some(existing) = self.artifacts.get(&key) {
            if existing != &evidence {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!(
                        "conflicting authorized artifact evidence for {}",
                        evidence.artifact_id
                    ),
                ));
            }
            return Ok(());
        }
        self.artifacts.insert(key, evidence);
        Ok(())
    }

    pub(super) fn verify_artifact(
        &self,
        expected: ArtifactEvidenceExpectation<'_>,
    ) -> Result<StoredArtifactEvidenceRef> {
        let evidence = self
            .artifacts
            .get(&(expected.artifact_id.clone(), expected.evidence_hash.clone()))
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!(
                        "missing replay-authorized artifact evidence for {}",
                        expected.artifact_id
                    ),
                )
            })?;
        verify_artifact_expectation(evidence, expected)?;
        Ok(evidence.clone())
    }

    pub(super) fn artifact_bytes_for_evidence(
        &self,
        evidence: &StoredArtifactEvidenceRef,
    ) -> Result<&[u8]> {
        let key = replay_artifact_authority_key(evidence)?;
        self.artifact_bytes_by_key(&key)
    }

    pub(super) fn artifact_bytes_by_key(&self, key: &ReplayArtifactAuthorityKey) -> Result<&[u8]> {
        self.artifact_bytes
            .get(key)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!("missing replay-authorized artifact bytes for {}", key.0),
                )
            })
    }
}
