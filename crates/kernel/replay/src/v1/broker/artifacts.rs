use super::*;

pub(super) enum ReplayObjectRef<'a> {
    Primary(store::current_lifecycle::CurrentObjectRef<'a>),
    Additional(&'a store::VerifiedRetainedArtifactBytes),
}

impl<'a> ReplayObjectRef<'a> {
    pub(super) fn bytes(&self) -> &'a [u8] {
        match self {
            Self::Primary(object) => object.bytes(),
            Self::Additional(object) => object.bytes(),
        }
    }

    pub(super) fn evidence(&self) -> &'a StoredArtifactEvidenceRef {
        match self {
            Self::Primary(object) => object.evidence(),
            Self::Additional(object) => object.evidence(),
        }
    }
}

impl ReplayBroker<'_> {
    pub(super) fn object_for_requirement(
        &self,
        requirement: &store::EventArtifactRequirement,
    ) -> Result<ReplayObjectRef<'_>> {
        if let Some(object) =
            store::current_lifecycle::read(self.view).object_for_requirement(requirement)
        {
            return Ok(ReplayObjectRef::Primary(object));
        }
        for object in &self.additional_artifacts {
            if object.evidence().artifact_id != requirement.artifact_id {
                continue;
            }
            let evidence_hash = object
                .evidence()
                .evidence_hash()
                .map_err(ReplayError::from)?;
            if evidence_hash != requirement.evidence_hash {
                continue;
            }
            store::validate_artifact_requirement_against_evidence(requirement, object.evidence())
                .map_err(|error| {
                artifact_requirement_replay_error(error, "artifact evidence mismatch")
            })?;
            return Ok(ReplayObjectRef::Additional(object));
        }
        Err(ReplayError::new(
            ReplayErrorKind::ArtifactMissing,
            format!(
                "missing replay-authorized artifact evidence for {}",
                requirement.artifact_id
            ),
        ))
    }

    fn object_for_expectation(
        &self,
        expected: ArtifactEvidenceExpectation<'_>,
    ) -> Result<ReplayObjectRef<'_>> {
        let lifecycle = store::current_lifecycle::read(self.view);
        let mut exact_requirement = None;
        let _ = lifecycle.visit_records(|record| {
            record.visit_artifact_requirements(|requirement| {
                if requirement.artifact_id != *expected.artifact_id
                    || requirement.evidence_hash != *expected.evidence_hash
                {
                    return std::ops::ControlFlow::Continue(());
                }
                exact_requirement = Some(requirement.clone());
                std::ops::ControlFlow::Break(())
            })
        });
        if let Some(requirement) = exact_requirement {
            let object = lifecycle
                .object_for_requirement(&requirement)
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::InvalidRunJournal,
                        format!(
                            "committed artifact requirement for {} has no exact object",
                            expected.artifact_id
                        ),
                    )
                })?;
            let object = ReplayObjectRef::Primary(object);
            verify_artifact_expectation(object.evidence(), expected)?;
            return Ok(object);
        }
        for object in &self.additional_artifacts {
            if object.evidence().artifact_id != *expected.artifact_id {
                continue;
            }
            if &object
                .evidence()
                .evidence_hash()
                .map_err(ReplayError::from)?
                != expected.evidence_hash
            {
                continue;
            }
            let object = ReplayObjectRef::Additional(object);
            verify_artifact_expectation(object.evidence(), expected)?;
            return Ok(object);
        }
        Err(ReplayError::new(
            ReplayErrorKind::ArtifactMissing,
            format!(
                "missing replay-authorized artifact evidence for {}",
                expected.artifact_id
            ),
        ))
    }

    pub(super) fn verify_artifact(
        &self,
        expected: ArtifactEvidenceExpectation<'_>,
    ) -> Result<StoredArtifactEvidenceRef> {
        let object = self.object_for_expectation(expected)?;
        Ok(object.evidence().clone())
    }

    pub(super) fn artifact_bytes_for_evidence(
        &self,
        evidence: &StoredArtifactEvidenceRef,
    ) -> Result<&[u8]> {
        let evidence_hash = evidence.evidence_hash().map_err(ReplayError::from)?;
        let object = self.object_for_expectation(ArtifactEvidenceExpectation {
            artifact_id: &evidence.artifact_id,
            evidence_hash: &evidence_hash,
            digest: &evidence.digest,
            schema_id: evidence.schema_id.as_ref(),
            semantic_type_id: evidence.semantic_type_id.as_ref(),
            role: evidence.artifact_role,
            producer_node_id: evidence.producer_node_id.as_ref(),
            producer_seed_id: evidence.producer_seed_id.as_ref(),
        })?;
        if object.evidence() != evidence {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!(
                    "retained artifact evidence changed for {}",
                    evidence.artifact_id
                ),
            ));
        }
        Ok(object.bytes())
    }
}
