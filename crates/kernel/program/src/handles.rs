use super::*;

/// Producer of a typed cell in value-lineage evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CellProducer {
    /// Cell produced by a state or framework node.
    Node(NodeId),
    /// Cell produced by a launch seed.
    Seed(SeedId),
}

/// Policy describing how a value lineage relates to its inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LineageTransformPolicy {
    /// Source value with no same-type input dependency.
    Source,
    /// State output derived from declared input cells and config.
    StateOutput,
    /// Same-run same-value bridge.
    SameValueBridge,
}

impl LineageTransformPolicy {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::StateOutput => "state_output",
            Self::SameValueBridge => "same_value_bridge",
        }
    }
}

/// Canonical reference to a stable domain key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableDomainKeyRef {
    /// Domain-key schema id.
    pub schema_id: SchemaId,
    /// Canonical domain-key content digest.
    pub content_digest: ContentDigest,
}

impl StableDomainKeyRef {
    /// Builds a stable domain-key reference from a typed key.
    pub fn from_key<K: StableDomainKey>(key: &K) -> Result<Self> {
        let schema_id = K::domain_key_descriptor()
            .and_then(|descriptor| descriptor.schema_id())
            .map_err(|error| PlanError::Value(error.to_string()))?;
        let content_digest = canonical_domain_key_bytes(key)?.content_digest();
        Ok(Self {
            schema_id,
            content_digest,
        })
    }

    pub(super) fn stable_sort_key(&self) -> String {
        format!(
            "{}:{}",
            self.schema_id.as_str(),
            self.content_digest.as_str()
        )
    }
}

fn canonical_domain_key_bytes<K: StableDomainKey>(key: &K) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(key).map_err(|error| PlanError::Serialize(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))
}

pub(super) fn stable_domain_key_refs<K: StableDomainKey>(
    keys: Vec<K>,
) -> Result<Vec<StableDomainKeyRef>> {
    let mut refs = keys
        .iter()
        .map(StableDomainKeyRef::from_key)
        .collect::<Result<Vec<_>>>()?;
    refs.sort();
    let mut seen = BTreeSet::new();
    for key_ref in &refs {
        if !seen.insert(key_ref.clone()) {
            return Err(PlanError::DuplicateDomainKey(key_ref.stable_sort_key()));
        }
    }
    Ok(refs)
}

/// Operation-lineage digest sequence visible to value-lineage records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationLineage {
    /// Operation instance ids from outermost to innermost active expansion.
    pub active_instances: Vec<OperationInstanceId>,
    /// Completed operation lineage frame digests already recorded in this scope.
    pub completed_frames: Vec<ContentDigest>,
    /// Digest of this lineage sequence.
    pub digest: ContentDigest,
}

impl OperationLineage {
    pub(super) fn empty() -> Result<Self> {
        Self::from_parts(Vec::new(), Vec::new())
    }

    pub(super) fn from_parts(
        active_instances: Vec<OperationInstanceId>,
        completed_frames: Vec<ContentDigest>,
    ) -> Result<Self> {
        let digest = canonical_digest(serde_json::json!({
            "active_instances": active_instances
                .iter()
                .map(OperationInstanceId::as_str)
                .collect::<Vec<_>>(),
            "completed_frames": completed_frames
                .iter()
                .map(ContentDigest::as_str)
                .collect::<Vec<_>>(),
        }))?;
        Ok(Self {
            active_instances,
            completed_frames,
            digest,
        })
    }
}

/// Hash-defining value-lineage evidence for a typed cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueLineage {
    /// Scope containing this value.
    pub scope_id: ScopeId,
    /// Producer for this value.
    pub producer: CellProducer,
    /// Input cells used to produce this value.
    pub input_cells: Vec<CellId>,
    /// Config reference digest used by the producer, when any.
    pub config_ref_digest: Option<ContentDigest>,
    /// Operation lineage active when this value was produced.
    pub operation_lineage: OperationLineage,
    /// Stable domain keys associated with this value.
    pub domain_keys: Vec<StableDomainKeyRef>,
    /// Lineage transform policy.
    pub transform_policy: LineageTransformPolicy,
}

/// Reference to a value-lineage record used by input bindings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueLineageRef {
    /// Digest identifying the value-lineage record.
    digest: ContentDigest,
}

impl ValueLineageRef {
    /// Creates a lineage reference from an already typed digest.
    pub fn new(digest: ContentDigest) -> Self {
        Self { digest }
    }

