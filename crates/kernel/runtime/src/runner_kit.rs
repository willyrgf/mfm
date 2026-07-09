use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySetFor, CapabilitySpec};
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DescriptorId, DigestAlgorithm, NodeId, SchemaId,
};
use mfm_program::{EffectRunner, MfmFactType, StateSpec};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::{ContextBoundOutput, MfmConfig, MfmValue, NonEmpty, ValidatedConfig};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    artifacts::fact_query_returned_ref_retention_refs, AdapterExecutableBinding,
    ContextOutputExtractor, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding,
    ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCell, MaterializedCellTerminal,
    MaterializedInputNode, MaterializedInputs, Result, RunnerEventPayload, RunnerFactRecorded,
    RunnerIngressContext, RuntimeError, StagedArtifact, StagedRetentionRefs,
};

/// Canonical JSON artifact prepared by a typed runner before runtime staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerJsonArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

/// Reusable output extractor for values implementing [`ContextBoundOutput`].
#[derive(Debug, Clone, Copy, Default)]
pub struct TypedContextOutputExtractor<T> {
    _output: PhantomData<fn(T) -> T>,
}

impl<T> TypedContextOutputExtractor<T> {
    /// Creates a typed context-output extractor.
    pub const fn new() -> Self {
        Self {
            _output: PhantomData,
        }
    }
}

impl<T> ContextOutputExtractor for TypedContextOutputExtractor<T>
where
    T: ContextBoundOutput,
{
    fn validate_context_output(
        &self,
        cell_context: &spec::CellContextSpec,
        artifact: &store::ArtifactEvidenceRef,
        bytes: &[u8],
    ) -> Result<()> {
        let spec::CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            ..
        } = cell_context
        else {
            return Ok(());
        };
        let output = decode_json_bytes::<T>(bytes)?;
        if output.context_ref() != context_ref
            || output.context_resource_kind() != resource_kind
            || output.context_stage() != stage
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "context-bound output artifact {} does not match certified output cell context",
                artifact.artifact_id
            )));
        }
        Ok(())
    }
}

impl RunnerJsonArtifact {
    /// Returns the canonical JSON bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the artifact evidence bound to the canonical bytes.
    pub fn evidence(&self) -> &store::ArtifactEvidenceRef {
        &self.evidence
    }

    /// Returns a runtime retention ref for this artifact.
    pub fn retention_ref(&self) -> events::RetentionRef {
        self.evidence.retention_ref()
    }

    /// Splits the artifact into canonical bytes and evidence.
    pub fn into_parts(self) -> (Vec<u8>, store::ArtifactEvidenceRef) {
        (self.bytes, self.evidence)
    }
}

/// Runner-owned input for recording one typed fact claim.
pub struct FactRecordInput<T: MfmFactType> {
    /// Typed fact value containing the subject and response material.
    fact: T,
    /// Visibility selected for the recorded claim.
    visibility: mfm_facts::FactVisibility,
    /// Optional source observation timestamp.
    observed_at: Option<String>,
}

impl<T: MfmFactType> FactRecordInput<T> {
    /// Creates fact record input with no source observation timestamp.
    pub fn new(fact: T, visibility: mfm_facts::FactVisibility) -> Self {
        Self {
            fact,
            visibility,
            observed_at: None,
        }
    }

    /// Sets the source observation timestamp.
    pub fn observed_at(mut self, observed_at: impl Into<String>) -> Self {
        self.observed_at = Some(observed_at.into());
        self
    }
}

/// Precomputed executable identity material shared by runner factories in one adapter binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerExecutableIdentityTemplate {
    cargo_package_digest: ContentDigest,
    binary_digest: ContentDigest,
}

impl RunnerExecutableIdentityTemplate {
    /// Builds executable identity material from the stable adapter package and runner labels.
    pub fn new(
        cargo_package: &'static str,
        runner: &'static str,
        version: &'static str,
    ) -> Result<Self> {
        Ok(Self {
            cargo_package_digest: executable_identity_digest(serde_json::json!({
                "crate": cargo_package,
                "version": version,
            }))?,
            binary_digest: executable_identity_digest(serde_json::json!({
                "crate": cargo_package,
                "runner": runner,
                "version": version,
            }))?,
        })
    }

    /// Builds an executable identity for one logical runner or adapter factory.
    pub fn executable(&self, factory_id: events::RunnerFactoryId) -> events::ExecutableIdentity {
        events::ExecutableIdentity {
            factory_id,
            cargo_package_digest: self.cargo_package_digest.clone(),
            binary_digest: self.binary_digest.clone(),
            nix_derivation_hash: None,
            nix_output_hash: None,
        }
    }

    /// Builds a reusable factory binding for one logical runner or adapter factory.
    pub fn factory_binding(&self, factory_id: events::RunnerFactoryId) -> RunnerFactoryBinding {
        let executable = self.executable(factory_id.clone());
        RunnerFactoryBinding {
            factory_id,
            executable,
        }
    }
}

/// Bound runner factory id and executable identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerFactoryBinding {
    factory_id: events::RunnerFactoryId,
    executable: events::ExecutableIdentity,
}

impl RunnerFactoryBinding {
    /// Returns the runner factory id.
    pub fn factory_id(&self) -> events::RunnerFactoryId {
        self.factory_id.clone()
    }

    /// Returns the executable identity bound to the factory.
    pub fn executable(&self) -> events::ExecutableIdentity {
        self.executable.clone()
    }
}

/// Loads and validates the certified config artifact for the current runner node.
pub async fn load_runner_config<T>(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    load_runner_config_for_node(ctx.node(), artifacts).await
}

/// Loads and validates the certified config artifact for a specific certified node.
pub async fn load_runner_config_for_node<T>(
    node: &spec::NodeSpec,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let requirement = certified_config_requirement(node);
    let verified = read_retained_artifact(artifacts, &requirement).await?;
    let config = decode_verified_json::<T>(&verified)?;
    ValidatedConfig::new(config)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

/// Loads and validates the launch config artifact for the runner ingress node.
pub fn load_launch_config<T>(ctx: &RunnerIngressContext<'_>) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    load_launch_config_for_node(ctx, ctx.node())
}

/// Loads and validates the launch config artifact for a specific ingress node.
pub fn load_launch_config_for_node<T>(
    ctx: &RunnerIngressContext<'_>,
    node: &spec::NodeSpec,
) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let artifact = ctx.config_artifact_for_node(node)?;
    let config = serde_json::from_slice::<T>(&artifact.bytes).map_err(|error| {
        RuntimeError::RunnerBinding(format!(
            "launch config for node {} failed to decode: {error}",
            node.node_id
        ))
    })?;
    ValidatedConfig::new(config).map_err(|error| RuntimeError::RunnerBinding(error.to_string()))
}

