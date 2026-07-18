use super::*;

/// Persisted typed scope specification emitted by the program builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSpec {
    /// Stable scope author key.
    pub key: ScopeKey,
    /// Derived scope id.
    pub scope_id: ScopeId,
    /// Parent scope id for child scopes.
    pub parent_scope_id: Option<ScopeId>,
    /// Operation lineage active when this scope id was derived.
    pub planning_lineage: OperationLineage,
}

/// Canonical config reference embedded in a typed state node draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigBindingSpec {
    /// Config schema id.
    pub schema_id: SchemaId,
    /// Canonical config bytes.
    pub canonical_json: PlainCanonicalJsonBytes,
    /// Canonical config content digest.
    pub content_digest: ContentDigest,
    /// Canonical config reference digest.
    pub config_ref_digest: ContentDigest,
    /// Canonical byte length.
    pub byte_len: usize,
}

/// Persisted typed state node draft emitted by the program builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateNodeSpec {
    /// Derived state node id.
    pub node_id: NodeId,
    /// Stable state author key.
    pub key: StateKey,
    /// Owning scope id.
    pub scope_id: ScopeId,
    /// Registered state kind.
    pub state_kind: StateKind,
    /// Registered state version.
    pub state_version: StateVersion,
    /// Registered state descriptor id.
    pub state_descriptor_id: DescriptorId,
    /// Stable registered state descriptor name.
    pub state_descriptor_name: String,
    /// Registered state context descriptor contract.
    pub context_descriptor: StateContextDescriptorSpec,
    /// Registered state input context contract.
    pub input_context_contract: StateInputContextContractSpec,
    /// Registered state output context contract.
    pub output_context_contract: StateOutputContextContractSpec,
    /// Certified transition-context requirement for this node.
    pub context: NodeContextSpec,
    /// Registered runner kind.
    pub runner: RunnerKind,
    /// Framework effect kind required by the registered state.
    pub effect_kind: EffectKind,
    /// Framework capability descriptor set required by the registered state.
    pub capability_bindings: CapabilitySetDescriptor,
    /// Behaviorally relevant adapter bindings required by the registered state.
    pub adapter_bindings: Vec<AdapterBindingSpec>,
    /// Fact descriptors this producing node may emit.
    pub fact_descriptor_allowlist: Vec<FactDescriptorRef>,
    /// Optional hash-defining effect contract from the registered descriptor.
    pub effect_contract_digest: Option<ContentDigest>,
    /// Cross-run resource claim when this node mutates an external system.
    pub side_effect_resource_claim: Option<ResourceClaimSpec>,
    /// Terminal verification policy when this node mutates an external system.
    pub side_effect_verification: Option<SideEffectVerificationSpec>,
    /// Forward side-effect verify framework node lowered for this submit node.
    pub side_effect_verify: Option<SideEffectVerifyDraftSpec>,
    /// Canonical config binding.
    pub config: ConfigBindingSpec,
    /// Typed input binding.
    pub input: InputBindingSpec,
    /// Output cell id.
    pub output_cell_id: CellId,
    /// Output schema id.
    pub output_schema_id: SchemaId,
    /// Output semantic type id.
    pub output_semantic_type_id: SemanticTypeId,
    /// Output value lineage ref.
    pub output_value_lineage: ValueLineageRef,
    /// Certified transition-context constraint for the output cell.
    pub output_context: CellContextSpec,
    /// Stable domain keys associated with the output value lineage.
    pub output_domain_keys: Vec<StableDomainKeyRef>,
    /// Planning lineage active while this node was emitted.
    pub planning_lineage: OperationLineage,
}

/// Draft metadata for a framework-owned side-effect verify node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectVerifyDraftSpec {
    /// Certified side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Derived verify framework node id.
    pub node_id: NodeId,
    /// Verify framework output cell id.
    pub output_cell_id: CellId,
    /// Verify framework output value lineage ref.
    pub output_value_lineage: ValueLineageRef,
    /// Certified transition-context constraint for the verify output cell.
    pub output_context: CellContextSpec,
    /// Stable domain keys associated with the verify output value lineage.
    pub output_domain_keys: Vec<StableDomainKeyRef>,
}

