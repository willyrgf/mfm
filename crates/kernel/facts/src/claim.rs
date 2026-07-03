use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    EventId, NodeId, RunId, SchemaId,
};

use crate::*;

/// Stable identity for one recorded fact claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactClaimId {
    pub(crate) source_run_id: RunId,
    pub(crate) source_seq: u64,
    pub(crate) source_ordinal: u32,
}

impl FactClaimId {
    /// Creates a claim id from run-stream coordinates.
    pub fn new(source_run_id: RunId, source_seq: u64, source_ordinal: u32) -> Result<Self> {
        if source_seq == 0 {
            return Err(FactError::descriptor(
                "fact claim source sequence must be non-zero",
            ));
        }
        Ok(Self {
            source_run_id,
            source_seq,
            source_ordinal,
        })
    }

    /// Returns the source run id.
    pub const fn source_run_id(&self) -> &RunId {
        &self.source_run_id
    }

    /// Returns the source run stream sequence.
    pub const fn source_seq(&self) -> u64 {
        self.source_seq
    }

    /// Returns the source event ordinal inside the atomic append.
    pub const fn source_ordinal(&self) -> u32 {
        self.source_ordinal
    }
}

/// Descriptor-derived subject authority recorded with a fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectEvidence {
    pub(crate) fact_subject_namespace_hash: ContentDigest,
    pub(crate) subject_material: PlainCanonicalJsonBytes,
    pub(crate) subject_material_hash: ContentDigest,
    pub(crate) fact_key: FactKey,
}

impl FactSubjectEvidence {
    /// Creates subject evidence from canonical subject material values.
    pub fn from_material(
        fact_subject_namespace_hash: ContentDigest,
        material: &FactSubjectMaterialV1,
    ) -> Result<Self> {
        let material_bytes = canonical_fact_subject_material_bytes(material)?;
        let subject_material =
            PlainCanonicalJsonBytes::from_canonical_json_slice(material_bytes.as_bytes())
                .map_err(|error| FactError::canonical(error.to_string()))?;
        let subject_material_hash = subject_material.content_digest();
        let fact_key = derive_fact_key(
            fact_subject_namespace_hash.clone(),
            subject_material_hash.clone(),
        )?;
        Self::new(
            fact_subject_namespace_hash,
            subject_material,
            subject_material_hash,
            fact_key,
        )
    }

    /// Creates subject evidence from canonical descriptor and subject material authority.
    pub fn new(
        fact_subject_namespace_hash: ContentDigest,
        subject_material: PlainCanonicalJsonBytes,
        subject_material_hash: ContentDigest,
        fact_key: FactKey,
    ) -> Result<Self> {
        if subject_material.content_digest() != subject_material_hash {
            return Err(FactError::descriptor(
                "subject material hash does not match subject material bytes",
            ));
        }
        let expected_fact_key = derive_fact_key(
            fact_subject_namespace_hash.clone(),
            subject_material_hash.clone(),
        )?;
        if expected_fact_key != fact_key {
            return Err(FactError::descriptor(
                "fact key does not match subject namespace and material hashes",
            ));
        }
        Ok(Self {
            fact_subject_namespace_hash,
            subject_material,
            subject_material_hash,
            fact_key,
        })
    }

    /// Returns the descriptor-derived subject namespace hash.
    pub const fn fact_subject_namespace_hash(&self) -> &ContentDigest {
        &self.fact_subject_namespace_hash
    }

    /// Returns the canonical subject material bytes.
    pub const fn subject_material(&self) -> &PlainCanonicalJsonBytes {
        &self.subject_material
    }

    /// Returns the canonical subject material hash.
    pub const fn subject_material_hash(&self) -> &ContentDigest {
        &self.subject_material_hash
    }

    /// Returns the derived subject fact key.
    pub const fn fact_key(&self) -> &FactKey {
        &self.fact_key
    }
}

/// Compact subject identity carried by internal fact references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectRef {
    pub(crate) fact_subject_namespace_hash: ContentDigest,
    pub(crate) fact_key: FactKey,
    pub(crate) subject_material_hash: ContentDigest,
}