/// Loads the root materialized input as a typed value from a state-output or seed cell.
pub async fn load_materialized_input_value<T>(
    inputs: &MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    load_materialized_node_value(&inputs.root, artifacts).await
}

/// Loads one materialized input node as a typed value from a state-output or seed cell.
pub async fn load_materialized_node_value<T>(
    node: &MaterializedInputNode,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::Cell(cell) = node else {
        return Err(RuntimeError::InvalidRunnerOutput(
            "materialized input node was not a cell".to_owned(),
        ));
    };
    let verified = load_materialized_cell_artifact(cell, artifacts).await?;
    decode_verified_json(&verified)
}

/// Loads the root materialized input as a struct-like value.
pub async fn load_materialized_struct_input<T>(
    inputs: &MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<T>
where
    T: DeserializeOwned,
{
    let value = materialized_input_node_json(&inputs.root, artifacts).await?;
    serde_json::from_value(value)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

/// Loads one field from a struct-shaped materialized input as a typed value.
pub async fn load_materialized_struct_field_value<T>(
    inputs: &MaterializedInputs,
    field_path: &str,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::Struct(fields) = &inputs.root else {
        return Err(RuntimeError::InvalidRunnerOutput(
            "materialized input root was not a struct".to_owned(),
        ));
    };
    let field = fields
        .iter()
        .find(|field| field.field_path.as_str() == field_path)
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "materialized input missing {field_path} field"
            ))
        })?;
    load_materialized_node_value(&field.node, artifacts).await
}

/// Loads a non-empty vector materialized input from cell-backed elements.
pub async fn load_non_empty_materialized_input<T>(
    inputs: &MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<NonEmpty<T>>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::NonEmptyVec(elements) = &inputs.root else {
        return Err(RuntimeError::InvalidRunnerOutput(
            "materialized input root was not a non-empty vector".to_owned(),
        ));
    };
    let mut values = Vec::with_capacity(elements.len());
    for element in elements {
        values.push(load_materialized_node_value(element, artifacts).await?);
    }
    NonEmpty::try_from_vec(values)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

/// Loads a side-effect artifact value produced by the current runner node.
pub async fn load_side_effect_value<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let (value, _) = load_side_effect_artifact(artifact, role, ctx, artifacts).await?;
    Ok(value)
}

/// Loads a side-effect artifact value produced by an explicit certified node.
pub async fn load_side_effect_value_for_node<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    producer_node_id: &NodeId,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let (value, _) =
        load_side_effect_artifact_for_node(artifact, role, producer_node_id, artifacts).await?;
    Ok(value)
}

/// Loads a side-effect artifact and returned evidence produced by the current runner node.
pub async fn load_side_effect_artifact<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<(T, store::ArtifactEvidenceRef)>
where
    T: MfmValue + DeserializeOwned,
{
    load_side_effect_artifact_for_node(artifact, role, &ctx.node().node_id, artifacts).await
}

/// Loads a side-effect artifact and returned evidence produced by an explicit certified node.
pub async fn load_side_effect_artifact_for_node<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    producer_node_id: &NodeId,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<(T, store::ArtifactEvidenceRef)>
where
    T: MfmValue + DeserializeOwned,
{
    let requirement = side_effect_artifact_requirement(artifact, role, producer_node_id)?;
    let verified = read_retained_artifact(artifacts, &requirement).await?;
    let evidence = verified.evidence().clone();
    let value = decode_verified_json(&verified)?;
    Ok((value, evidence))
}

/// Materializes an input node tree into JSON, loading cell bytes where needed.
pub async fn materialized_input_node_json(
    node: &MaterializedInputNode,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<serde_json::Value> {
    match node {
        MaterializedInputNode::Unit => Ok(serde_json::Value::Null),
        MaterializedInputNode::Cell(cell) => {
            let verified = load_materialized_cell_artifact(cell, artifacts).await?;
            decode_verified_json(&verified)
        }
        MaterializedInputNode::Tuple(elements) => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(Box::pin(materialized_input_node_json(element, artifacts)).await?);
            }
            Ok(serde_json::Value::Array(values))
        }
        MaterializedInputNode::Struct(fields) => {
            let mut object = serde_json::Map::new();
            for field in fields {
                object.insert(
                    field.field_path.as_str().to_owned(),
                    Box::pin(materialized_input_node_json(&field.node, artifacts)).await?,
                );
            }
            Ok(serde_json::Value::Object(object))
        }
        MaterializedInputNode::Vec(elements) | MaterializedInputNode::NonEmptyVec(elements) => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(Box::pin(materialized_input_node_json(element, artifacts)).await?);
            }
            Ok(serde_json::Value::Array(values))
        }
    }
}

async fn load_materialized_cell_artifact(
    cell: &MaterializedCell,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<store::VerifiedRunArtifactBytes> {
    let requirement = match &cell.terminal {
        MaterializedCellTerminal::Produced {
            producer_node_id,
            artifact_id,
            content_digest,
        } => store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::StateOutput,
            artifact_id: artifact_id.clone(),
            digest: Some(content_digest.clone()),
            byte_len: None,
            media_type: None,
            schema_id: Some(cell.schema_id.clone()),
            semantic_type_id: Some(cell.semantic_type_id.clone()),
            producer_node_id: Some(producer_node_id.clone()),
            producer_seed_id: None,
            artifact_role: Some(events::ArtifactRole::StateOutput),
        },
        MaterializedCellTerminal::Seed {
            seed_id,
            artifact_id,
            content_digest,
        } => store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::SeedCell,
            artifact_id: artifact_id.clone(),
            digest: Some(content_digest.clone()),
            byte_len: None,
            media_type: None,
            schema_id: Some(cell.schema_id.clone()),
            semantic_type_id: Some(cell.semantic_type_id.clone()),
            producer_node_id: None,
            producer_seed_id: Some(seed_id.clone()),
            artifact_role: Some(events::ArtifactRole::SeedInput),
        },
        MaterializedCellTerminal::Skipped { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(
                "materialized input cell was skipped".to_owned(),
            ));
        }
    };
    read_retained_artifact(artifacts, &requirement).await
}