/// Operation lineage frame emitted by a registry-mediated call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationLineageFrameSpec {
    /// Derived operation instance id.
    pub operation_instance_id: OperationInstanceId,
    /// Stable operation author key.
    pub key: OperationKey,
    /// Owning scope id.
    pub scope_id: ScopeId,
    /// Registered operation kind.
    pub operation_kind: OperationKind,
    /// Registered operation version.
    pub operation_version: OperationVersion,
    /// Registered operation descriptor id.
    pub operation_descriptor_id: DescriptorId,
    /// Stable registered operation descriptor name.
    pub operation_name: String,
    /// Deterministic expansion ABI recorded by the descriptor.
    pub expansion_abi: &'static str,
    /// Operation lineage active before this operation expanded.
    pub parent_operation_lineage: OperationLineage,
    /// Canonical operation config binding.
    pub config: ConfigBindingSpec,
    /// Typed operation input binding.
    pub input: OperationInputBindingSpec,
    /// Operation output schema id.
    pub output_schema_id: SchemaId,
    /// Output handles actually returned by expansion.
    pub output_handles: Vec<TypedHandleRef>,
    /// Digest of this lineage frame.
    pub lineage_digest: ContentDigest,
}

/// Framework bridge direction for same-value cross-scope movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeKind {
    /// Parent value imported into a child scope.
    ImportFromParent,
    /// Child value exported into the parent scope.
    ExportToParent,
}

impl BridgeKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ImportFromParent => "import-from-parent",
            Self::ExportToParent => "export-to-parent",
        }
    }
}

/// Bridge policy supported by typed program v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgePolicy {
    /// Same-run, same-value movement with no semantic transform.
    SameRunSameValueV1,
}

impl BridgePolicy {
    /// Returns the v1 same-run same-value bridge policy.
    pub const fn same_run_same_value() -> Self {
        Self::SameRunSameValueV1
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SameRunSameValueV1 => "same-run-same-value-v1",
        }
    }
}

/// Framework provenance for an emitted bridge node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeProvenance {
    /// Bridge emitted by the typed kernel child-scope builder.
    FrameworkChildScopeV1,
}

/// Persisted bridge reference. This is audit evidence, not live authority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BridgeRef {
    /// Source scope id.
    pub source_scope_id: ScopeId,
    /// Target scope id.
    pub target_scope_id: ScopeId,
    /// Source cell id.
    pub source_cell_id: CellId,
    /// Target cell id created by the bridge.
    pub target_cell_id: CellId,
    /// Value semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Value schema id.
    pub schema_id: SchemaId,
    /// Framework bridge node id.
    pub bridge_node_id: NodeId,
}

/// Bridge node specification emitted into the typed program draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeNodeSpec {
    /// Derived bridge node id.
    pub node_id: NodeId,
    /// Stable bridge author key.
    pub key: BridgeKey,
    /// Source scope id.
    pub source_scope_id: ScopeId,
    /// Target scope id.
    pub target_scope_id: ScopeId,
    /// Source cell id.
    pub source_cell_id: CellId,
    /// Target cell id.
    pub target_cell_id: CellId,
    /// Target value lineage ref.
    pub target_value_lineage: ValueLineageRef,
    /// Value semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Value schema id.
    pub schema_id: SchemaId,
    /// Certified transition-context constraint preserved by the same-value bridge.
    pub context: CellContextSpec,
    /// Bridge direction.
    pub bridge_kind: BridgeKind,
    /// Same-value bridge policy.
    pub policy: BridgePolicy,
    /// Framework provenance.
    pub provenance: BridgeProvenance,
    /// Planning lineage active while this bridge was emitted.
    pub planning_lineage: OperationLineage,
}

impl BridgeNodeSpec {
    /// Returns the persisted bridge reference for this emitted node.
    pub fn bridge_ref(&self) -> BridgeRef {
        BridgeRef {
            source_scope_id: self.source_scope_id.clone(),
            target_scope_id: self.target_scope_id.clone(),
            source_cell_id: self.source_cell_id.clone(),
            target_cell_id: self.target_cell_id.clone(),
            semantic_type_id: self.semantic_type_id.clone(),
            schema_id: self.schema_id.clone(),
            bridge_node_id: self.node_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct BridgeSessionToken(pub(super) DigestBytes);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BridgeEvidenceCore {
    pub(super) bridge_ref: BridgeRef,
    pub(super) session_token: BridgeSessionToken,
}

/// Live bridge authority owned by one active child-scope builder invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvidence<'program, 'parent> {
    pub(super) core: BridgeEvidenceCore,
    pub(super) _program: PhantomData<fn(&'program ()) -> &'program ()>,
    pub(super) _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
    pub(super) _private: (),
}

impl<'program, 'parent> BridgeEvidence<'program, 'parent> {
    /// Returns the persisted bridge reference carried by this live evidence.
    pub fn bridge_ref(&self) -> &BridgeRef {
        &self.core.bridge_ref
    }
}

/// Public-output cell binding specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputCellSpec {
    /// Stable public field path.
    public_field_path: PublicFieldPath,
    /// Bound typed cell reference.
    cell: TypedHandleRef,
}

impl PublicOutputCellSpec {
    /// Creates a public-output cell binding from a branded typed handle.
    pub fn from_handle<'program, 'scope, T: MfmValue>(
        public_field_path: PublicFieldPath,
        handle: &Handle<'program, 'scope, T>,
    ) -> Self {
        Self {
            public_field_path,
            cell: handle.typed_ref(),
        }
    }

    /// Returns the public field path.
    pub fn public_field_path(&self) -> &PublicFieldPath {
        &self.public_field_path
    }

    /// Returns the bound typed cell reference.
    pub fn cell(&self) -> &TypedHandleRef {
        &self.cell
    }
}

/// Public-output binding specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputSpec {
    /// Stable public-output binding key.
    pub(super) key: PublicOutputKey,
    /// Public output schema id.
    pub(super) public_schema_id: SchemaId,
    /// Bound output cells.
    pub(super) outputs: Vec<PublicOutputCellSpec>,
}

impl PublicOutputSpec {
    /// Returns the stable public-output binding key.
    pub fn key(&self) -> &PublicOutputKey {
        &self.key
    }

