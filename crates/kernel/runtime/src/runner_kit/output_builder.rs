use super::builder_helpers::{
    artifact_schema_id, ensure_fact_descriptor_matches_type, ensure_node_allows_fact_descriptor,
    insert_retention_ref, runtime_fact_error,
};
use super::runner_artifact_builder::RunnerArtifactBuilder;
use super::runner_payload_builder::{RunnerPayloadBuilder, RunnerSideEffectBinding};
use super::*;

/// Builder for assembling an erased runner output batch.
pub struct RunnerOutputBuilder<'a, 'ctx> {
    artifacts: RunnerArtifactBuilder<'a, 'ctx>,
    staged_artifacts: Vec<StagedArtifact>,
    staged_retention_refs: Vec<StagedRetentionRefs>,
    read_facts: Vec<events::FactRecorded>,
    payloads: Vec<RunnerEventPayload>,
}

impl<'a, 'ctx> RunnerOutputBuilder<'a, 'ctx> {
    /// Creates an empty output builder bound to one runner invocation context.
    pub fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self {
            artifacts: RunnerArtifactBuilder::new(ctx),
            staged_artifacts: Vec::new(),
            staged_retention_refs: Vec::new(),
            read_facts: Vec::new(),
            payloads: Vec::new(),
        }
    }

    /// Stages an attempt artifact.
    pub fn stage_attempt_artifact(&mut self, artifact: &RunnerJsonArtifact) -> Result<&mut Self> {
        self.staged_artifacts
            .push(self.artifacts.staged_attempt(artifact)?);
        Ok(self)
    }

    /// Stages a side-effect artifact.
    pub(crate) fn stage_side_effect_artifact(
        &mut self,
        artifact: &RunnerJsonArtifact,
        ledger_key: events::SideEffectLedgerKey,
        invocation_epoch: u32,
    ) -> Result<&mut Self> {
        self.staged_artifacts
            .push(
                self.artifacts
                    .staged_side_effect(artifact, ledger_key, invocation_epoch)?,
            );
        Ok(self)
    }

    /// Stages a side-effect artifact and retains it as runtime evidence.
    pub(crate) fn stage_side_effect_runtime_evidence(
        &mut self,
        artifact: &RunnerJsonArtifact,
        side_effect: &RunnerSideEffectBinding,
    ) -> Result<&mut Self> {
        self.stage_side_effect_artifact(
            artifact,
            side_effect.ledger_key.clone(),
            side_effect.invocation_epoch,
        )?;
        self.retain_runtime_evidence(artifact)?;
        Ok(self)
    }

    /// Appends runtime retention refs for one artifact.
    pub fn retain_runtime_evidence(&mut self, artifact: &RunnerJsonArtifact) -> Result<&mut Self> {
        self.staged_retention_refs
            .push(StagedRetentionRefs::runtime_evidence(vec![
                artifact.retention_ref()?
            ]));
        Ok(self)
    }

    /// Stages and retains request/response evidence for an external capability read.
    pub fn record_external_read_evidence<T>(&mut self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        let artifact = self.artifacts.external_read_evidence(value)?;
        self.stage_attempt_artifact(&artifact)?;
        self.retain_runtime_evidence(&artifact)?;
        Ok(artifact)
    }

    /// Stages a state-output artifact, retains it as runtime evidence, and emits its cell payload.
    pub fn state_output<T>(&mut self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        let payloads = RunnerPayloadBuilder::new(self.artifacts.ctx);
        let artifact = self.stage_state_output_artifact(value)?;
        self.payload(payloads.cell_produced(&artifact)?);
        Ok(artifact)
    }

    /// Stages a state-output artifact and private fact-query replay evidence.
    pub fn state_output_and_record_fact_query_evidence<T>(
        &mut self,
        value: &T,
        evidence: mfm_facts::FactQueryEvidence,
    ) -> Result<()>
    where
        T: MfmValue,
    {
        self.state_output(value)?;
        self.record_fact_query_evidence(evidence)
    }

    pub(in crate::runner_kit) fn record_read_fact<T>(&mut self, fact: &T) -> Result<()>
    where
        T: MfmFactType,
    {
        let descriptor = T::descriptor().map_err(runtime_fact_error)?;
        ensure_fact_descriptor_matches_type::<T>(&descriptor)?;
        let descriptor_hash =
            mfm_facts::fact_descriptor_hash(&descriptor).map_err(runtime_fact_error)?;
        ensure_node_allows_fact_descriptor(self.artifacts.ctx.node(), &descriptor_hash)?;

        let subject_json = serde_json::to_value(fact.subject())
            .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
        let subject = mfm_facts::typed_fact_subject_evidence(&descriptor, &subject_json)
            .map_err(runtime_fact_error)?;
        let response = self.artifacts.fact_response(fact.response())?;
        let response_schema_id = artifact_schema_id(&response)?;
        if descriptor.response_schema_id() != &response_schema_id {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "fact response schema {} did not match descriptor response schema {}",
                response_schema_id,
                descriptor.response_schema_id()
            )));
        }
        let response_evidence = response.evidence().clone();
        let claim = mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
            fact_kind: descriptor.fact_kind().clone(),
            fact_descriptor_hash: descriptor_hash,
            subject,
            response: mfm_facts::FactResponseEvidence::new(
                response_schema_id,
                response_evidence.digest.clone(),
                response_evidence.artifact_id.clone(),
                response_evidence.evidence_hash()?,
            ),
        })
        .map_err(runtime_fact_error)?;

        self.stage_attempt_artifact(&response)?;
        self.read_facts.push(events::FactRecorded {
            spec_hash: self.artifacts.ctx.spec_hash().clone(),
            node_id: self.artifacts.ctx.node().node_id.clone(),
            attempt_id: self.artifacts.ctx.attempt_id().clone(),
            claim,
        });

        Ok(())
    }

    /// Stages private replay evidence for a live fact query.
    ///
    /// The commit planner emits the existing generic `ArtifactReferenced` event for the staged
    /// artifact. The evidence is retained as runtime evidence and is not represented as a
    /// `FactRecorded` claim.
    pub fn record_fact_query_evidence(
        &mut self,
        evidence: mfm_facts::FactQueryEvidence,
    ) -> Result<()> {
        mfm_facts::validate_fact_query_evidence(&evidence).map_err(runtime_fact_error)?;
        let artifact = self.artifacts.fact_query_evidence(&evidence)?;
        let staged = self.artifacts.staged_attempt(&artifact)?;
        let retention_refs = fact_query_evidence_retention_refs(
            &artifact,
            &evidence,
            self.artifacts.ctx.projections(),
        )?;
        let returned_refs = evidence.receipt().returned_refs().to_vec();
        self.staged_artifacts.push(staged);
        self.staged_retention_refs
            .push(StagedRetentionRefs::fact_query_evidence(
                retention_refs,
                returned_refs,
            ));
        Ok(())
    }

    fn stage_state_output_artifact<T>(&mut self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        let artifact = self.artifacts.state_output(value)?;
        self.stage_attempt_artifact(&artifact)?;
        self.retain_runtime_evidence(&artifact)?;
        Ok(artifact)
    }

    /// Appends a runner-owned event payload.
    pub fn payload(&mut self, payload: RunnerEventPayload) -> &mut Self {
        self.payloads.push(payload);
        self
    }

    /// Finishes the builder into an erased runner output batch.
    pub fn finish(self) -> ErasedRunnerOutput {
        ErasedRunnerOutput::from_parts_with_read_facts(
            self.staged_artifacts,
            self.staged_retention_refs,
            self.read_facts,
            self.payloads,
        )
    }
}

impl ErasedRunnerOutput {
    /// Builds an erased runner output containing one state-output value.
    pub fn state_output<T>(ctx: &ErasedRunCtx<'_>, value: &T) -> Result<Self>
    where
        T: MfmValue,
    {
        let mut output = RunnerOutputBuilder::new(ctx);
        output.state_output(value)?;
        Ok(output.finish())
    }
}

pub(crate) fn fact_query_evidence_retention_refs(
    evidence_artifact: &RunnerJsonArtifact,
    evidence: &mfm_facts::FactQueryEvidence,
    projections: &store::ProjectionSnapshot,
) -> Result<Vec<events::RetentionRef>> {
    let mut refs = BTreeMap::<(ArtifactId, ContentDigest), events::RetentionRef>::new();
    insert_retention_ref(&mut refs, evidence_artifact.retention_ref()?);

    for fact_ref in evidence.receipt().returned_refs() {
        for retention_ref in fact_query_returned_ref_retention_refs(projections, fact_ref)? {
            insert_retention_ref(&mut refs, retention_ref);
        }
    }

    Ok(refs.into_values().collect())
}