async fn read_retained_artifact(
    artifacts: &dyn store::RetainedArtifactReadProvider,
    requirement: &store::EventArtifactRequirement,
) -> Result<store::VerifiedRunArtifactBytes> {
    artifacts
        .read_retained_artifact(requirement)
        .await
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn decode_verified_json<T>(artifact: &store::VerifiedRunArtifactBytes) -> Result<T>
where
    T: DeserializeOwned,
{
    decode_json_bytes(artifact.bytes())
}

fn decode_json_bytes<T>(bytes: &[u8]) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn certified_config_requirement(node: &spec::NodeSpec) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: node.config_ref.artifact_id.clone(),
        digest: Some(node.config_ref.digest.clone()),
        byte_len: Some(node.config_ref.byte_len),
        media_type: Some(node.config_ref.media_type.clone()),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    }
}

fn side_effect_artifact_requirement(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    producer_node_id: &NodeId,
) -> Result<store::EventArtifactRequirement> {
    Ok(store::EventArtifactRequirement {
        source: side_effect_artifact_source(role)?,
        artifact_id: artifact.artifact_id.clone(),
        digest: Some(artifact.content_digest.clone()),
        byte_len: None,
        media_type: None,
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: None,
        producer_node_id: Some(producer_node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(role),
    })
}

fn side_effect_artifact_source(
    role: events::ArtifactRole,
) -> Result<store::EventArtifactReferenceSource> {
    match role {
        events::ArtifactRole::SideEffectIntent => {
            Ok(store::EventArtifactReferenceSource::SideEffectIntent)
        }
        events::ArtifactRole::PreparedInvocation => {
            Ok(store::EventArtifactReferenceSource::PreparedInvocation)
        }
        events::ArtifactRole::NotSubmittedProof => {
            Ok(store::EventArtifactReferenceSource::NotSubmittedProof)
        }
        events::ArtifactRole::Submission => Ok(store::EventArtifactReferenceSource::Submission),
        events::ArtifactRole::SubmissionUnknownEvidence => {
            Ok(store::EventArtifactReferenceSource::SubmissionUnknownEvidence)
        }
        events::ArtifactRole::Receipt => Ok(store::EventArtifactReferenceSource::Receipt),
        events::ArtifactRole::Confirmation => Ok(store::EventArtifactReferenceSource::Confirmation),
        events::ArtifactRole::AmbiguityEvidence => {
            Ok(store::EventArtifactReferenceSource::AmbiguityEvidence)
        }
        _ => Err(RuntimeError::InvalidRunnerOutput(format!(
            "artifact role {role:?} is not a side-effect artifact"
        ))),
    }
}

fn executable_identity_digest(value: serde_json::Value) -> Result<ContentDigest> {
    let json = serde_json::to_string(&value)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    Ok(PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?
        .content_digest())
}

/// Builder for runner-owned JSON artifacts and staging handles.
pub struct RunnerArtifactBuilder<'a, 'ctx> {
    ctx: &'a ErasedRunCtx<'ctx>,
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

    /// Builds a schema-less prepared invocation artifact.
    pub(crate) fn prepared_invocation<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: Serialize,
    {
        self.json_artifact(value, events::ArtifactRole::PreparedInvocation, None, None)
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

/// Capability and adapter binding metadata for runner payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerCapabilityBinding {
    /// Capability kind used by the runner.
    pub(crate) capability_kind: CapabilityKind,
    /// Capability version used by the runner.
    pub(crate) capability_version: CapabilityVersion,
    /// Adapter kind used by the runner.
    pub(crate) adapter_kind: AdapterKind,
    /// Adapter version used by the runner.
    pub(crate) adapter_version: AdapterVersion,
}

impl RunnerCapabilityBinding {
    /// Builds binding metadata for a typed capability and adapter identity.
    pub fn for_capability<C>(
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    ) -> Result<Self>
    where
        C: CapabilitySpec,
    {
        Ok(Self {
            capability_kind: C::kind()
                .map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?,
            capability_version: C::version()
                .map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?,
            adapter_kind,
            adapter_version,
        })
    }

    /// Returns the capability kind used by the runner.
    pub const fn capability_kind(&self) -> &CapabilityKind {
        &self.capability_kind
    }

    /// Returns the capability version used by the runner.
    pub const fn capability_version(&self) -> &CapabilityVersion {
        &self.capability_version
    }

    /// Returns the adapter kind used by the runner.
    pub const fn adapter_kind(&self) -> &AdapterKind {
        &self.adapter_kind
    }

    /// Returns the adapter version used by the runner.
    pub const fn adapter_version(&self) -> &AdapterVersion {
        &self.adapter_version
    }
}

/// Shared side-effect ledger coordinates for runner payload builders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunnerSideEffectBinding {
    /// Side-effect ledger key.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: events::SideEffectLedgerPurpose,
    /// Certified side-effect pair id.
    pub pair_id: mfm_ids::SideEffectPairId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
}

/// Claim metadata for a side-effect invocation owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunnerClaimBinding {
    /// Claim owner.
    pub claim_owner: events::RunnerInvocationId,
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
}

/// Prepared invocation metadata for a side-effect runner payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunnerPreparedInvocationBinding {
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
    /// Optional exclusive resource lane key evidence echoed from the held lane.
    pub resource_key: Option<events::ResourceKeyEvidence>,
}

/// Builder for runner-owned event payloads.
pub struct RunnerPayloadBuilder<'a, 'ctx> {
    ctx: &'a ErasedRunCtx<'ctx>,
}

macro_rules! runner_side_effect_payload {
    (
        $builder:expr,
        $side_effect:expr,
        $pair_role:expr,
        $variant:ident {
            $($field:ident : $value:expr),* $(,)?
        }
    ) => {
        side_effect::$variant {
            spec_hash: $builder.ctx.spec_hash().clone(),
            node_id: $builder.ctx.node().node_id.clone(),
            attempt_id: $builder.ctx.attempt_id().clone(),
            ledger_key: $side_effect.ledger_key.clone(),
            ledger_purpose: $side_effect.ledger_purpose.clone(),
            pair_id: $side_effect.pair_id.clone(),
            pair_role: $pair_role,
            invocation_epoch: $side_effect.invocation_epoch,
            $($field: $value,)*
        }
    };
}