    /// Returns the public output schema id.
    pub fn public_schema_id(&self) -> &SchemaId {
        &self.public_schema_id
    }

    /// Returns bound output cells.
    pub fn outputs(&self) -> &[PublicOutputCellSpec] {
        &self.outputs
    }
}

/// Root-bound public output evidence returned by `RootBuilder::bind_public_outputs`.
#[derive(Debug, PartialEq, Eq)]
pub struct RootBound<'program, 'scope> {
    pub(super) public_output_spec: PublicOutputSpec,
    pub(super) _program: PhantomData<fn(&'program ()) -> &'program ()>,
    pub(super) _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    pub(super) _private: (),
}

/// Parent-visible value returned from a child scope after bridge validation.
#[derive(Debug, PartialEq, Eq)]
pub struct Bridged<'program, 'parent, R> {
    pub(super) value: R,
    pub(super) bridge_evidence: Vec<BridgeEvidence<'program, 'parent>>,
    pub(super) _program: PhantomData<fn(&'program ()) -> &'program ()>,
    pub(super) _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
    pub(super) _private: (),
}

/// Framework-owned trait for values that may leave a child scope.
pub trait BridgeableToParent<'program, 'parent>: private::BridgeableSealed {
    /// Returns live bridge evidence that must validate against the active child session.
    fn bridge_evidence(&self) -> Vec<BridgeEvidence<'program, 'parent>>;
}

impl<'program, 'parent, T> BridgeableToParent<'program, 'parent> for Handle<'program, 'parent, T>
where
    T: MfmValue,
{
    fn bridge_evidence(&self) -> Vec<BridgeEvidence<'program, 'parent>> {
        self.origin.bridge_evidence()
    }
}

impl<'program, 'parent> BridgeableToParent<'program, 'parent> for () {
    fn bridge_evidence(&self) -> Vec<BridgeEvidence<'program, 'parent>> {
        Vec::new()
    }
}

macro_rules! impl_bridgeable_tuple {
    ($($name:ident),+ $(,)?) => {
        impl<'program, 'parent, $($name),+> BridgeableToParent<'program, 'parent>
            for ($($name,)+)
        where
            $($name: BridgeableToParent<'program, 'parent>,)+
        {
            fn bridge_evidence(&self) -> Vec<BridgeEvidence<'program, 'parent>> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                let mut output = Vec::new();
                $(output.extend($name.bridge_evidence());)+
                output
            }
        }
    };
}

impl_bridgeable_tuple!(A);
impl_bridgeable_tuple!(A, B);
impl_bridgeable_tuple!(A, B, C);
impl_bridgeable_tuple!(A, B, C, D);

/// Unbranded typed program draft produced by [`build_root`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedProgramDraft {
    pub(super) root_key: ScopeKey,
    pub(super) root_scope_id: ScopeId,
    pub(super) saga_policy: SagaPolicy,
    pub(super) contexts: Vec<CertifiedContextSpec>,
    pub(super) context_validators: Vec<ContextValidatorSpec>,
    pub(super) seeds: Vec<RootSeedSpec>,
    pub(super) scopes: Vec<ScopeSpec>,
    pub(super) state_nodes: Vec<StateNodeSpec>,
    pub(super) remediation_nodes: BTreeMap<NodeId, StateNodeSpec>,
    pub(super) operation_lineage: Vec<OperationLineageFrameSpec>,
    pub(super) bridge_nodes: Vec<BridgeNodeSpec>,
    pub(super) public_output_spec: PublicOutputSpec,
}

impl TypedProgramDraft {
    /// Returns the root scope key.
    pub fn root_key(&self) -> &ScopeKey {
        &self.root_key
    }

