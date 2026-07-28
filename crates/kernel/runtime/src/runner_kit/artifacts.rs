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

/// Precomputed executable identity material shared by every runner factory in one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableIdentityTemplate {
    binary_digest: ContentDigest,
}

impl ExecutableIdentityTemplate {
    /// Creates a process template from the canonical executable-bytes identity digest.
    pub fn new(binary_digest: ContentDigest) -> Self {
        Self { binary_digest }
    }

    /// Returns the canonical executable-bytes identity digest.
    pub fn binary_digest(&self) -> &ContentDigest {
        &self.binary_digest
    }

    /// Builds an executable identity for one logical runner or adapter factory.
    pub(crate) fn executable(
        &self,
        factory_id: events::RunnerFactoryId,
    ) -> events::ExecutableIdentity {
        events::ExecutableIdentity {
            factory_id,
            binary_digest: self.binary_digest.clone(),
        }
    }

    /// Builds a reusable factory binding for one logical runner or adapter factory.
    pub(crate) fn factory_binding(
        &self,
        factory_id: events::RunnerFactoryId,
    ) -> RunnerFactoryBinding {
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
pub fn load_runner_config_for_node<T>(
    ctx: &ErasedRunCtx<'_>,
    node: &spec::NodeSpec,
) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    ensure_certified_node(ctx.certified_node(&node.node_id), node)?;
    load_config_for_node(ctx.lifecycle(), node)
}

/// Loads and validates the certified config artifact for a pre-invocation state.
pub fn load_pre_invocation_runner_config_for_node<T>(
    ctx: &PreInvocationRunCtx<'_>,
    node: &spec::NodeSpec,
) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    ensure_certified_node(ctx.runtime_spec().node(&node.node_id), node)?;
    load_config_for_node(ctx.lifecycle(), node)
}

fn ensure_certified_node(
    certified: Option<&spec::NodeSpec>,
    requested: &spec::NodeSpec,
) -> Result<()> {
    if certified == Some(requested) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not part of the certified runtime spec",
            requested.node_id
        )))
    }
}

fn load_config_for_node<T>(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    node: &spec::NodeSpec,
) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let requirement = store::config_ref_artifact_requirement(&node.config_ref)?;
    let artifact = committed_object_for_requirement(lifecycle, &requirement)?;
    let config = decode_json_bytes::<T>(artifact.bytes())?;
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

/// Loads one materialized input node as a typed value from the verified run journal.
pub fn load_materialized_node_value<T>(
    ctx: &ErasedRunCtx<'_>,
    node: &MaterializedInputNode,
) -> Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    load_materialized_node_value_from_lifecycle(node, ctx.lifecycle())
}

/// Loads the root materialized input as a struct-like value.
pub fn load_materialized_struct_input<T>(
    ctx: &ErasedRunCtx<'_>,
    inputs: &MaterializedInputs,
) -> Result<T>
where
    T: DeserializeOwned,
{
    let value = materialized_input_node_json_from_lifecycle(&inputs.root, ctx.lifecycle())?;
    serde_json::from_value(value)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

/// Loads the root materialized pre-invocation input as a struct-like value.
pub fn load_pre_invocation_materialized_struct_input<T>(ctx: &PreInvocationRunCtx<'_>) -> Result<T>
where
    T: DeserializeOwned,
{
    let value = materialized_input_node_json_from_lifecycle(&ctx.inputs().root, ctx.lifecycle())?;
    serde_json::from_value(value)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

/// Loads the complete materialized input tree as its certified state-input type.
pub fn load_materialized_input<T>(ctx: &ErasedRunCtx<'_>) -> Result<T>
where
    T: StateInput + DeserializeOwned,
{
    let inputs = ctx.inputs();
    let expected_schema = T::input_schema_id()
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    if inputs.input_schema_id != expected_schema {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "materialized input schema {} did not match state input schema {}",
            inputs.input_schema_id, expected_schema
        )));
    }
    let value = materialized_input_node_json_from_lifecycle(&inputs.root, ctx.lifecycle())?;
    serde_json::from_value(value)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

/// Loads one field from a struct-shaped materialized input as a typed value.
pub fn load_materialized_struct_field_value<T>(
    ctx: &ErasedRunCtx<'_>,
    inputs: &MaterializedInputs,
    field_path: &str,
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
    load_materialized_node_value_from_lifecycle(&field.node, ctx.lifecycle())
}

/// Loads a non-empty vector materialized input from cell-backed elements.
pub fn load_non_empty_materialized_input<T>(
    ctx: &ErasedRunCtx<'_>,
    inputs: &MaterializedInputs,
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
        values.push(load_materialized_node_value_from_lifecycle(
            element,
            ctx.lifecycle(),
        )?);
    }
    NonEmpty::try_from_vec(values)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

/// Loads a side-effect artifact using exact evidence from the verified run journal.
pub fn load_side_effect_artifact<T>(
    ctx: &ErasedRunCtx<'_>,
    artifact: &store::current_lifecycle::CurrentArtifactProjectionRef<'_>,
    role: events::ArtifactRole,
) -> Result<(T, store::ArtifactEvidenceRef)>
where
    T: MfmValue + DeserializeOwned,
{
    let requirement = side_effect_artifact_requirement(artifact, role)?;
    let object = committed_object_for_requirement(ctx.lifecycle(), &requirement)?;
    let evidence = object.evidence().clone();
    let value = decode_json_bytes(object.bytes())?;
    Ok((value, evidence))
}

/// Materializes an input node tree into JSON from exact verified-journal objects.
pub fn materialized_input_node_json(
    ctx: &ErasedRunCtx<'_>,
    node: &MaterializedInputNode,
) -> Result<serde_json::Value> {
    materialized_input_node_json_from_lifecycle(node, ctx.lifecycle())
}

fn materialized_input_node_json_from_lifecycle(
    node: &MaterializedInputNode,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<serde_json::Value> {
    match node {
        MaterializedInputNode::Unit => Ok(serde_json::Value::Null),
        MaterializedInputNode::Cell(cell) => {
            let object = load_materialized_cell_artifact(cell, lifecycle)?;
            decode_json_bytes(object.bytes())
        }
        MaterializedInputNode::Tuple(elements) => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(materialized_input_node_json_from_lifecycle(
                    element, lifecycle,
                )?);
            }
            Ok(serde_json::Value::Array(values))
        }
        MaterializedInputNode::Struct(fields) => {
            let mut object = serde_json::Map::new();
            for field in fields {
                object.insert(
                    field.field_path.as_str().to_owned(),
                    materialized_input_node_json_from_lifecycle(&field.node, lifecycle)?,
                );
            }
            Ok(serde_json::Value::Object(object))
        }
        MaterializedInputNode::Vec(elements) | MaterializedInputNode::NonEmptyVec(elements) => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(materialized_input_node_json_from_lifecycle(
                    element, lifecycle,
                )?);
            }
            Ok(serde_json::Value::Array(values))
        }
    }
}