impl<'a, 'ctx> RunnerPayloadBuilder<'a, 'ctx> {
    /// Creates a payload builder bound to one runner invocation context.
    pub fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self { ctx }
    }

    /// Builds a `CellProduced` runner payload from a state-output artifact.
    pub fn cell_produced(&self, artifact: &RunnerJsonArtifact) -> Result<RunnerEventPayload> {
        ensure_artifact_role(artifact, events::ArtifactRole::StateOutput)?;
        Ok(RunnerEventPayload::CellProduced(events::CellProduced {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            cell_id: self.ctx.node().output_cell.clone(),
            scope_id: self.ctx.output_cell().scope_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            semantic_type_id: self.ctx.output_cell().semantic_type_id.clone(),
            schema_id: self.ctx.output_cell().schema_id.clone(),
            value_lineage: self.ctx.output_cell().value_lineage.clone(),
            context: self.ctx.output_cell().context.clone(),
            artifact_id: artifact.evidence.artifact_id.clone(),
            content_digest: artifact.evidence.digest.clone(),
            producer_state_kind: Some(self.ctx.node().state_kind.clone()),
            producer_state_version: Some(self.ctx.node().state_version.clone()),
        }))
    }

    /// Builds a `CellSkipped` runner payload for a certified maybe-skipped output cell.
    pub fn cell_skipped(&self, skip_reason: events::SkipReason) -> RunnerEventPayload {
        RunnerEventPayload::CellSkipped(events::CellSkipped {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            cell_id: self.ctx.node().output_cell.clone(),
            scope_id: self.ctx.output_cell().scope_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            semantic_type_id: self.ctx.output_cell().semantic_type_id.clone(),
            schema_id: self.ctx.output_cell().schema_id.clone(),
            value_lineage: self.ctx.output_cell().value_lineage.clone(),
            context: self.ctx.output_cell().context.clone(),
            skip_reason,
        })
    }

    /// Builds a `SideEffectIntentPersisted` runner payload.
    pub(crate) fn side_effect_intent_persisted<Idempotency>(
        &self,
        side_effect: RunnerSideEffectBinding,
        intent: &RunnerJsonArtifact,
        idempotency: &Idempotency,
        idempotency_key: events::IdempotencyKeyRef,
        binding: RunnerCapabilityBinding,
    ) -> Result<RunnerEventPayload>
    where
        Idempotency: MfmValue,
    {
        ensure_artifact_role(intent, events::ArtifactRole::SideEffectIntent)?;
        Ok(RunnerEventPayload::SideEffectIntentPersisted(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Submit,
                IntentPersisted {
                    scope_id: self.ctx.node().scope_id.clone(),
                    intent_schema_id: artifact_schema_id(intent)?,
                    intent_hash: intent.evidence.digest.clone(),
                    intent_artifact_id: intent.evidence.artifact_id.clone(),
                    idempotency_input_schema_id: Idempotency::schema_id()
                        .map_err(runtime_value_error)?,
                    idempotency_input_hash: canonical_json(idempotency)?.content_digest(),
                    idempotency_key: idempotency_key,
                    capability_kind: binding.capability_kind,
                    capability_version: binding.capability_version,
                    adapter_kind: binding.adapter_kind,
                    adapter_version: binding.adapter_version,
                }
            ),
        ))
    }

    /// Builds a `SideEffectClaimed` runner payload.
    pub(crate) fn side_effect_claimed(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectClaimed(runner_side_effect_payload!(
            self,
            side_effect,
            events::SideEffectPairRole::Submit,
            Claimed {
                claim_owner: claim.claim_owner,
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token,
            }
        ))
    }

    /// Builds a `ResourceLaneReleaseIntent` runner payload for terminal lane release.
    pub(crate) fn resource_lane_release_intent(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim_id: events::ResourceLaneClaimId,
        release_reason: events::ResourceLaneReleaseReason,
    ) -> RunnerEventPayload {
        RunnerEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
            spec_hash: self.ctx.spec_hash().clone(),
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: events::SideEffectPairRole::Verify,
            invocation_epoch: side_effect.invocation_epoch,
            claim_id,
            release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
            release_reason,
        })
    }

    /// Builds a `SideEffectInvocationPrepared` runner payload.
    pub(crate) fn side_effect_invocation_prepared(
        &self,
        side_effect: RunnerSideEffectBinding,
        prepared: Option<&RunnerJsonArtifact>,
        prepared_binding: RunnerPreparedInvocationBinding,
    ) -> Result<RunnerEventPayload> {
        if let Some(artifact) = prepared {
            ensure_artifact_role(artifact, events::ArtifactRole::PreparedInvocation)?;
        }
        Ok(RunnerEventPayload::SideEffectInvocationPrepared(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Submit,
                InvocationPrepared {
                    claim_generation: prepared_binding.claim_generation,
                    claim_fencing_token: prepared_binding.claim_fencing_token,
                    resource_key: prepared_binding.resource_key,
                    prepared_artifact_id: prepared
                        .map(|artifact| artifact.evidence.artifact_id.clone()),
                    prepared_hash: prepared.map(|artifact| artifact.evidence.digest.clone()),
                }
            ),
        ))
    }

    /// Builds a `SideEffectInvocationStarted` runner payload.
    pub(crate) fn side_effect_invocation_started(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectInvocationStarted(runner_side_effect_payload!(
            self,
            side_effect,
            events::SideEffectPairRole::Submit,
            InvocationStarted {
                claim_owner: claim.claim_owner,
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token,
            }
        ))
    }

    /// Builds a `SideEffectNotSubmittedProven` runner payload with an explicit pair role.
    pub(crate) fn side_effect_not_submitted_proven_with_role(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        proof: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(proof, events::ArtifactRole::NotSubmittedProof)?;
        Ok(RunnerEventPayload::SideEffectNotSubmittedProven(
            runner_side_effect_payload!(
                self,
                side_effect,
                pair_role,
                NotSubmittedProven {
                    proof_schema_id: artifact_schema_id(proof)?,
                    proof_hash: proof.evidence.digest.clone(),
                    proof_artifact_id: proof.evidence.artifact_id.clone(),
                }
            ),
        ))
    }

    /// Builds a `SideEffectSubmissionObserved` runner payload with an explicit pair role.
    pub(crate) fn side_effect_submission_observed_with_role(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        submission: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(submission, events::ArtifactRole::Submission)?;
        Ok(RunnerEventPayload::SideEffectSubmissionObserved(
            runner_side_effect_payload!(
                self,
                side_effect,
                pair_role,
                SubmissionObserved {
                    submission_schema_id: artifact_schema_id(submission)?,
                    submission_hash: submission.evidence.digest.clone(),
                    submission_artifact_id: submission.evidence.artifact_id.clone(),
                }
            ),
        ))
    }

    /// Builds a `SideEffectSubmissionUnknown` runner payload.
    pub(crate) fn side_effect_submission_unknown(
        &self,
        side_effect: RunnerSideEffectBinding,
        evidence: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(evidence, events::ArtifactRole::SubmissionUnknownEvidence)?;
        Ok(RunnerEventPayload::SideEffectSubmissionUnknown(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Submit,
                SubmissionUnknown {
                    evidence_schema_id: artifact_schema_id(evidence)?,
                    evidence_hash: evidence.evidence.digest.clone(),
                    evidence_artifact_id: evidence.evidence.artifact_id.clone(),
                }
            ),
        ))
    }

    /// Builds a `SideEffectReceiptObserved` runner payload.
    pub(crate) fn side_effect_receipt_observed(
        &self,
        side_effect: RunnerSideEffectBinding,
        receipt: &RunnerJsonArtifact,
        replay_verifier_id: events::ReplayVerifierId,
        resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(receipt, events::ArtifactRole::Receipt)?;
        Ok(RunnerEventPayload::SideEffectReceiptObserved(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Verify,
                ReceiptObserved {
                    receipt_schema_id: artifact_schema_id(receipt)?,
                    receipt_hash: receipt.evidence.digest.clone(),
                    receipt_artifact_id: receipt.evidence.artifact_id.clone(),
                    replay_verifier_id: replay_verifier_id,
                    resource_touched_set: resource_touched_set,
                }
            ),
        ))
    }

    /// Builds a `SideEffectConfirmationObserved` runner payload.
    pub(crate) fn side_effect_confirmation_observed(
        &self,
        side_effect: RunnerSideEffectBinding,
        confirmation: &RunnerJsonArtifact,
        replay_verifier_id: events::ReplayVerifierId,
        resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(confirmation, events::ArtifactRole::Confirmation)?;
        Ok(RunnerEventPayload::SideEffectConfirmationObserved(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Verify,
                ConfirmationObserved {
                    confirmation_schema_id: artifact_schema_id(confirmation)?,
                    confirmation_hash: confirmation.evidence.digest.clone(),
                    confirmation_artifact_id: confirmation.evidence.artifact_id.clone(),
                    replay_verifier_id: replay_verifier_id,
                    resource_touched_set: resource_touched_set,
                }
            ),
        ))
    }

    /// Builds a `SideEffectAmbiguous` runner payload.
    pub(crate) fn side_effect_ambiguous(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        ambiguity_code: events::AmbiguityCode,
        evidence: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(evidence, events::ArtifactRole::AmbiguityEvidence)?;
        Ok(RunnerEventPayload::SideEffectAmbiguous(
            runner_side_effect_payload!(
                self,
                side_effect,
                pair_role,
                Ambiguous {
                    ambiguity_code: ambiguity_code,
                    evidence_schema_id: artifact_schema_id(evidence)?,
                    evidence_hash: evidence.evidence.digest.clone(),
                    evidence_artifact_id: evidence.evidence.artifact_id.clone(),
                }
            ),
        ))
    }

    /// Builds a `SideEffectFailed` runner payload.
    pub(crate) fn side_effect_failed(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        failure_phase: side_effect::FailurePhase,
        retryable: bool,
        error: events::MfmErrorInfo,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectFailed(runner_side_effect_payload!(
            self,
            side_effect,
            pair_role,
            Failed {
                failure_phase: failure_phase,
                retryable: retryable,
                error: error,
            }
        ))
    }
}