    /// Returns the root scope id.
    pub fn root_scope_id(&self) -> &ScopeId {
        &self.root_scope_id
    }

    /// Returns the run-level saga policy for this draft.
    pub fn saga_policy(&self) -> &SagaPolicy {
        &self.saga_policy
    }

    /// Returns certified transition contexts declared by the draft.
    pub fn contexts(&self) -> &[CertifiedContextSpec] {
        &self.contexts
    }

    /// Returns typed context validators retained by this in-memory draft.
    pub fn context_validators(&self) -> &[ContextValidatorSpec] {
        &self.context_validators
    }

    /// Returns root seed specs.
    pub fn seeds(&self) -> &[RootSeedSpec] {
        &self.seeds
    }

    /// Returns emitted scope specs, including the root scope.
    pub fn scopes(&self) -> &[ScopeSpec] {
        &self.scopes
    }

    /// Returns emitted typed state nodes.
    pub fn state_nodes(&self) -> &[StateNodeSpec] {
        &self.state_nodes
    }

    /// Returns remediation nodes keyed by the forward side-effect node they compensate.
    pub fn remediation_nodes(&self) -> &BTreeMap<NodeId, StateNodeSpec> {
        &self.remediation_nodes
    }

    /// Returns registry-mediated operation lineage frames.
    pub fn operation_lineage(&self) -> &[OperationLineageFrameSpec] {
        &self.operation_lineage
    }

    /// Returns emitted framework bridge nodes.
    pub fn bridge_nodes(&self) -> &[BridgeNodeSpec] {
        &self.bridge_nodes
    }

    /// Returns the public-output spec bound at the root.
    pub fn public_output_spec(&self) -> &PublicOutputSpec {
        &self.public_output_spec
    }

    /// Validates that a persisted bridge ref is backed by an emitted bridge node.
    pub fn validate_bridge_ref_for_certification(
        &self,
        bridge_ref: &BridgeRef,
    ) -> Result<&BridgeNodeSpec> {
        self.bridge_nodes
            .iter()
            .find(|node| node.bridge_ref() == *bridge_ref)
            .ok_or(PlanError::UnknownBridgeRef)
    }

    /// Validates same-scope same-type lineage equality for certification fixtures.
    pub fn validate_same_scope_same_type_lineage_for_certification(
        &self,
        expected: &TypedHandleRef,
        actual: &TypedHandleRef,
    ) -> Result<()> {
        if expected.scope_id == actual.scope_id
            && expected.schema_id == actual.schema_id
            && expected.semantic_type_id == actual.semantic_type_id
            && expected.value_lineage != actual.value_lineage
        {
            return Err(PlanError::LineageMismatch(format!(
                "scope={} schema={} semantic={} expected={} actual={}",
                expected.scope_id.as_str(),
                expected.schema_id.as_str(),
                expected.semantic_type_id.as_str(),
                expected.value_lineage.digest().as_str(),
                actual.value_lineage.digest().as_str()
            )));
        }
        Ok(())
    }
}

pub(super) fn validate_state_context_binding(
    descriptor: &StateDescriptorIdentity,
    context: Option<&CertifiedContextSpec>,
) -> Result<()> {
    match (descriptor.context(), context) {
        (StateContextDescriptorSpec::NoContext, None) => Ok(()),
        (StateContextDescriptorSpec::NoContext, Some(context)) => {
            Err(PlanError::ContextContract(format!(
                "state {} is NoContext but was planned with {}",
                descriptor.name(),
                context.context_ref
            )))
        }
        (StateContextDescriptorSpec::Required(_), None) => Err(PlanError::ContextContract(
            format!("state {} requires a certified context", descriptor.name()),
        )),
        (StateContextDescriptorSpec::Required(requirement), Some(context)) => {
            if context.context_descriptor_id != requirement.context_descriptor_id
                || context.schema_id != requirement.schema_id
                || context.semantic_type_id != requirement.semantic_type_id
                || context.canonicalizer_identity != requirement.canonicalizer_identity
            {
                return Err(PlanError::ContextContract(format!(
                    "state {} context descriptor mismatch for {}",
                    descriptor.name(),
                    context.context_ref
                )));
            }
            Ok(())
        }
    }
}

pub(super) fn output_context_from_contract(
    descriptor: &StateDescriptorIdentity,
    context: Option<&CertifiedContextSpec>,
) -> Result<CellContextSpec> {
    match descriptor.output_context() {
        StateOutputContextContractSpec::NoContext => Ok(CellContextSpec::NoContext),
        StateOutputContextContractSpec::Produces {
            resource_kind,
            stage,
        } => {
            let context = context.ok_or_else(|| {
                PlanError::ContextContract(format!(
                    "state {} produces a context-bound resource without context",
                    descriptor.name()
                ))
            })?;
            Ok(CellContextSpec::Bound {
                context_ref: context.context_ref.clone(),
                resource_kind: resource_kind.clone(),
                stage: stage.clone(),
                producer: Box::new(ContextProducerSpec {
                    producer_descriptor_ids: vec![descriptor.descriptor_id().clone()],
                    seed_producers_allowed: false,
                }),
            })
        }
    }
}

/// Pre-certification typed program plus launch material produced by deterministic planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedProgramLaunchPlan {
    /// Typed program draft to be lowered and certified by app assembly.
    pub draft: TypedProgramDraft,
    /// Canonical non-secret config material required by the draft.
    pub config_material: Vec<TypedProgramConfigMaterial>,
    /// Canonical non-secret seed material required by the draft.
    pub seed_material: Vec<TypedProgramSeedMaterial>,
}