impl FactSubjectRef {
    /// Creates a compact subject reference.
    pub fn new(
        fact_subject_namespace_hash: ContentDigest,
        fact_key: FactKey,
        subject_material_hash: ContentDigest,
    ) -> Self {
        Self {
            fact_subject_namespace_hash,
            fact_key,
            subject_material_hash,
        }
    }

    /// Creates a compact subject reference from recorded subject evidence.
    pub fn from_evidence(evidence: &FactSubjectEvidence) -> Self {
        Self::new(
            evidence.fact_subject_namespace_hash().clone(),
            evidence.fact_key().clone(),
            evidence.subject_material_hash().clone(),
        )
    }

    /// Returns the descriptor-derived subject namespace hash.
    pub const fn fact_subject_namespace_hash(&self) -> &ContentDigest {
        &self.fact_subject_namespace_hash
    }

    /// Returns the derived subject fact key.
    pub const fn fact_key(&self) -> &FactKey {
        &self.fact_key
    }

    /// Returns the canonical subject material hash.
    pub const fn subject_material_hash(&self) -> &ContentDigest {
        &self.subject_material_hash
    }
}

/// Optional request evidence recorded with a fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRequestEvidence {
    pub(crate) request_schema_id: SchemaId,
    pub(crate) request_hash: ContentDigest,
}

impl FactRequestEvidence {
    /// Creates request evidence from canonical request authority.
    pub fn new(request_schema_id: SchemaId, request_hash: ContentDigest) -> Self {
        Self {
            request_schema_id,
            request_hash,
        }
    }

    /// Returns the request schema id.
    pub const fn request_schema_id(&self) -> &SchemaId {
        &self.request_schema_id
    }

    /// Returns the canonical request hash.
    pub const fn request_hash(&self) -> &ContentDigest {
        &self.request_hash
    }
}

/// Response artifact evidence recorded with a fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactResponseEvidence {
    pub(crate) response_schema_id: SchemaId,
    pub(crate) response_hash: ContentDigest,
    pub(crate) artifact_id: ArtifactId,
    pub(crate) artifact_evidence_hash: ContentDigest,
}

impl FactResponseEvidence {
    /// Creates response artifact evidence.
    pub fn new(
        response_schema_id: SchemaId,
        response_hash: ContentDigest,
        artifact_id: ArtifactId,
        artifact_evidence_hash: ContentDigest,
    ) -> Self {
        Self {
            response_schema_id,
            response_hash,
            artifact_id,
            artifact_evidence_hash,
        }
    }

    /// Returns the response schema id.
    pub const fn response_schema_id(&self) -> &SchemaId {
        &self.response_schema_id
    }

    /// Returns the canonical response hash.
    pub const fn response_hash(&self) -> &ContentDigest {
        &self.response_hash
    }

    /// Returns the response artifact id.
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// Returns the response artifact evidence hash.
    pub const fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.artifact_evidence_hash
    }
}

/// Certified runtime provenance for a producing fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactProducerProvenance {
    pub(crate) capability_kind: CapabilityKind,
    pub(crate) capability_version: CapabilityVersion,
    pub(crate) adapter_kind: AdapterKind,
    pub(crate) adapter_version: AdapterVersion,
}

impl FactProducerProvenance {
    /// Creates producer provenance from certified capability and adapter identity.
    pub fn new(
        capability_kind: CapabilityKind,
        capability_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    ) -> Self {
        Self {
            capability_kind,
            capability_version,
            adapter_kind,
            adapter_version,
        }
    }

    /// Returns the producing capability kind.
    pub const fn capability_kind(&self) -> &CapabilityKind {
        &self.capability_kind
    }

    /// Returns the producing capability version.
    pub const fn capability_version(&self) -> &CapabilityVersion {
        &self.capability_version
    }

    /// Returns the producing adapter kind.
    pub const fn adapter_kind(&self) -> &AdapterKind {
        &self.adapter_kind
    }

    /// Returns the producing adapter version.
    pub const fn adapter_version(&self) -> &AdapterVersion {
        &self.adapter_version
    }
}