/// Builder for assembling an erased runner output batch.
pub struct RunnerOutputBuilder<'a, 'ctx> {
    artifacts: RunnerArtifactBuilder<'a, 'ctx>,
    staged_artifacts: Vec<StagedArtifact>,
    staged_retention_refs: Vec<StagedRetentionRefs>,
    payloads: Vec<RunnerEventPayload>,
}

impl<'a, 'ctx> RunnerOutputBuilder<'a, 'ctx> {
    /// Creates an empty output builder bound to one runner invocation context.
    pub fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self {
            artifacts: RunnerArtifactBuilder::new(ctx),
            staged_artifacts: Vec::new(),
            staged_retention_refs: Vec::new(),
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
        self.retain_runtime_evidence(artifact);
        Ok(self)
    }

    /// Appends runtime retention refs for one artifact.
    pub fn retain_runtime_evidence(&mut self, artifact: &RunnerJsonArtifact) -> &mut Self {
        self.staged_retention_refs
            .push(StagedRetentionRefs::runtime_evidence(vec![
                artifact.retention_ref()
            ]));
        self
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

    /// Stages a state-output artifact for a fact value and appends the matching fact record.
    pub fn state_output_and_record_fact<T>(
        &mut self,
        input: FactRecordInput<T>,
        producer: RunnerCapabilityBinding,
    ) -> Result<()>
    where
        T: MfmFactType + MfmValue,
    {
        let payloads = RunnerPayloadBuilder::new(self.artifacts.ctx);
        let state_artifact = self.stage_state_output_artifact(&input.fact)?;
        self.record_fact(input, producer)?;
        self.payload(payloads.cell_produced(&state_artifact)?);
        Ok(())
    }

    /// Stages a state-output artifact and private fact-query replay evidence.
    pub fn state_output_and_record_fact_query_evidence<T>(
        &mut self,
        value: &T,
        evidence: mfm_facts::FactQueryEvidence,
        trust_root: &store::FactQueryReceiptTrustRoot,
    ) -> Result<()>
    where
        T: MfmValue,
    {
        self.state_output(value)?;
        self.record_fact_query_evidence(evidence, trust_root)
    }

    /// Stages a typed fact response artifact and appends the matching `FactRecorded` payload.
    pub fn record_fact<T>(
        &mut self,
        input: FactRecordInput<T>,
        producer: RunnerCapabilityBinding,
    ) -> Result<()>
    where
        T: MfmFactType,
    {
        let descriptor = T::descriptor().map_err(runtime_fact_error)?;
        ensure_fact_descriptor_matches_type::<T>(&descriptor)?;
        let descriptor_hash =
            mfm_facts::fact_descriptor_hash(&descriptor).map_err(runtime_fact_error)?;
        ensure_node_allows_fact_descriptor(self.artifacts.ctx.node(), &descriptor_hash)?;

        let subject_json = serde_json::to_value(input.fact.subject())
            .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
        let subject = mfm_facts::typed_fact_subject_evidence(&descriptor, &subject_json)
            .map_err(runtime_fact_error)?;
        let response = self.artifacts.fact_response(input.fact.response())?;
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
            visibility: input.visibility,
            fact_kind: descriptor.fact_kind().clone(),
            fact_descriptor_hash: descriptor_hash,
            subject,
            observed_at: input.observed_at,
            request: None,
            response: mfm_facts::FactResponseEvidence::new(
                response_schema_id,
                response_evidence.digest.clone(),
                response_evidence.artifact_id.clone(),
                response_evidence.evidence_hash()?,
            ),
            producer: mfm_facts::FactProducerProvenance::new(
                producer.capability_kind,
                producer.capability_version,
                producer.adapter_kind,
                producer.adapter_version,
            ),
        })
        .map_err(runtime_fact_error)?;