impl TypedProgramLaunchPlan {
    /// Builds a launch plan from a draft that does not require launch seed material.
    pub fn from_draft(draft: TypedProgramDraft) -> Result<Self> {
        Self::from_draft_and_seed_material(draft, BTreeMap::new())
    }

    /// Builds a launch plan from a draft and canonical seed material keyed by seed id.
    ///
    /// Seed material is matched by [`SeedId`] and returned in draft seed order so callers do not
    /// depend on positional seed input.
    pub fn from_draft_and_seed_material(
        draft: TypedProgramDraft,
        seeds: BTreeMap<SeedId, PlainCanonicalJsonBytes>,
    ) -> Result<Self> {
        let config_material = config_material_for_draft(&draft)?;
        let seed_material = seed_material_for_draft(&draft, seeds)?;
        Ok(Self {
            draft,
            config_material,
            seed_material,
        })
    }
}

/// Canonical non-secret config material required to launch a typed program draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedProgramConfigMaterial {
    /// Config schema id consumed by the typed program.
    pub schema_id: SchemaId,
    /// Canonical JSON config bytes.
    pub bytes: PlainCanonicalJsonBytes,
    /// Media type for the canonical config bytes.
    pub media_type: MediaType,
}

/// Canonical non-secret seed material required to launch a typed program draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedProgramSeedMaterial {
    /// Seed id consumed by the typed program.
    pub seed_id: SeedId,
    /// Canonical JSON seed bytes.
    pub bytes: PlainCanonicalJsonBytes,
    /// Media type for the canonical seed bytes.
    pub media_type: MediaType,
}

fn config_material_for_draft(draft: &TypedProgramDraft) -> Result<Vec<TypedProgramConfigMaterial>> {
    let media_type = typed_program_launch_json_media_type()?;
    Ok(draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        .map(|config| TypedProgramConfigMaterial {
            schema_id: config.schema_id.clone(),
            bytes: config.canonical_json.clone(),
            media_type: media_type.clone(),
        })
        .collect())
}

fn seed_material_for_draft(
    draft: &TypedProgramDraft,
    mut seeds: BTreeMap<SeedId, PlainCanonicalJsonBytes>,
) -> Result<Vec<TypedProgramSeedMaterial>> {
    let media_type = typed_program_launch_json_media_type()?;
    let mut material = Vec::with_capacity(draft.seeds().len());
    for seed in draft.seeds() {
        let bytes = seeds.remove(&seed.seed_id).ok_or_else(|| {
            PlanError::Key(format!(
                "missing entry-point seed material for {}",
                seed.seed_id
            ))
        })?;
        let digest = bytes.content_digest();
        let byte_len = bytes.as_bytes().len();
        if digest != seed.content_digest || byte_len != seed.byte_len {
            return Err(PlanError::Canonical(format!(
                "entry-point seed material did not match draft seed {}",
                seed.seed_id
            )));
        }
        material.push(TypedProgramSeedMaterial {
            seed_id: seed.seed_id.clone(),
            bytes,
            media_type: media_type.clone(),
        });
    }
    if let Some(seed_id) = seeds.keys().next() {
        return Err(PlanError::Key(format!(
            "unknown entry-point seed material for {seed_id}"
        )));
    }
    Ok(material)
}

fn typed_program_launch_json_media_type() -> Result<MediaType> {
    MediaType::new("application/json").map_err(|error| PlanError::Key(error.to_string()))
}