    /// Returns the lineage digest.
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// Non-forgeable typed reference to a planned cell.
#[derive(Debug, PartialEq, Eq)]
pub struct Handle<'program, 'scope, T: MfmValue> {
    cell_id: CellId,
    scope_id: ScopeId,
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    value_lineage: ValueLineageRef,
    context: CellContextSpec,
    pub(super) origin: HandleOrigin,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    _value: PhantomData<fn(T) -> T>,
}

impl<'program, 'scope, T: MfmValue> Clone for Handle<'program, 'scope, T> {
    fn clone(&self) -> Self {
        Self {
            cell_id: self.cell_id.clone(),
            scope_id: self.scope_id.clone(),
            schema_id: self.schema_id.clone(),
            semantic_type_id: self.semantic_type_id.clone(),
            value_lineage: self.value_lineage.clone(),
            context: self.context.clone(),
            origin: self.origin.clone(),
            _program: PhantomData,
            _scope: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<'program, 'scope, T: MfmValue> Handle<'program, 'scope, T> {
    pub(super) fn new(
        cell_id: CellId,
        scope_id: ScopeId,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        value_lineage: ValueLineageRef,
        context: CellContextSpec,
    ) -> Self {
        Self {
            cell_id,
            scope_id,
            schema_id,
            semantic_type_id,
            value_lineage,
            context,
            origin: HandleOrigin::Local,
            _program: PhantomData,
            _scope: PhantomData,
            _value: PhantomData,
        }
    }

    pub(super) fn new_bridge(
        cell_id: CellId,
        scope_id: ScopeId,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        value_lineage: ValueLineageRef,
        context: CellContextSpec,
        evidence: BridgeEvidenceCore,
    ) -> Self {
        Self {
            cell_id,
            scope_id,
            schema_id,
            semantic_type_id,
            value_lineage,
            context,
            origin: HandleOrigin::Bridge {
                evidence: Box::new(evidence),
            },
            _program: PhantomData,
            _scope: PhantomData,
            _value: PhantomData,
        }
    }

    /// Returns an unbranded typed handle reference for descriptors.
    pub fn typed_ref(&self) -> TypedHandleRef {
        TypedHandleRef {
            cell_id: self.cell_id.clone(),
            scope_id: self.scope_id.clone(),
            schema_id: self.schema_id.clone(),
            semantic_type_id: self.semantic_type_id.clone(),
            value_lineage: self.value_lineage.clone(),
            context: self.context.clone(),
        }
    }
}

/// Branded output handle for a forward side-effect node with linked remediation.
#[derive(Debug, PartialEq, Eq)]
pub struct ForwardSideEffectHandle<'program, 'scope, T: MfmValue> {
    node_id: NodeId,
    handle: Handle<'program, 'scope, T>,
}

impl<'program, 'scope, T: MfmValue> Clone for ForwardSideEffectHandle<'program, 'scope, T> {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id.clone(),
            handle: self.handle.clone(),
        }
    }
}

impl<'program, 'scope, T: MfmValue> ForwardSideEffectHandle<'program, 'scope, T> {
    pub(super) fn new(node_id: NodeId, handle: Handle<'program, 'scope, T>) -> Self {
        Self { node_id, handle }
    }

    /// Returns the linked forward side-effect node id.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns this forward output as an ordinary typed handle reference.
    pub fn handle(&self) -> &Handle<'program, 'scope, T> {
        &self.handle
    }

    /// Converts this branded forward output into its ordinary typed handle.
    pub fn into_handle(self) -> Handle<'program, 'scope, T> {
        self.handle
    }

    /// Returns an unbranded typed handle reference for descriptors.
    pub fn typed_ref(&self) -> TypedHandleRef {
        self.handle.typed_ref()
    }
}

/// Branded handle for a remediation node.
///
/// This intentionally does not implement state-input conversion: remediation outputs are outside
/// the forward graph and cannot be scheduled by ordinary forward authoring APIs.
#[derive(Debug, PartialEq, Eq)]
pub struct RemediationHandle<'program, 'scope, T: MfmValue> {
    node_id: NodeId,
    handle: Handle<'program, 'scope, T>,
}

impl<'program, 'scope, T: MfmValue> Clone for RemediationHandle<'program, 'scope, T> {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id.clone(),
            handle: self.handle.clone(),
        }
    }
}

impl<'program, 'scope, T: MfmValue> RemediationHandle<'program, 'scope, T> {
    pub(super) fn new(node_id: NodeId, handle: Handle<'program, 'scope, T>) -> Self {
        Self { node_id, handle }
    }