fn load_materialized_node_value_from_lifecycle<T>(
    node: &MaterializedInputNode,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::Cell(cell) = node else {
        return Err(RuntimeError::InvalidRunnerOutput(
            "materialized input node was not a cell".to_owned(),
        ));
    };
    let object = load_materialized_cell_artifact(cell, lifecycle)?;
    decode_json_bytes(object.bytes())
}

fn load_materialized_cell_artifact<'view>(
    cell: &MaterializedCell,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'view>,
) -> Result<store::current_lifecycle::CurrentObjectRef<'view>> {
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
        } => {
            let admitted = lifecycle.seed(&cell.cell_id)?.ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "materialized seed cell {} is not committed in the run journal",
                    cell.cell_id
                ))
            })?;
            let admitted = admitted.evidence();
            if &admitted.seed_id != seed_id
                || &admitted.seed_artifact.artifact_id != artifact_id
                || &admitted.seed_artifact.content_digest != content_digest
                || &admitted.seed_artifact.evidence_hash != evidence_hash
                || admitted.schema_id != cell.schema_id
                || admitted.semantic_type_id != cell.semantic_type_id
            {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "materialized seed cell {} does not match admitted evidence",
                    cell.cell_id
                )));
            }
            store::seed_cell_artifact_requirement(admitted)
        }
        MaterializedCellTerminal::Skipped { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(
                "materialized input cell was skipped".to_owned(),
            ));
        }
    };
    committed_object_for_requirement(lifecycle, &requirement)
}

fn committed_object_for_requirement<'view>(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'view>,
    requirement: &store::EventArtifactRequirement,
) -> Result<store::current_lifecycle::CurrentObjectRef<'view>> {
    let mut committed_requirement = None;
    let _ = lifecycle.visit_records(|record| {
        record.visit_artifact_requirements(|candidate| {
            if candidate == requirement {
                committed_requirement = Some(candidate.clone());
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        })
    });
    let committed_requirement = committed_requirement.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "artifact {} does not have exact committed requirement authority",
            requirement.artifact_id
        ))
    })?;
    lifecycle
        .object_for_requirement(&committed_requirement)
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "artifact {} lacks exact retained-object authority",
                requirement.artifact_id
            ))
        })
}

fn decode_json_bytes<T>(bytes: &[u8]) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn side_effect_artifact_requirement(
    artifact: &store::current_lifecycle::CurrentArtifactProjectionRef<'_>,
    role: events::ArtifactRole,
) -> Result<store::EventArtifactRequirement> {
    Ok(store::EventArtifactRequirement {
        source: side_effect_artifact_source(role)?,
        artifact_id: artifact.artifact_id().clone(),
        evidence_hash: artifact.evidence_hash().clone(),
        digest: Some(artifact.content_digest().clone()),
        byte_len: None,
        media_type: None,
        schema_id: artifact.schema_id().cloned(),
        semantic_type_id: None,
        producer_node_id: Some(artifact.producer_node_id().clone()),
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