        self.stage_attempt_artifact(&response)?;
        self.payload(RunnerEventPayload::FactRecorded(RunnerFactRecorded::new(
            events::FactRecorded {
                spec_hash: self.artifacts.ctx.spec_hash().clone(),
                node_id: self.artifacts.ctx.node().node_id.clone(),
                attempt_id: self.artifacts.ctx.attempt_id().clone(),
                claim,
            },
        )));

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
        trust_root: &store::FactQueryReceiptTrustRoot,
    ) -> Result<()> {
        store::validate_fact_query_evidence_recording(&evidence, trust_root)
            .map_err(RuntimeError::from)?;
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
        self.retain_runtime_evidence(&artifact);
        Ok(artifact)
    }

    /// Appends a runner-owned event payload.
    pub fn payload(&mut self, payload: RunnerEventPayload) -> &mut Self {
        self.payloads.push(payload);
        self
    }

    /// Finishes the builder into an erased runner output batch.
    pub fn finish(self) -> ErasedRunnerOutput {
        ErasedRunnerOutput::from_parts(
            self.staged_artifacts,
            self.staged_retention_refs,
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

fn fact_query_evidence_retention_refs(
    evidence_artifact: &RunnerJsonArtifact,
    evidence: &mfm_facts::FactQueryEvidence,
    projections: &store::ProjectionSnapshot,
) -> Result<Vec<events::RetentionRef>> {
    let mut refs = BTreeMap::<(ArtifactId, events::ArtifactRole), events::RetentionRef>::new();
    insert_retention_ref(&mut refs, evidence_artifact.retention_ref());

    for fact_ref in evidence.receipt().returned_refs() {
        for retention_ref in fact_query_returned_ref_retention_refs(projections, fact_ref)? {
            insert_retention_ref(&mut refs, retention_ref);
        }
    }

    Ok(refs.into_values().collect())
}

fn insert_retention_ref(
    refs: &mut BTreeMap<(ArtifactId, events::ArtifactRole), events::RetentionRef>,
    retention_ref: events::RetentionRef,
) {
    refs.entry((retention_ref.artifact_id.clone(), retention_ref.role))
        .or_insert(retention_ref);
}

/// Builder for registering runner bindings while keeping executable identity explicit.
pub struct RunnerRegistrationBuilder<'a> {
    registry: &'a mut ErasedRunnerRegistry,
}

impl<'a> RunnerRegistrationBuilder<'a> {
    /// Creates a runner registration builder.
    pub fn new(registry: &'a mut ErasedRunnerRegistry) -> Self {
        Self { registry }
    }

    /// Registers one descriptor runner using caller-supplied factory and executable identity.
    pub fn register_runner(
        &mut self,
        descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        let binding = ErasedRunnerBinding::new(descriptor_id, factory_id, executable, runner)?;
        self.registry.register(binding)?;
        Ok(self)
    }

    /// Registers a typed state runner when its capabilities are bound by process assembly.
    pub fn register_state_runner_with_factory<S>(
        &mut self,
        factory: &RunnerFactoryBinding,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<mfm_program::StateDescriptorIdentity>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = mfm_program::state_descriptor::<S>()
            .map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?;
        self.register_runner(
            descriptor.descriptor_id().clone(),
            factory.factory_id(),
            factory.executable(),
            runner,
        )?;
        Ok(descriptor)
    }

    /// Registers the adapter-owned runner used by side-effect verify framework nodes
    /// for one certified side-effect submit descriptor.
    pub fn register_side_effect_verify_runner(
        &mut self,
        submit_descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        self.registry.register_side_effect_verify_runner(
            submit_descriptor_id,
            factory_id,
            executable,
            runner,
        )?;
        Ok(self)
    }

    /// Registers an adapter-owned side-effect verify runner using a bound factory.
    pub fn register_side_effect_verify_runner_with_factory(
        &mut self,
        submit_descriptor_id: DescriptorId,
        factory: &RunnerFactoryBinding,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        self.register_side_effect_verify_runner(
            submit_descriptor_id,
            factory.factory_id(),
            factory.executable(),
            runner,
        )
    }

    /// Registers executable evidence for one certified adapter binding.
    pub fn register_adapter_executable(
        &mut self,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        executable: events::ExecutableIdentity,
    ) -> Result<&mut Self> {
        self.registry
            .register_adapter_executable(AdapterExecutableBinding::new(
                adapter_kind,
                adapter_version,
                executable,
            ))?;
        Ok(self)
    }

    /// Registers executable evidence for one certified adapter binding using a bound factory.
    pub fn register_adapter_executable_with_factory(
        &mut self,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        factory: &RunnerFactoryBinding,
    ) -> Result<&mut Self> {
        self.register_adapter_executable(adapter_kind, adapter_version, factory.executable())
    }
}

fn ensure_artifact_role(artifact: &RunnerJsonArtifact, role: events::ArtifactRole) -> Result<()> {
    if artifact.evidence.artifact_role == role {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner artifact role {} did not match expected role {}",
            artifact.evidence.artifact_role.as_str(),
            role.as_str()
        )))
    }
}

fn artifact_schema_id(artifact: &RunnerJsonArtifact) -> Result<SchemaId> {
    artifact.evidence.schema_id.clone().ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "runner artifact role {} did not carry schema metadata",
            artifact.evidence.artifact_role.as_str()
        ))
    })
}

fn ensure_fact_descriptor_matches_type<T>(descriptor: &mfm_facts::FactDescriptor) -> Result<()>
where
    T: MfmFactType,
{
    let subject_schema_id = <T::Subject as MfmValue>::schema_id().map_err(runtime_value_error)?;
    let response_schema_id = <T::Response as MfmValue>::schema_id().map_err(runtime_value_error)?;

    if descriptor.subject_schema_id() != &subject_schema_id {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "fact subject schema {} did not match subject type schema {}",
            descriptor.subject_schema_id(),
            subject_schema_id
        )));
    }
    if descriptor.response_schema_id() != &response_schema_id {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "fact response schema {} did not match response type schema {}",
            descriptor.response_schema_id(),
            response_schema_id
        )));
    }

    Ok(())
}

fn ensure_node_allows_fact_descriptor(
    node: &spec::NodeSpec,
    descriptor_hash: &ContentDigest,
) -> Result<()> {
    if node
        .fact_descriptor_allowlist
        .iter()
        .any(|reference| &reference.descriptor_hash == descriptor_hash)
    {
        return Ok(());
    }
    Err(RuntimeError::InvalidRunnerOutput(format!(
        "node {} is not certified to emit fact descriptor {}",
        node.node_id, descriptor_hash
    )))
}