    /// Returns the remediation node id.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns an unbranded typed handle reference for audit and tests.
    pub fn typed_ref(&self) -> TypedHandleRef {
        self.handle.typed_ref()
    }
}

/// Branded forward/remediation handles returned by linked side-effect construction.
pub type LinkedSideEffectHandles<'program, 'scope, ForwardOutput, RemediationOutput> = (
    ForwardSideEffectHandle<'program, 'scope, ForwardOutput>,
    RemediationHandle<'program, 'scope, RemediationOutput>,
);

/// Parameters for a forward side-effect node in a linked compensation pair.
pub struct SideEffectNodeParams<S: SideEffectState, I> {
    /// Scope-local author key for the forward node.
    pub key: StateKey,
    /// Deterministic forward state config.
    pub config: S::Config,
    /// Forward state input binding source.
    pub input: I,
    /// Resource claim for the forward side-effect ledger.
    pub resource_claim: ResourceClaim,
    /// Terminal verification policy for the forward side-effect ledger.
    pub verification: SideEffectVerificationSpec,
}

/// Parameters for a remediation node in a linked compensation pair.
pub struct RemediationNodeParams<R: SideEffectState> {
    /// Scope-local author key for the remediation node.
    pub key: StateKey,
    /// Deterministic remediation state config.
    pub config: R::Config,
    /// Resource claim for the remediation side-effect ledger.
    pub resource_claim: ResourceClaim,
    /// Terminal verification policy for the remediation side-effect ledger.
    pub verification: SideEffectVerificationSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HandleOrigin {
    Local,
    Bridge { evidence: Box<BridgeEvidenceCore> },
}

impl HandleOrigin {
    pub(super) fn bridge_evidence<'program, 'parent>(
        &self,
    ) -> Vec<BridgeEvidence<'program, 'parent>> {
        match self {
            Self::Local => Vec::new(),
            Self::Bridge { evidence } => vec![BridgeEvidence {
                core: (**evidence).clone(),
                _program: PhantomData,
                _parent: PhantomData,
                _private: (),
            }],
        }
    }
}

/// Unbranded typed handle reference emitted into draft specs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedHandleRef {
    /// Planned cell id.
    pub(super) cell_id: CellId,
    /// Planned scope id.
    pub(super) scope_id: ScopeId,
    /// Value schema id.
    pub(super) schema_id: SchemaId,
    /// Value semantic type id.
    pub(super) semantic_type_id: SemanticTypeId,
    /// Value lineage reference.
    pub(super) value_lineage: ValueLineageRef,
    /// Certified context constraint carried by the planned cell.
    pub(super) context: CellContextSpec,
}

impl TypedHandleRef {
    /// Returns the planned cell id.
    pub fn cell_id(&self) -> &CellId {
        &self.cell_id
    }

    /// Returns the planned scope id.
    pub fn scope_id(&self) -> &ScopeId {
        &self.scope_id
    }

    /// Returns the value schema id.
    pub fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns the value semantic type id.
    pub fn semantic_type_id(&self) -> &SemanticTypeId {
        &self.semantic_type_id
    }

    /// Returns the value lineage reference.
    pub fn value_lineage(&self) -> &ValueLineageRef {
        &self.value_lineage
    }

    /// Returns the certified context constraint carried by the planned cell.
    pub fn context(&self) -> &CellContextSpec {
        &self.context
    }
}

/// Root seed cell specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSeedSpec {
    /// Stable seed author key.
    pub key: SeedKey,
    /// Derived seed id.
    pub seed_id: SeedId,
    /// Seed output cell id.
    pub cell_id: CellId,
    /// Root scope id.
    pub scope_id: ScopeId,
    /// Seed value schema id.
    pub schema_id: SchemaId,
    /// Seed value semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Seed value lineage ref.
    pub value_lineage: ValueLineageRef,
    /// Canonical seed content digest.
    pub content_digest: ContentDigest,
    /// Canonical seed byte length.
    pub byte_len: usize,
}

impl RootSeedSpec {
    /// Returns this seed's typed cell reference.
    pub fn typed_ref(&self) -> TypedHandleRef {
        TypedHandleRef {
            cell_id: self.cell_id.clone(),
            scope_id: self.scope_id.clone(),
            schema_id: self.schema_id.clone(),
            semantic_type_id: self.semantic_type_id.clone(),
            value_lineage: self.value_lineage.clone(),
            context: CellContextSpec::no_context(),
        }
    }
}
