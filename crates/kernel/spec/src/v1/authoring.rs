use super::*;

/// Non-semantic audit metadata carried beside a certified spec.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TypedExecutionSpecAudit {
    /// Descriptor audit references.
    pub descriptor_audit_refs: Vec<DescriptorAuditRef>,
    /// Source package references.
    pub source_package_refs: Vec<SourcePackageRef>,
    /// Non-semantic provenance references.
    pub non_semantic_provenance: Vec<AuditProvenanceRef>,
}

/// Descriptor audit reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorAuditRef {
    /// Descriptor id.
    pub descriptor_id: DescriptorId,
    /// Audit artifact id.
    pub artifact_id: ArtifactId,
    /// Audit artifact digest.
    pub digest: ContentDigest,
}

/// Source package reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePackageRef {
    /// Source package name.
    pub name: String,
    /// Source package version.
    pub version: String,
    /// Optional source artifact id.
    pub artifact_id: Option<ArtifactId>,
}

/// Non-semantic audit provenance reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditProvenanceRef {
    /// Stable provenance kind.
    pub kind: String,
    /// Provenance artifact id.
    pub artifact_id: ArtifactId,
    /// Provenance digest.
    pub digest: ContentDigest,
}

/// Authoring provenance for a typed spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoringProvenance {
    /// Spec was authored by expanding one operation.
    OperationExpansion {
        /// Operation descriptor id.
        operation_descriptor_id: DescriptorId,
        /// Operation config digest.
        config_hash: ContentDigest,
    },
    /// Spec was authored by direct state composition.
    StateComposition {
        /// Composition descriptor.
        descriptor: CompositionDescriptor,
        /// Composition config digest.
        config_hash: ContentDigest,
    },
    /// Spec mixes operation expansion and directly declared states.
    MixedComposition {
        /// Mixed composition descriptor.
        descriptor: CompositionDescriptor,
        /// Composition config digest.
        config_hash: ContentDigest,
    },
}

impl AuthoringProvenance {
    pub(super) fn json(&self) -> serde_json::Value {
        match self {
            Self::OperationExpansion {
                operation_descriptor_id,
                config_hash,
            } => serde_json::json!({
                "config_hash": config_hash.as_str(),
                "kind": "operation_expansion",
                "operation_descriptor_id": operation_descriptor_id.as_str(),
            }),
            Self::StateComposition {
                descriptor,
                config_hash,
            } => serde_json::json!({
                "config_hash": config_hash.as_str(),
                "descriptor": descriptor.json(),
                "kind": "state_composition",
            }),
            Self::MixedComposition {
                descriptor,
                config_hash,
            } => serde_json::json!({
                "config_hash": config_hash.as_str(),
                "descriptor": descriptor.json(),
                "kind": "mixed_composition",
            }),
        }
    }
}

/// Stable composition descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionDescriptor {
    /// Composition descriptor id.
    pub descriptor_id: DescriptorId,
    /// Stable descriptor name.
    pub name: String,
    /// Stable descriptor version.
    pub version: String,
}

impl CompositionDescriptor {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "descriptor_id": self.descriptor_id.as_str(),
            "name": self.name,
            "version": self.version,
        })
    }
}

/// Config artifact reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRef {
    /// Config schema id.
    pub schema_id: SchemaId,
    /// Config artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical config digest.
    pub digest: ContentDigest,
    /// Canonical byte length.
    pub byte_len: u64,
    /// Config artifact media type.
    pub media_type: MediaType,
}

impl ConfigRef {
    pub(super) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "artifact_id": self.artifact_id.as_str(),
            "byte_len": self.byte_len,
            "digest": self.digest.as_str(),
            "media_type": self.media_type.as_str(),
            "schema_id": self.schema_id.as_str(),
        })
    }
}

/// Persisted typed scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSpec {
    /// Scope id.
    pub scope_id: ScopeId,
    /// Parent scope id, when nested.
    pub parent_scope_id: Option<ScopeId>,
    /// Stable author key.
    pub stable_key: StableAuthorKey,
    /// Operation lineage active when this scope id was derived.
    pub planning_lineage: PlanningLineage,
}

impl ScopeSpec {
    pub(super) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "parent_scope_id": self.parent_scope_id.as_ref().map(ScopeId::as_str),
            "planning_lineage": self.planning_lineage.json(),
            "scope_id": self.scope_id.as_str(),
            "stable_key": self.stable_key.as_str(),
        })
    }
}

/// Declared root seed cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedSpec {
    /// Seed id.
    pub seed_id: SeedId,
    /// Stable seed key.
    pub seed_key: StableAuthorKey,
    /// Planned seed cell id.
    pub cell_id: CellId,
    /// Owning scope id.
    pub scope_id: ScopeId,
    /// Seed semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Seed schema id.
    pub schema_id: SchemaId,
    /// Required launch digest, if fixed by the spec.
    pub required_digest: Option<ContentDigest>,
}

impl SeedSpec {
    pub(super) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "cell_id": self.cell_id.as_str(),
            "required_digest": self.required_digest.as_ref().map(ContentDigest::as_str),
            "schema_id": self.schema_id.as_str(),
            "scope_id": self.scope_id.as_str(),
            "seed_id": self.seed_id.as_str(),
            "seed_key": self.seed_key.as_str(),
            "semantic_type_id": self.semantic_type_id.as_str(),
        })
    }
}