fn canonical_json<T>(value: &T) -> Result<PlainCanonicalJsonBytes>
where
    T: Serialize,
{
    let json =
        serde_json::to_string(value).map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))
}

fn runtime_value_error(error: mfm_values::ValueError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_fact_error(error: mfm_facts::FactError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use mfm_canonical::CanonicalValue;
    use mfm_store::v1::test_support::{
        fact_descriptor_projection_fixture_for_test, fact_projection_fixture_for_test,
        signed_fact_query_receipt_for_test, FactProjectionFixtureInputForTest,
        SignedFactQueryReceiptFixtureInputForTest,
    };

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([byte; 32]),
        )
    }

    fn artifact_id(byte: u8) -> ArtifactId {
        ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([byte; 32]),
        )
    }

    fn run_id(byte: u8) -> mfm_ids::RunId {
        mfm_ids::RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([byte; 32]),
        )
    }

    fn event_id(byte: u8) -> mfm_ids::EventId {
        mfm_ids::EventId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([byte; 32]),
        )
    }

    fn schema_id() -> SchemaId {
        SchemaId::new(
            "mfm.test.response",
            "mfm.test.v1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([1; 32]),
        )
        .expect("schema")
    }

    fn capability_kind() -> CapabilityKind {
        CapabilityKind::new(
            "mfm.test.capability",
            "read",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([2; 32]),
        )
        .expect("capability")
    }

    fn adapter_kind() -> AdapterKind {
        AdapterKind::new(
            "mfm.test.adapter",
            "read",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([3; 32]),
        )
        .expect("adapter")
    }

    fn descriptor() -> mfm_facts::FactDescriptor {
        mfm_facts::FactDescriptor::new(
            mfm_facts::FactKind::new("chain.head").expect("kind"),
            schema_id(),
            schema_id(),
            schema_id(),
            vec![
                mfm_facts::FactFieldDescriptor::new(
                    mfm_facts::FactFieldId::new("subject.height").expect("field"),
                    mfm_facts::FactFieldValueType::UnsignedInteger,
                    mfm_facts::FactFieldExtraction::Subject(
                        mfm_facts::CanonicalValuePath::new("height").expect("extraction"),
                    ),
                    mfm_facts::FactFieldPolicy::new(
                        vec![mfm_facts::FactQueryOperator::Equal],
                        mfm_facts::FactFieldExposure::Returnable,
                    )
                    .required(),
                )
                .expect("subject field"),
                mfm_facts::FactFieldDescriptor::new(
                    mfm_facts::FactFieldId::new("result.height").expect("field"),
                    mfm_facts::FactFieldValueType::UnsignedInteger,
                    mfm_facts::FactFieldExtraction::Response(
                        mfm_facts::CanonicalValuePath::new("height").expect("extraction"),
                    ),
                    mfm_facts::FactFieldPolicy::new(
                        vec![mfm_facts::FactQueryOperator::Equal],
                        mfm_facts::FactFieldExposure::Returnable,
                    )
                    .sortable()
                    .required(),
                )
                .expect("result field"),
            ],
            vec![mfm_facts::FactOrderingPolicy::new(
                mfm_facts::FactOrderingName::new("result.height.asc").expect("ordering"),
                vec![mfm_facts::FactOrderingTerm::new(
                    mfm_facts::FactFieldId::new("result.height").expect("field"),
                    mfm_facts::SortDirection::Ascending,
                    mfm_facts::NullOrdering::Last,
                    true,
                )],
            )
            .expect("ordering")],
        )
        .expect("descriptor")
    }

    fn fact_producer_node_id() -> mfm_ids::NodeId {
        mfm_ids::NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([0x19; 32]),
        )
    }

    fn fact_projection_fixture(
        subject_height: u64,
    ) -> (
        mfm_facts::InternalFactRef,
        store::FactDescriptorProjection,
        store::FactRecordProjection,
        store::FactIndexProjection,
    ) {
        let descriptor = descriptor();
        let descriptor_fixture = fact_descriptor_projection_fixture_for_test(descriptor.clone())
            .expect("descriptor fixture");
        let fact_fixture = fact_projection_fixture_for_test(
            &descriptor,
            descriptor_fixture.descriptor_hash.clone(),
            FactProjectionFixtureInputForTest {
                run_id: run_id(0x10),
                source_seq: 1,
                source_ordinal: 0,
                source_event_id: event_id(0x11),
                node_id: fact_producer_node_id(),
                attempt_id: mfm_ids::AttemptId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    mfm_ids::DigestBytes::from_array([0x42; 32]),
                ),
                commit_id: store::CommitKey::new("fact-query-authority").expect("commit key"),
                store_commit_order: 1,
                recorded_at: "2026-07-01T00:00:00Z".to_owned(),
                observed_at: None,
                visibility: mfm_facts::FactVisibility::indexed_default(
                    mfm_facts::FactAudience::Platform,
                ),
                subject: CanonicalValue::object([(
                    "height",
                    CanonicalValue::Unsigned(subject_height),
                )])
                .expect("subject"),
                response: CanonicalValue::object([("height", CanonicalValue::Unsigned(800000))])
                    .expect("response"),
                request: None,
                response_schema_id: schema_id(),
                response_artifact_id: None,
                producer: mfm_facts::FactProducerProvenance::new(
                    capability_kind(),
                    CapabilityVersion::new("mfm.test.capability.v1").expect("capability version"),
                    adapter_kind(),
                    AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
                ),
            },
        )
        .expect("fact fixture");
        let index = fact_fixture.index.expect("indexed fact fixture");
        let fact_ref = index.internal_ref().expect("internal fact ref");
        (
            fact_ref,
            descriptor_fixture.projection,
            fact_fixture.record,
            index,
        )
    }

    fn fact_ref() -> mfm_facts::InternalFactRef {
        fact_projection_fixture(17).0
    }

    fn fact_authority_projections(
        fact_ref: &mfm_facts::InternalFactRef,
    ) -> store::ProjectionSnapshot {
        let (_built_ref, descriptor_projection, record_projection, index_projection) =
            fact_projection_fixture(17);
        assert_eq!(index_projection.fact_claim_id, *fact_ref.fact_claim_id());
        assert_eq!(
            index_projection.source_event_id,
            *fact_ref.source_event_id()
        );
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_descriptors: BTreeMap::from([(
                descriptor_projection.descriptor_hash.clone(),
                descriptor_projection,
            )]),
            fact_records: BTreeMap::from([(
                record_projection.fact_claim_id.clone(),
                record_projection,
            )]),
            fact_index_entries: BTreeMap::from([(
                index_projection.fact_claim_id.clone(),
                index_projection,
            )]),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("projection snapshot")
    }

    fn fact_authority_projections_without(
        fact_ref: &mfm_facts::InternalFactRef,
        descriptor: bool,
        record: bool,
        index: bool,
    ) -> store::ProjectionSnapshot {
        let full = fact_authority_projections(fact_ref);
        let mut parts = store::ProjectionSnapshotParts::from_snapshot(&full);
        if !descriptor {
            parts.fact_descriptors.clear();
        }
        if !record {
            parts.fact_records.clear();
        }
        if !index {
            parts.fact_index_entries.clear();
        }
        store::ProjectionSnapshot::from_parts(parts).expect("filtered fact authority projection")
    }

    fn fact_authority_projections_with_tampered_subject(
        fact_ref: &mfm_facts::InternalFactRef,
    ) -> store::ProjectionSnapshot {
        let full = fact_authority_projections(fact_ref);
        let mut parts = store::ProjectionSnapshotParts::from_snapshot(&full);
        parts
            .fact_records
            .get_mut(fact_ref.fact_claim_id())
            .expect("fact record")
            .claim = fact_projection_fixture(18).2.claim;
        store::ProjectionSnapshot::from_parts(parts).expect("tampered fact authority projection")
    }

    fn query_evidence_artifact() -> RunnerJsonArtifact {
        RunnerJsonArtifact {
            bytes: b"{}".to_vec(),
            evidence: store::ArtifactEvidenceRef {
                artifact_id: artifact_id(0x30),
                digest: digest(0x31),
                byte_len: 2,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id: None,
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::FactQueryEvidence,
            },
        }
    }

    fn query_evidence(fact_ref: mfm_facts::InternalFactRef) -> mfm_facts::FactQueryEvidence {
        let query_scope = mfm_facts::FactQueryScope::new(
            mfm_facts::FactAudience::Platform,
            mfm_facts::FactVisibilityScope::Default,
        );
        let store_scope = mfm_facts::StoreScopeRef::new("default").expect("store scope");
        let input = mfm_facts::FactQueryInput::new(
            store_scope.clone(),
            query_scope.clone(),
            mfm_facts::ScopeDecisionEvidence::new(digest(0x19)),
            vec![mfm_facts::FactQueryPredicate::new(
                mfm_facts::FactFieldId::new("subject.height").expect("field"),
                mfm_facts::FactQueryOperator::Equal,
                mfm_facts::FactCanonicalScalar::UnsignedInteger(1),
            )],
            vec![mfm_facts::FactFieldId::new("result.height").expect("field")],
            mfm_facts::FactOrderingName::new("result.height.asc").expect("ordering"),
            Some(1),
        )
        .expect("query input");
        let plan = mfm_facts::compile_fact_query_plan(&descriptor(), input).expect("plan");
        let frontier = mfm_facts::StoreReadFrontier::new(
            store_scope,
            query_scope,
            mfm_facts::DescriptorCatalogWatermark::new(1),
            mfm_facts::FactProjectionGeneration::new(1),
            1,
            mfm_facts::StoreCommitWatermark::new(1),
        );
        let plan_hash = mfm_facts::fact_query_plan_hash(&plan).expect("plan hash");
        let rows = [mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new())];
        let key = SigningKey::from_bytes(&[0x52; 32]);
        let receipt =
            signed_fact_query_receipt_for_test(SignedFactQueryReceiptFixtureInputForTest {
                plan_hash: &plan_hash,
                key: &key,
                store_identity: mfm_facts::StoreIdentity::new("store.default").expect("store"),
                key_id: mfm_facts::StoreKeyId::new("key.default").expect("key"),
                read_frontier: frontier,
                rows: &rows,
                include_returned_field_summaries: false,
                limit: None,
            });
        mfm_facts::FactQueryEvidence::new(
            plan,
            receipt,
            mfm_facts::FactSelectionEvidence::new(digest(0x22), Vec::new(), None)
                .expect("selection"),
        )
    }

    #[test]
    fn fact_query_evidence_retention_refs_include_returned_fact_authority_artifacts() {
        let fact_ref = fact_ref();
        let evidence_artifact = query_evidence_artifact();
        let projections = fact_authority_projections(&fact_ref);
        let descriptor_projection = projections
            .fact_descriptor(fact_ref.fact_descriptor_hash())
            .expect("descriptor projection");
        let evidence = query_evidence(fact_ref.clone());

        let refs = fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections)
            .expect("fact query retention refs");

        assert!(refs.contains(&events::RetentionRef {
            artifact_id: evidence_artifact.evidence.artifact_id.clone(),
            role: events::ArtifactRole::FactQueryEvidence,
            content_digest: evidence_artifact.evidence.digest.clone(),
        }));
        assert!(refs.contains(&events::RetentionRef {
            artifact_id: descriptor_projection.descriptor_artifact_id.clone(),
            role: events::ArtifactRole::FactDescriptor,
            content_digest: fact_ref.fact_descriptor_hash().clone(),
        }));
        assert!(refs.contains(&events::RetentionRef {
            artifact_id: fact_ref.artifact_id().clone(),
            role: events::ArtifactRole::FactResponse,
            content_digest: fact_ref.response_hash().clone(),
        }));
    }

    #[test]
    fn fact_query_evidence_retention_refs_reject_invalid_returned_fact_authority() {
        for (name, descriptor, record, index, tampered, expected) in [
            (
                "missing descriptor",
                false,
                true,
                true,
                false,
                "missing descriptor authority",
            ),
            (
                "missing source fact",
                true,
                false,
                true,
                false,
                "missing source fact authority",
            ),
            (
                "missing indexed fact",
                true,
                true,
                false,
                false,
                "missing indexed fact authority",
            ),
            (
                "tampered subject",
                true,
                true,
                true,
                true,
                "source fact authority does not match",
            ),
        ] {
            let fact_ref = fact_ref();
            let evidence_artifact = query_evidence_artifact();
            let projections = if tampered {
                fact_authority_projections_with_tampered_subject(&fact_ref)
            } else {
                fact_authority_projections_without(&fact_ref, descriptor, record, index)
            };
            let evidence = query_evidence(fact_ref);

            let error = match fact_query_evidence_retention_refs(
                &evidence_artifact,
                &evidence,
                &projections,
            ) {
                Ok(_) => panic!("{name} authority should reject"),
                Err(error) => error,
            };

            assert!(
                matches!(&error, RuntimeError::InvalidRunnerOutput(message) if message.contains(expected)),
                "{name} returned {error:?}"
            );
        }
    }
}
