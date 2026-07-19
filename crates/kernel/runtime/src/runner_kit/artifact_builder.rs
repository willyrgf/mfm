use super::builder_helpers::{canonical_json, runtime_fact_error, runtime_value_error};
use super::*;

/// Builder for runner-owned JSON artifacts and staging handles.
pub struct RunnerArtifactBuilder<'a, 'ctx> {
    pub(super) ctx: &'a ErasedRunCtx<'ctx>,
}

impl<'a, 'ctx> RunnerArtifactBuilder<'a, 'ctx> {
    /// Creates a builder bound to one runner invocation context.
    pub fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self { ctx }
    }

    /// Canonicalizes a serializable value as plain JSON bytes.
    pub fn canonical_bytes<T>(&self, value: &T) -> Result<PlainCanonicalJsonBytes>
    where
        T: Serialize,
    {
        canonical_json(value)
    }

    /// Computes the canonical content digest for a serializable value.
    pub fn content_digest<T>(&self, value: &T) -> Result<ContentDigest>
    where
        T: Serialize,
    {
        Ok(self.canonical_bytes(value)?.content_digest())
    }

    /// Builds a state-output artifact using the value type metadata.
    pub fn state_output<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.value_artifact(value, events::ArtifactRole::StateOutput)
    }

    /// Builds a read fact-response artifact.
    pub fn fact_response<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::FactResponse)
    }

    /// Builds retained request/response evidence for an external capability read.
    pub fn external_read_evidence<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::ExternalReadEvidence)
    }

    /// Builds a private fact query replay evidence artifact from canonical facts-kernel bytes.
    pub fn fact_query_evidence(
        &self,
        evidence: &mfm_facts::FactQueryEvidence,
    ) -> Result<RunnerJsonArtifact> {
        let bytes =
            mfm_facts::canonical_fact_query_evidence_bytes(evidence).map_err(runtime_fact_error)?;
        self.json_bytes_artifact(
            bytes.as_bytes(),
            events::ArtifactRole::FactQueryEvidence,
            Some(mfm_facts::fact_query_evidence_schema_id().map_err(runtime_fact_error)?),
            None,
        )
    }

    /// Builds a side-effect intent artifact.
    pub(crate) fn side_effect_intent<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::SideEffectIntent)
    }

    /// Builds a typed prepared invocation artifact.
    pub(crate) fn prepared_invocation<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::PreparedInvocation)
    }

    /// Builds a not-submitted proof artifact.
    pub(crate) fn not_submitted_proof<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::NotSubmittedProof)
    }

    /// Builds a submission artifact.
    pub(crate) fn submission<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Submission)
    }

    /// Builds a submission-unknown evidence artifact.
    pub(crate) fn submission_unknown<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::SubmissionUnknownEvidence)
    }

    /// Builds a receipt artifact.
    pub(crate) fn receipt<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Receipt)
    }

    /// Builds a confirmation artifact.
    pub(crate) fn confirmation<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Confirmation)
    }

    /// Builds an ambiguity evidence artifact.
    pub(crate) fn ambiguity_evidence<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::AmbiguityEvidence)
    }

    /// Stages an inline attempt artifact through the existing runtime artifact boundary.
    pub fn staged_attempt(&self, artifact: &RunnerJsonArtifact) -> Result<StagedArtifact> {
        StagedArtifact::inline_attempt_artifact(
            self.ctx,
            artifact.bytes.clone(),
            artifact.evidence.clone(),
        )
    }

    /// Stages an inline side-effect artifact for one ledger epoch.
    pub(crate) fn staged_side_effect(
        &self,
        artifact: &RunnerJsonArtifact,
        ledger_key: events::SideEffectLedgerKey,
        invocation_epoch: u32,
    ) -> Result<StagedArtifact> {
        StagedArtifact::inline_side_effect_artifact(
            self.ctx,
            artifact.bytes.clone(),
            artifact.evidence.clone(),
            ledger_key,
            invocation_epoch,
        )
    }

    fn value_artifact<T>(&self, value: &T, role: events::ArtifactRole) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.json_artifact(
            value,
            role,
            Some(T::schema_id().map_err(runtime_value_error)?),
            Some(T::semantic_id().map_err(runtime_value_error)?),
        )
    }

    fn evidence_artifact<T>(
        &self,
        value: &T,
        role: events::ArtifactRole,
    ) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.json_artifact(
            value,
            role,
            Some(T::schema_id().map_err(runtime_value_error)?),
            None,
        )
    }

    fn json_artifact<T>(
        &self,
        value: &T,
        role: events::ArtifactRole,
        schema_id: Option<SchemaId>,
        semantic_type_id: Option<mfm_ids::SemanticTypeId>,
    ) -> Result<RunnerJsonArtifact>
    where
        T: Serialize,
    {
        let bytes = self.canonical_bytes(value)?;
        self.json_bytes_artifact(bytes.as_bytes(), role, schema_id, semantic_type_id)
    }

    fn json_bytes_artifact(
        &self,
        bytes: &[u8],
        role: events::ArtifactRole,
        schema_id: Option<SchemaId>,
        semantic_type_id: Option<mfm_ids::SemanticTypeId>,
    ) -> Result<RunnerJsonArtifact> {
        let digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: bytes.len() as u64,
            media_type: spec::MediaType::new("application/json")?,
            schema_id,
            semantic_type_id,
            producer_node_id: Some(self.ctx.node().node_id.clone()),
            producer_seed_id: None,
            artifact_role: role,
        };
        Ok(RunnerJsonArtifact {
            bytes: bytes.to_owned(),
            evidence,
        })
    }
}
