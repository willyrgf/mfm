use super::super::*;
use super::contract::ArtifactRole;

/// Named typed cell reference used by public output events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedTypedCellRef {
    /// Public output field path.
    pub public_field_path: PublicFieldPath,
    /// Cell id.
    pub cell_id: CellId,
    /// Cell producer.
    pub producer: CellProducer,
    /// Cell scope id.
    pub scope_id: ScopeId,
    /// Cell semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Cell schema id.
    pub schema_id: SchemaId,
    /// Value lineage reference.
    pub value_lineage: ValueLineageRef,
    /// Cell value content digest.
    pub content_digest: ContentDigest,
    /// Cell value artifact id.
    pub artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the cell value artifact.
    pub evidence_hash: ContentDigest,
}

/// Typed skip reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipReason {
    /// Stable skip code.
    pub code: ErrorCode,
    /// Public safe message.
    pub safe_message: String,
}

/// Retained artifact reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionRef {
    /// Artifact id retained.
    pub artifact_id: ArtifactId,
    /// Artifact role retained.
    pub role: ArtifactRole,
    /// Exact canonical artifact evidence identity retained.
    pub evidence_hash: ContentDigest,
    /// Artifact content digest.
    pub content_digest: ContentDigest,
}

/// Retention reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RetentionReason {
    /// Initial certified run retention.
    RunAdmitted,
    /// Runtime artifact retention.
    RuntimeEvidence,
    /// Public output retention.
    PublicOutput,
    /// Manifest compaction/projection retention.
    ManifestProjection,
}
