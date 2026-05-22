#![warn(missing_docs)]
//! Typed state-program authoring contracts for MFM.
//!
//! This crate owns the branded authoring surface for typed programs. Handles
//! are minted only by framework builders, carry invariant program/scope/value
//! brands, and are lowered into unbranded draft specs only after root public
//! outputs have been bound inside the generative build closure.

extern crate self as mfm_program;

use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{
    CellId, ContentDigest, DigestAlgorithm, DigestBytes, NodeId, SchemaId, ScopeId, SeedId,
    SemanticTypeId,
};
use mfm_values::{MfmValue, PublicOutputDescriptor};

#[cfg(test)]
mod tests;

/// Result type for typed program authoring operations.
pub type Result<T> = std::result::Result<T, PlanError>;

/// Error returned by typed program authoring operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// Author-supplied stable key failed validation.
    Key(String),
    /// Value descriptor construction failed.
    Value(String),
    /// Canonical seed construction failed.
    Canonical(String),
    /// JSON serialization failed before canonicalization.
    Serialize(String),
    /// A root seed key was declared more than once.
    DuplicateSeedKey(String),
    /// A child scope key was declared more than once under the same parent.
    DuplicateChildScopeKey(String),
    /// A bridge key was declared more than once in the same child scope session.
    DuplicateBridgeKey(String),
    /// A public output field path was declared more than once.
    DuplicatePublicOutputPath(String),
    /// Root public outputs were bound more than once.
    PublicOutputsAlreadyBound,
    /// Public output binding must contain at least one cell.
    EmptyPublicOutputs,
    /// Live bridge evidence did not belong to the active child scope session.
    InvalidBridgeEvidence(String),
    /// A persisted bridge reference was not backed by an emitted bridge node.
    UnknownBridgeRef,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(message) => write!(f, "invalid typed program key: {message}"),
            Self::Value(message) => write!(f, "typed value descriptor error: {message}"),
            Self::Canonical(message) => write!(f, "canonical seed error: {message}"),
            Self::Serialize(message) => write!(f, "seed serialization error: {message}"),
            Self::DuplicateSeedKey(key) => write!(f, "duplicate root seed key {key}"),
            Self::DuplicateChildScopeKey(key) => write!(f, "duplicate child scope key {key}"),
            Self::DuplicateBridgeKey(key) => write!(f, "duplicate bridge key {key}"),
            Self::DuplicatePublicOutputPath(path) => {
                write!(f, "duplicate public output field path {path}")
            }
            Self::PublicOutputsAlreadyBound => f.write_str("root public outputs already bound"),
            Self::EmptyPublicOutputs => f.write_str("root public output binding is empty"),
            Self::InvalidBridgeEvidence(message) => {
                write!(f, "invalid bridge evidence: {message}")
            }
            Self::UnknownBridgeRef => f.write_str("bridge ref is not backed by an emitted node"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Stable root or child scope author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeKey(String);

impl ScopeKey {
    /// Creates a checked scope key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("scope key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable root seed author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeedKey(String);

impl SeedKey {
    /// Creates a checked seed key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("seed key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable public-output binding key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicOutputKey(String);

impl PublicOutputKey {
    /// Creates a checked public-output key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("public output key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable bridge node author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BridgeKey(String);

impl BridgeKey {
    /// Creates a checked bridge key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("bridge key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable public-output field path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicFieldPath(String);

impl PublicFieldPath {
    /// Creates a checked public field path.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("public field path", value.as_ref()).map(Self)
    }

    /// Returns the stable field path string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical launch seed material for a typed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSeed<T: MfmValue> {
    bytes: PlainCanonicalJsonBytes,
    content_digest: ContentDigest,
    byte_len: usize,
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    _value: PhantomData<fn(T) -> T>,
}

impl<T: MfmValue> CanonicalSeed<T> {
    /// Serializes and canonicalizes a launch seed value.
    pub fn from_value(value: &T) -> Result<Self> {
        let json = serde_json::to_string(value)
            .map_err(|error| PlanError::Serialize(error.to_string()))?;
        let bytes = PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|error| PlanError::Canonical(error.to_string()))?;
        Self::from_canonical_json(bytes)
    }

    /// Creates seed material from already-canonical JSON bytes.
    pub fn from_canonical_json(bytes: PlainCanonicalJsonBytes) -> Result<Self> {
        let _: T = serde_json::from_slice(bytes.as_bytes()).map_err(|error| {
            PlanError::Canonical(format!("seed bytes do not decode as value type: {error}"))
        })?;
        let schema_id = T::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let semantic_type_id =
            T::semantic_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let content_digest = bytes.content_digest();
        let byte_len = bytes.as_bytes().len();
        Ok(Self {
            bytes,
            content_digest,
            byte_len,
            schema_id,
            semantic_type_id,
            _value: PhantomData,
        })
    }

    /// Returns canonical JSON bytes for this seed.
    pub fn canonical_json(&self) -> &PlainCanonicalJsonBytes {
        &self.bytes
    }

    /// Returns the seed content digest.
    pub fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    /// Returns the canonical byte length.
    pub fn byte_len(&self) -> usize {
        self.byte_len
    }
}

/// Non-forgeable typed reference to a planned cell.
#[derive(Debug, PartialEq, Eq)]
pub struct Handle<'program, 'scope, T: MfmValue> {
    cell_id: CellId,
    scope_id: ScopeId,
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    origin: HandleOrigin,
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
            origin: self.origin.clone(),
            _program: PhantomData,
            _scope: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<'program, 'scope, T: MfmValue> Handle<'program, 'scope, T> {
    fn new(
        cell_id: CellId,
        scope_id: ScopeId,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
    ) -> Self {
        Self {
            cell_id,
            scope_id,
            schema_id,
            semantic_type_id,
            origin: HandleOrigin::Local,
            _program: PhantomData,
            _scope: PhantomData,
            _value: PhantomData,
        }
    }

    fn new_bridge(
        cell_id: CellId,
        scope_id: ScopeId,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        evidence: BridgeEvidenceCore,
    ) -> Self {
        Self {
            cell_id,
            scope_id,
            schema_id,
            semantic_type_id,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HandleOrigin {
    Local,
    Bridge { evidence: Box<BridgeEvidenceCore> },
}

impl HandleOrigin {
    fn bridge_evidence<'program, 'parent>(&self) -> Vec<BridgeEvidence<'program, 'parent>> {
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
    cell_id: CellId,
    /// Planned scope id.
    scope_id: ScopeId,
    /// Value schema id.
    schema_id: SchemaId,
    /// Value semantic type id.
    semantic_type_id: SemanticTypeId,
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
    /// Canonical seed content digest.
    pub content_digest: ContentDigest,
    /// Canonical seed byte length.
    pub byte_len: usize,
}

/// Persisted typed scope specification emitted by the program builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSpec {
    /// Stable scope author key.
    pub key: ScopeKey,
    /// Derived scope id.
    pub scope_id: ScopeId,
    /// Parent scope id for child scopes.
    pub parent_scope_id: Option<ScopeId>,
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
    fn as_str(self) -> &'static str {
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

    fn as_str(self) -> &'static str {
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
    /// Value semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Value schema id.
    pub schema_id: SchemaId,
    /// Bridge direction.
    pub bridge_kind: BridgeKind,
    /// Same-value bridge policy.
    pub policy: BridgePolicy,
    /// Framework provenance.
    pub provenance: BridgeProvenance,
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
struct BridgeSessionToken(DigestBytes);

#[derive(Debug, Clone, PartialEq, Eq)]
struct BridgeEvidenceCore {
    bridge_ref: BridgeRef,
    session_token: BridgeSessionToken,
}

/// Live bridge authority owned by one active child-scope builder invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvidence<'program, 'parent> {
    core: BridgeEvidenceCore,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
    _private: (),
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
    key: PublicOutputKey,
    /// Public output schema id.
    public_schema_id: SchemaId,
    /// Bound output cells.
    outputs: Vec<PublicOutputCellSpec>,
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
    public_output_spec: PublicOutputSpec,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    _private: (),
}

/// Parent-visible value returned from a child scope after bridge validation.
#[derive(Debug, PartialEq, Eq)]
pub struct Bridged<'program, 'parent, R> {
    value: R,
    bridge_evidence: Vec<BridgeEvidence<'program, 'parent>>,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
    _private: (),
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
    root_key: ScopeKey,
    root_scope_id: ScopeId,
    seeds: Vec<RootSeedSpec>,
    scopes: Vec<ScopeSpec>,
    bridge_nodes: Vec<BridgeNodeSpec>,
    public_output_spec: PublicOutputSpec,
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

    /// Returns root seed specs.
    pub fn seeds(&self) -> &[RootSeedSpec] {
        &self.seeds
    }

    /// Returns emitted scope specs, including the root scope.
    pub fn scopes(&self) -> &[ScopeSpec] {
        &self.scopes
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
}

/// Root-scope builder for a typed program.
pub struct RootBuilder<'program, 'scope> {
    root_key: ScopeKey,
    scope: ScopeBuilder<'program, 'scope>,
    seeds: Vec<RootSeedSpec>,
    seed_keys: BTreeSet<String>,
    public_outputs_bound: bool,
}

impl<'program, 'scope> RootBuilder<'program, 'scope> {
    /// Returns the root scope builder.
    pub fn scope(&mut self) -> &mut ScopeBuilder<'program, 'scope> {
        &mut self.scope
    }

    /// Declares a root launch seed and returns its typed cell handle.
    pub fn seed<T: MfmValue>(
        &mut self,
        key: SeedKey,
        value: CanonicalSeed<T>,
    ) -> Result<Handle<'program, 'scope, T>> {
        if !self.seed_keys.insert(key.as_str().to_owned()) {
            return Err(PlanError::DuplicateSeedKey(key.as_str().to_owned()));
        }

        let seed_id = seed_id(self.scope.scope_id(), &key)?;
        let cell_id = seed_cell_id(&seed_id)?;
        let spec = RootSeedSpec {
            key,
            seed_id,
            cell_id: cell_id.clone(),
            scope_id: self.scope.scope_id().clone(),
            schema_id: value.schema_id.clone(),
            semantic_type_id: value.semantic_type_id.clone(),
            content_digest: value.content_digest,
            byte_len: value.byte_len,
        };
        let handle = Handle::new(
            cell_id,
            spec.scope_id.clone(),
            spec.schema_id.clone(),
            spec.semantic_type_id.clone(),
        );
        self.seeds.push(spec);
        Ok(handle)
    }

    /// Binds root public outputs and returns unforgeable root-bound evidence.
    pub fn bind_public_outputs<P>(
        &mut self,
        key: PublicOutputKey,
        outputs: &P,
    ) -> Result<RootBound<'program, 'scope>>
    where
        P: PublicOutputs<'program, 'scope>,
    {
        if self.public_outputs_bound {
            return Err(PlanError::PublicOutputsAlreadyBound);
        }
        self.public_outputs_bound = true;
        let output_cells = outputs.output_cells()?;
        if output_cells.is_empty() {
            return Err(PlanError::EmptyPublicOutputs);
        }
        let mut seen = BTreeSet::new();
        for output in &output_cells {
            if !seen.insert(output.public_field_path().as_str().to_owned()) {
                return Err(PlanError::DuplicatePublicOutputPath(
                    output.public_field_path().as_str().to_owned(),
                ));
            }
        }
        Ok(RootBound {
            public_output_spec: PublicOutputSpec {
                key,
                public_schema_id: outputs.public_schema_id()?,
                outputs: output_cells,
            },
            _program: PhantomData,
            _scope: PhantomData,
            _private: (),
        })
    }
}

/// Scope-local builder. State and operation planning APIs are added in later
/// typed-core sections.
pub struct ScopeBuilder<'program, 'scope> {
    scope_id: ScopeId,
    child_scope_keys: BTreeSet<String>,
    child_scopes: Vec<ScopeSpec>,
    bridge_nodes: Vec<BridgeNodeSpec>,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

impl<'program, 'scope> ScopeBuilder<'program, 'scope> {
    /// Returns the typed scope id.
    pub fn scope_id(&self) -> &ScopeId {
        &self.scope_id
    }

    /// Opens a child scope and returns only values explicitly bridged back to this scope.
    pub fn child_scope<R>(
        &mut self,
        key: ScopeKey,
        f: impl for<'child> FnOnce(
            &mut ChildScopeBuilder<'program, 'scope, 'child>,
        ) -> Result<Bridged<'program, 'scope, R>>,
    ) -> Result<R> {
        if !self.child_scope_keys.insert(key.as_str().to_owned()) {
            return Err(PlanError::DuplicateChildScopeKey(key.as_str().to_owned()));
        }

        let child_scope_id = child_scope_id(&self.scope_id, &key)?;
        let session_token = bridge_session_token(&self.scope_id, &child_scope_id, &key);
        let mut child = ChildScopeBuilder {
            parent_scope_id: self.scope_id.clone(),
            scope: ScopeBuilder {
                scope_id: child_scope_id.clone(),
                child_scope_keys: BTreeSet::new(),
                child_scopes: Vec::new(),
                bridge_nodes: Vec::new(),
                _program: PhantomData,
                _scope: PhantomData,
            },
            session_token,
            bridge_keys: BTreeSet::new(),
            active_bridge_refs: BTreeSet::new(),
            bridge_nodes: Vec::new(),
            _parent: PhantomData,
        };

        let bridged = f(&mut child)?;
        child.validate_bridge_evidence_set(&bridged.bridge_evidence)?;
        self.child_scopes.push(ScopeSpec {
            key,
            scope_id: child_scope_id,
            parent_scope_id: Some(self.scope_id.clone()),
        });
        self.child_scopes.extend(child.scope.child_scopes);
        self.bridge_nodes.extend(child.bridge_nodes);
        self.bridge_nodes.extend(child.scope.bridge_nodes);
        Ok(bridged.value)
    }
}

/// Child-scope builder with live bridge-session authority.
pub struct ChildScopeBuilder<'program, 'parent, 'child> {
    parent_scope_id: ScopeId,
    scope: ScopeBuilder<'program, 'child>,
    session_token: BridgeSessionToken,
    bridge_keys: BTreeSet<String>,
    active_bridge_refs: BTreeSet<String>,
    bridge_nodes: Vec<BridgeNodeSpec>,
    _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
}

impl<'program, 'parent, 'child> ChildScopeBuilder<'program, 'parent, 'child> {
    /// Returns the child scope builder for nested child scopes.
    pub fn scope(&mut self) -> &mut ScopeBuilder<'program, 'child> {
        &mut self.scope
    }

    /// Returns the child scope id.
    pub fn scope_id(&self) -> &ScopeId {
        self.scope.scope_id()
    }

    /// Imports a parent handle into this child scope through an emitted bridge node.
    pub fn import_from_parent<T: MfmValue>(
        &mut self,
        key: BridgeKey,
        value: Handle<'program, 'parent, T>,
        policy: BridgePolicy,
    ) -> Result<Handle<'program, 'child, T>> {
        let source = value.typed_ref();
        let (target, evidence) = self.emit_bridge(
            key,
            BridgeKind::ImportFromParent,
            source,
            self.parent_scope_id.clone(),
            self.scope.scope_id().clone(),
            policy,
        )?;
        Ok(Handle::new_bridge(
            target.cell_id,
            target.scope_id,
            target.schema_id,
            target.semantic_type_id,
            evidence,
        ))
    }

    /// Exports a child handle into the parent scope through an emitted bridge node.
    pub fn export_to_parent<T: MfmValue>(
        &mut self,
        key: BridgeKey,
        value: Handle<'program, 'child, T>,
        policy: BridgePolicy,
    ) -> Result<Handle<'program, 'parent, T>> {
        let source = value.typed_ref();
        let (target, evidence) = self.emit_bridge(
            key,
            BridgeKind::ExportToParent,
            source,
            self.scope.scope_id().clone(),
            self.parent_scope_id.clone(),
            policy,
        )?;
        Ok(Handle::new_bridge(
            target.cell_id,
            target.scope_id,
            target.schema_id,
            target.semantic_type_id,
            evidence,
        ))
    }

    /// Wraps parent-visible values after validating their live bridge evidence.
    pub fn bridge_to_parent<R>(&mut self, value: R) -> Result<Bridged<'program, 'parent, R>>
    where
        R: BridgeableToParent<'program, 'parent>,
    {
        let evidence = value.bridge_evidence();
        self.validate_bridge_evidence_set(&evidence)?;
        Ok(Bridged {
            value,
            bridge_evidence: evidence,
            _program: PhantomData,
            _parent: PhantomData,
            _private: (),
        })
    }

    fn emit_bridge(
        &mut self,
        key: BridgeKey,
        bridge_kind: BridgeKind,
        source: TypedHandleRef,
        source_scope_id: ScopeId,
        target_scope_id: ScopeId,
        policy: BridgePolicy,
    ) -> Result<(TypedHandleRef, BridgeEvidenceCore)> {
        if source.scope_id != source_scope_id {
            return Err(PlanError::InvalidBridgeEvidence(format!(
                "source handle scope {} did not match bridge source {}",
                source.scope_id.as_str(),
                source_scope_id.as_str()
            )));
        }
        if !self.bridge_keys.insert(key.as_str().to_owned()) {
            return Err(PlanError::DuplicateBridgeKey(key.as_str().to_owned()));
        }

        let node_id = bridge_node_id(
            &source_scope_id,
            &target_scope_id,
            &source.cell_id,
            &key,
            bridge_kind,
            policy,
        )?;
        let target_cell_id = bridge_cell_id(&node_id)?;
        let spec = BridgeNodeSpec {
            node_id: node_id.clone(),
            key,
            source_scope_id,
            target_scope_id: target_scope_id.clone(),
            source_cell_id: source.cell_id,
            target_cell_id: target_cell_id.clone(),
            semantic_type_id: source.semantic_type_id.clone(),
            schema_id: source.schema_id.clone(),
            bridge_kind,
            policy,
            provenance: BridgeProvenance::FrameworkChildScopeV1,
        };
        let bridge_ref = spec.bridge_ref();
        self.active_bridge_refs.insert(bridge_ref_key(&bridge_ref));
        self.bridge_nodes.push(spec);
        let target = TypedHandleRef {
            cell_id: target_cell_id,
            scope_id: target_scope_id,
            schema_id: source.schema_id,
            semantic_type_id: source.semantic_type_id,
        };
        let evidence = BridgeEvidenceCore {
            bridge_ref,
            session_token: self.session_token,
        };
        Ok((target, evidence))
    }

    fn validate_bridge_evidence_set(
        &self,
        evidence_set: &[BridgeEvidence<'program, 'parent>],
    ) -> Result<()> {
        for evidence in evidence_set {
            self.validate_bridge_evidence(evidence)?;
        }
        Ok(())
    }

    fn validate_bridge_evidence(&self, evidence: &BridgeEvidence<'program, 'parent>) -> Result<()> {
        let bridge_ref = &evidence.core.bridge_ref;
        if evidence.core.session_token != self.session_token {
            if bridge_ref.target_scope_id == self.parent_scope_id
                && bridge_ref.source_scope_id != *self.scope.scope_id()
            {
                return Ok(());
            }
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge evidence belongs to a different child-scope session".to_owned(),
            ));
        }
        if bridge_ref.source_scope_id != *self.scope.scope_id()
            || bridge_ref.target_scope_id != self.parent_scope_id
        {
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge evidence does not export from this child to its parent".to_owned(),
            ));
        }
        if !self
            .active_bridge_refs
            .contains(&bridge_ref_key(bridge_ref))
        {
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge ref was not created by this child-scope builder".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Program-level public output binding contract generated by derives.
pub trait PublicOutputs<'program, 'scope> {
    /// Returns the public output schema id.
    fn public_schema_id(&self) -> Result<SchemaId>;

    /// Returns bound public output cells.
    fn output_cells(&self) -> Result<Vec<PublicOutputCellSpec>>;
}

/// Builds a branded root typed program and returns an unbranded draft.
pub fn build_root<F>(root_key: ScopeKey, f: F) -> Result<TypedProgramDraft>
where
    F: for<'program, 'root> FnOnce(
        &mut RootBuilder<'program, 'root>,
    ) -> Result<RootBound<'program, 'root>>,
{
    let root_scope_id = scope_id(&root_key)?;
    let mut builder = RootBuilder {
        root_key: root_key.clone(),
        scope: ScopeBuilder {
            scope_id: root_scope_id.clone(),
            child_scope_keys: BTreeSet::new(),
            child_scopes: Vec::new(),
            bridge_nodes: Vec::new(),
            _program: PhantomData,
            _scope: PhantomData,
        },
        seeds: Vec::new(),
        seed_keys: BTreeSet::new(),
        public_outputs_bound: false,
    };
    let bound = f(&mut builder)?;
    Ok(TypedProgramDraft {
        root_key: builder.root_key,
        root_scope_id: root_scope_id.clone(),
        seeds: builder.seeds,
        scopes: {
            let mut scopes = vec![ScopeSpec {
                key: root_key,
                scope_id: root_scope_id,
                parent_scope_id: None,
            }];
            scopes.extend(builder.scope.child_scopes);
            scopes
        },
        bridge_nodes: builder.scope.bridge_nodes,
        public_output_spec: bound.public_output_spec,
    })
}

/// Returns the public schema id for a derive-backed public output descriptor.
pub fn public_schema_id<P: PublicOutputDescriptor>() -> Result<SchemaId> {
    P::public_schema_id().map_err(|error| PlanError::Value(error.to_string()))
}

fn checked_key(label: &str, value: &str) -> Result<String> {
    if is_valid_author_key(value) {
        Ok(value.to_owned())
    } else {
        Err(PlanError::Key(format!(
            "{label} {value:?} must match [a-z0-9][a-z0-9._/-]*"
        )))
    }
}

fn is_valid_author_key(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|ch| {
        ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-' | '/')
    })
}

fn scope_id(key: &ScopeKey) -> Result<ScopeId> {
    digest_only_id("scope", key.as_str(), ScopeId::from_digest)
}

fn child_scope_id(parent_scope_id: &ScopeId, key: &ScopeKey) -> Result<ScopeId> {
    digest_only_id(
        "child-scope",
        &format!("{}:{}", parent_scope_id.as_str(), key.as_str()),
        ScopeId::from_digest,
    )
}

fn seed_id(scope_id: &ScopeId, key: &SeedKey) -> Result<SeedId> {
    digest_only_id(
        "seed",
        &format!("{}:{}", scope_id.as_str(), key.as_str()),
        SeedId::from_digest,
    )
}

fn seed_cell_id(seed_id: &SeedId) -> Result<CellId> {
    digest_only_id("seed-cell", seed_id.as_str(), CellId::from_digest)
}

fn bridge_node_id(
    source_scope_id: &ScopeId,
    target_scope_id: &ScopeId,
    source_cell_id: &CellId,
    key: &BridgeKey,
    bridge_kind: BridgeKind,
    policy: BridgePolicy,
) -> Result<NodeId> {
    digest_only_id(
        "bridge-node",
        &format!(
            "{}:{}:{}:{}:{}:{}",
            source_scope_id.as_str(),
            target_scope_id.as_str(),
            source_cell_id.as_str(),
            key.as_str(),
            bridge_kind.as_str(),
            policy.as_str()
        ),
        NodeId::from_digest,
    )
}

fn bridge_cell_id(node_id: &NodeId) -> Result<CellId> {
    digest_only_id("bridge-cell", node_id.as_str(), CellId::from_digest)
}

fn bridge_session_token(
    parent_scope_id: &ScopeId,
    child_scope_id: &ScopeId,
    key: &ScopeKey,
) -> BridgeSessionToken {
    BridgeSessionToken(sha256_digest_bytes(
        format!(
            "mfm.program:bridge-session:{}:{}:{}",
            parent_scope_id.as_str(),
            child_scope_id.as_str(),
            key.as_str()
        )
        .as_bytes(),
    ))
}

fn bridge_ref_key(bridge_ref: &BridgeRef) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        bridge_ref.source_scope_id.as_str(),
        bridge_ref.target_scope_id.as_str(),
        bridge_ref.source_cell_id.as_str(),
        bridge_ref.target_cell_id.as_str(),
        bridge_ref.semantic_type_id.as_str(),
        bridge_ref.schema_id.as_str(),
        bridge_ref.bridge_node_id.as_str()
    )
}

fn digest_only_id<I>(
    domain: &str,
    value: &str,
    construct: fn(DigestAlgorithm, DigestBytes) -> I,
) -> Result<I> {
    let digest = sha256_digest_bytes(format!("mfm.program:{domain}:{value}").as_bytes());
    Ok(construct(DigestAlgorithm::Sha256JcsV1, digest))
}

mod private {
    use super::{Handle, MfmValue};

    pub trait BridgeableSealed {}

    impl<'program, 'parent, T> BridgeableSealed for Handle<'program, 'parent, T> where T: MfmValue {}

    impl BridgeableSealed for () {}

    macro_rules! impl_tuple {
        ($($name:ident),+ $(,)?) => {
            impl<$($name),+> BridgeableSealed for ($($name,)+) {}
        };
    }

    impl_tuple!(A);
    impl_tuple!(A, B);
    impl_tuple!(A, B, C);
    impl_tuple!(A, B, C, D);
}