/// Constructor parts for a normalized recorded fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactClaimParts {
    /// Visibility selected by the producer.
    pub visibility: FactVisibility,
    /// Fact kind from the validated descriptor.
    pub fact_kind: FactKind,
    /// Content digest of the canonical fact descriptor bytes.
    pub fact_descriptor_hash: ContentDigest,
    /// Descriptor-derived subject evidence.
    pub subject: FactSubjectEvidence,
    /// Optional source observation timestamp.
    pub observed_at: Option<String>,
    /// Optional request evidence.
    pub request: Option<FactRequestEvidence>,
    /// Response artifact evidence.
    pub response: FactResponseEvidence,
    /// Producer provenance.
    pub producer: FactProducerProvenance,
}

/// Normalized fact claim carried by `FactRecorded` events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactClaim {
    pub(crate) parts: FactClaimParts,
}

impl FactClaim {
    /// Creates a normalized fact claim from validated parts.
    pub fn new(parts: FactClaimParts) -> Result<Self> {
        if parts
            .observed_at
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(FactError::descriptor(
                "fact claim observed_at must be non-empty when present",
            ));
        }
        Ok(Self { parts })
    }

    /// Returns the claim visibility.
    pub const fn visibility(&self) -> &FactVisibility {
        &self.parts.visibility
    }

    /// Returns the fact kind.
    pub const fn fact_kind(&self) -> &FactKind {
        &self.parts.fact_kind
    }

    /// Returns the fact descriptor hash.
    pub const fn fact_descriptor_hash(&self) -> &ContentDigest {
        &self.parts.fact_descriptor_hash
    }

    /// Returns subject evidence.
    pub const fn subject(&self) -> &FactSubjectEvidence {
        &self.parts.subject
    }

    /// Returns the optional source observation timestamp.
    pub fn observed_at(&self) -> Option<&str> {
        self.parts.observed_at.as_deref()
    }

    /// Returns optional request evidence.
    pub const fn request(&self) -> Option<&FactRequestEvidence> {
        self.parts.request.as_ref()
    }

    /// Returns response artifact evidence.
    pub const fn response(&self) -> &FactResponseEvidence {
        &self.parts.response
    }

    /// Returns producer provenance.
    pub const fn producer(&self) -> &FactProducerProvenance {
        &self.parts.producer
    }
}

/// Constructor parts for an [`InternalFactRef`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalFactRefParts {
    /// Source fact claim id.
    pub fact_claim_id: FactClaimId,
    /// Source event id for the recorded fact event.
    pub source_event_id: EventId,
    /// Store-assigned recorded time.
    pub recorded_at: String,
    /// Producing node id for the recorded fact response artifact.
    pub producer_node_id: NodeId,
    /// Optional source observation time.
    pub observed_at: Option<String>,
    /// Fact visibility; must be indexed for an internal ref.
    pub visibility: FactVisibility,
    /// Fact kind.
    pub fact_kind: FactKind,
    /// Fact descriptor hash.
    pub fact_descriptor_hash: ContentDigest,
    /// Subject identity.
    pub subject: FactSubjectRef,
    /// Optional request evidence.
    pub request: Option<FactRequestEvidence>,
    /// Response artifact evidence.
    pub response: FactResponseEvidence,
    /// Producer provenance.
    pub producer: FactProducerProvenance,
}

/// Internal trusted reference to an indexed fact projection row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalFactRef {
    pub(crate) parts: InternalFactRefParts,
}

impl InternalFactRef {
    /// Builds an internal fact ref from a recorded fact claim.
    pub fn from_claim(
        fact_claim_id: FactClaimId,
        source_event_id: EventId,
        recorded_at: String,
        producer_node_id: NodeId,
        claim: &FactClaim,
    ) -> Result<Option<Self>> {
        if matches!(claim.visibility(), FactVisibility::RunPrivate) {
            return Ok(None);
        }
        Self::new(InternalFactRefParts {
            fact_claim_id,
            source_event_id,
            recorded_at,
            producer_node_id,
            observed_at: claim.observed_at().map(str::to_owned),
            visibility: claim.visibility().clone(),
            fact_kind: claim.fact_kind().clone(),
            fact_descriptor_hash: claim.fact_descriptor_hash().clone(),
            subject: FactSubjectRef::from_evidence(claim.subject()),
            request: claim.request().cloned(),
            response: claim.response().clone(),
            producer: claim.producer().clone(),
        })
        .map(Some)
    }

