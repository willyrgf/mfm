use super::*;

/// Canonical JSON artifact prepared by a typed runner before runtime staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerJsonArtifact {
    pub(super) bytes: Vec<u8>,
    pub(super) evidence: store::ArtifactEvidenceRef,
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
    pub fn retention_ref(&self) -> Result<events::RetentionRef> {
        Ok(self.evidence.retention_ref()?)
    }

    /// Splits the artifact into canonical bytes and evidence.
    pub fn into_parts(self) -> (Vec<u8>, store::ArtifactEvidenceRef) {
        (self.bytes, self.evidence)
    }
}

/// Runner-owned input for recording one typed fact claim.
pub struct FactRecordInput<T: MfmFactType> {
    /// Typed fact value containing the subject and response material.
    pub(super) fact: T,
    /// Visibility selected for the recorded claim.
    pub(super) visibility: mfm_facts::FactVisibility,
    /// Optional source observation timestamp.
    pub(super) observed_at: Option<String>,
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

/// Loads and validates the certified config artifact for a specific certified node.
pub async fn load_runner_config_for_node<T>(
    node: &spec::NodeSpec,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let requirement = certified_config_requirement(node)?;
    let verified = read_retained_artifact(artifacts, &requirement).await?;
    let config = decode_verified_json::<T>(&verified)?;
    ValidatedConfig::new(config)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
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

/// Loads a side-effect artifact using the producer identity retained by the verified projection.
pub async fn load_side_effect_artifact<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> Result<(T, store::ArtifactEvidenceRef)>
where
    T: MfmValue + DeserializeOwned,
{
    let requirement = side_effect_artifact_requirement(artifact, role)?;
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
            evidence_hash,
        } => store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::StateOutput,
            artifact_id: artifact_id.clone(),
            evidence_hash: evidence_hash.clone(),
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
            evidence_hash,
        } => store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::SeedCell,
            artifact_id: artifact_id.clone(),
            evidence_hash: evidence_hash.clone(),
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

fn certified_config_requirement(node: &spec::NodeSpec) -> Result<store::EventArtifactRequirement> {
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: node.config_ref.artifact_id.clone(),
        digest: node.config_ref.digest.clone(),
        byte_len: node.config_ref.byte_len,
        media_type: node.config_ref.media_type.clone(),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    Ok(store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: node.config_ref.artifact_id.clone(),
        evidence_hash: evidence.evidence_hash()?,
        digest: Some(node.config_ref.digest.clone()),
        byte_len: Some(node.config_ref.byte_len),
        media_type: Some(node.config_ref.media_type.clone()),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    })
}

fn side_effect_artifact_requirement(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
) -> Result<store::EventArtifactRequirement> {
    Ok(store::EventArtifactRequirement {
        source: side_effect_artifact_source(role)?,
        artifact_id: artifact.artifact_id.clone(),
        evidence_hash: artifact.evidence_hash.clone(),
        digest: Some(artifact.content_digest.clone()),
        byte_len: None,
        media_type: None,
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: None,
        producer_node_id: Some(artifact.producer_node_id.clone()),
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