    /// Creates an internal fact ref from validated parts.
    pub fn new(parts: InternalFactRefParts) -> Result<Self> {
        if matches!(parts.visibility, FactVisibility::RunPrivate) {
            return Err(FactError::descriptor(
                "internal fact refs require indexed visibility",
            ));
        }
        if parts.recorded_at.is_empty() {
            return Err(FactError::descriptor(
                "internal fact refs require recorded_at",
            ));
        }
        if parts
            .observed_at
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(FactError::descriptor(
                "internal fact refs require non-empty observed_at when present",
            ));
        }
        Ok(Self { parts })
    }

    /// Returns the source fact claim id.
    pub const fn fact_claim_id(&self) -> &FactClaimId {
        &self.parts.fact_claim_id
    }

    /// Returns the source event id.
    pub const fn source_event_id(&self) -> &EventId {
        &self.parts.source_event_id
    }

    /// Returns the recorded time.
    pub fn recorded_at(&self) -> &str {
        &self.parts.recorded_at
    }

    /// Returns the producing node id for the fact response artifact.
    pub const fn producer_node_id(&self) -> &NodeId {
        &self.parts.producer_node_id
    }

    /// Returns the optional observed time.
    pub fn observed_at(&self) -> Option<&str> {
        self.parts.observed_at.as_deref()
    }

    /// Returns fact visibility.
    pub const fn visibility(&self) -> &FactVisibility {
        &self.parts.visibility
    }

    /// Returns the fact kind.
    pub const fn fact_kind(&self) -> &FactKind {
        &self.parts.fact_kind
    }

    /// Returns the fact descriptor hash.
    pub const fn fact_descriptor_hash(&self) -> &ContentDigest {
        &self.parts.fact_descriptor_hash
    }

    /// Returns the fact key.
    pub const fn fact_key(&self) -> &FactKey {
        self.parts.subject.fact_key()
    }

    /// Returns the subject namespace hash.
    pub const fn fact_subject_namespace_hash(&self) -> &ContentDigest {
        self.parts.subject.fact_subject_namespace_hash()
    }

    /// Returns the subject material hash.
    pub const fn subject_material_hash(&self) -> &ContentDigest {
        self.parts.subject.subject_material_hash()
    }

    /// Returns the optional request schema id.
    pub const fn request_schema_id(&self) -> Option<&SchemaId> {
        match &self.parts.request {
            Some(request) => Some(request.request_schema_id()),
            None => None,
        }
    }

    /// Returns the optional request hash.
    pub const fn request_hash(&self) -> Option<&ContentDigest> {
        match &self.parts.request {
            Some(request) => Some(request.request_hash()),
            None => None,
        }
    }

    /// Returns the response schema id.
    pub const fn response_schema_id(&self) -> &SchemaId {
        self.parts.response.response_schema_id()
    }

    /// Returns the response content hash.
    pub const fn response_hash(&self) -> &ContentDigest {
        self.parts.response.response_hash()
    }

    /// Returns the response artifact id.
    pub const fn artifact_id(&self) -> &ArtifactId {
        self.parts.response.artifact_id()
    }

    /// Returns the response artifact evidence hash.
    pub const fn artifact_evidence_hash(&self) -> &ContentDigest {
        self.parts.response.artifact_evidence_hash()
    }

    /// Returns the producing capability kind.
    pub const fn capability_kind(&self) -> &CapabilityKind {
        self.parts.producer.capability_kind()
    }

    /// Returns the producing capability version.
    pub const fn capability_version(&self) -> &CapabilityVersion {
        self.parts.producer.capability_version()
    }

    /// Returns the producing adapter kind.
    pub const fn adapter_kind(&self) -> &AdapterKind {
        self.parts.producer.adapter_kind()
    }

    /// Returns the producing adapter version.
    pub const fn adapter_version(&self) -> &AdapterVersion {
        self.parts.producer.adapter_version()
    }
}
