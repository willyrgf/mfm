use super::*;

/// Persisted typed-spec certificate.
///
/// This is durable evidence only. Parsed or constructed certificate data is hostile until
/// [`verify_persisted_spec_certificate`] validates it against a registry and returns
/// [`CertifiedTypedSpec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSpecCertificate {
    /// Hash of the hash-defining certificate evidence.
    pub certificate_hash: ContentDigest,
    /// Hash-defining certificate evidence.
    pub evidence: CertifiedSpecCertificateEvidence,
}

impl CertifiedSpecCertificate {
    /// Builds persisted certificate evidence and computes its deterministic certificate hash.
    ///
    /// This does not mint runtime authority. Call [`verify_persisted_spec_certificate`] to turn
    /// persisted bytes into [`CertifiedTypedSpec`].
    pub fn from_evidence(evidence: CertifiedSpecCertificateEvidence) -> Result<Self> {
        let certificate_hash = evidence.canonical_json()?.content_digest();
        Ok(Self {
            certificate_hash,
            evidence,
        })
    }

    /// Parses persisted certificate JSON as untrusted certificate data.
    pub fn from_json_slice(input: &[u8]) -> Result<Self> {
        let input = std::str::from_utf8(input)
            .map_err(|error| certificate(format!("certificate JSON is not UTF-8: {error}")))?;
        Self::from_json_str(input)
    }

    /// Parses persisted certificate JSON as untrusted certificate data.
    pub fn from_json_str(input: &str) -> Result<Self> {
        let input_canonical = PlainCanonicalJsonBytes::from_json_str(input)
            .map_err(|error| certificate(error.to_string()))?;
        let value: serde_json::Value =
            serde_json::from_str(input).map_err(|error| certificate(error.to_string()))?;
        let parsed = parse_certificate(&value)?;
        parsed.verify_hash()?;
        if parsed.canonical_json()? != input_canonical {
            return Err(certificate(
                "persisted certificate contains unknown or non-normalized fields",
            ));
        }
        Ok(parsed)
    }

    /// Returns canonical JSON bytes for this persisted certificate.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json_bytes(self.json())
    }

    /// Verifies that `certificate_hash` matches the hash-defining evidence.
    pub fn verify_hash(&self) -> Result<()> {
        let actual = self.evidence.canonical_json()?.content_digest();
        if self.certificate_hash != actual {
            return Err(certificate(format!(
                "certificate hash mismatch: expected {}, recomputed {}",
                self.certificate_hash, actual
            )));
        }
        Ok(())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "certificate_hash": self.certificate_hash.as_str(),
            "evidence": self.evidence.json(),
        })
    }
}

/// Hash-defining certificate evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSpecCertificateEvidence {
    /// Persisted certificate contract version.
    pub certificate_version: String,
    /// Persisted certificate media type.
    pub media_type: String,
    /// Certifier algorithm identity.
    pub certifier_algorithm: String,
    /// Spec hash that this certificate covers.
    pub spec_hash: SpecHash,
    /// Digest of the certification registry authority.
    pub registry_digest: ContentDigest,
    /// Descriptor identities and digests covered by certification.
    pub descriptor_identities: Vec<CertifiedDescriptorEvidence>,
    /// Schema role grants resolved during certification.
    pub schema_role_grants: Vec<CertifiedSchemaRoleGrantEvidence>,
    /// Manual authorization verifier identities resolved during certification.
    pub manual_authorization_verifiers: Vec<CertifiedManualAuthorizationVerifierEvidence>,
    /// Operator authority snapshots resolved during certification.
    pub operator_authority_snapshots: Vec<CertifiedOperatorAuthoritySnapshotEvidence>,
}

impl CertifiedSpecCertificateEvidence {
    /// Returns canonical JSON bytes for the hash-defining certificate evidence.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json_bytes(self.json())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "certificate_version": self.certificate_version.as_str(),
            "certifier_algorithm": self.certifier_algorithm.as_str(),
            "descriptor_identities": self
                .descriptor_identities
                .iter()
                .map(CertifiedDescriptorEvidence::json)
                .collect::<Vec<_>>(),
            "manual_authorization_verifiers": self
                .manual_authorization_verifiers
                .iter()
                .map(CertifiedManualAuthorizationVerifierEvidence::json)
                .collect::<Vec<_>>(),
            "media_type": self.media_type.as_str(),
            "operator_authority_snapshots": self
                .operator_authority_snapshots
                .iter()
                .map(CertifiedOperatorAuthoritySnapshotEvidence::json)
                .collect::<Vec<_>>(),
            "registry_digest": self.registry_digest.as_str(),
            "schema_role_grants": self
                .schema_role_grants
                .iter()
                .map(CertifiedSchemaRoleGrantEvidence::json)
                .collect::<Vec<_>>(),
            "spec_hash": self.spec_hash.as_str(),
        })
    }
}

/// Certified schema role in the certification registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CertifiedSchemaRole {
    /// Schema may be used as manual resolution evidence.
    ManualResolutionEvidence,
    /// Schema may be used as manual resolution authorization proof evidence.
    ManualResolutionAuthorization,
}

impl CertifiedSchemaRole {
    /// Returns the stable persisted schema-role string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManualResolutionEvidence => "manual_resolution_evidence",
            Self::ManualResolutionAuthorization => "manual_resolution_authorization",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "manual_resolution_evidence" => Ok(Self::ManualResolutionEvidence),
            "manual_resolution_authorization" => Ok(Self::ManualResolutionAuthorization),
            other => Err(certificate(format!(
                "unknown certified schema role {other:?}"
            ))),
        }
    }
}

/// Schema role grant resolved from the certification registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSchemaRoleGrantEvidence {
    /// Schema id that received the grant.
    pub schema_id: SchemaId,
    /// Certified schema role.
    pub role: CertifiedSchemaRole,
}

impl CertifiedSchemaRoleGrantEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "role": self.role.as_str(),
            "schema_id": self.schema_id.as_str(),
        })
    }
}

/// Manual authorization verifier resolved from the certification registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedManualAuthorizationVerifierEvidence {
    /// Verifier id trusted by the registry.
    pub verifier_id: spec::ManualAuthorizationVerifierId,
}

impl CertifiedManualAuthorizationVerifierEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "verifier_id": self.verifier_id.as_str(),
        })
    }
}

/// Operator authority snapshot resolved from the certification registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedOperatorAuthoritySnapshotEvidence {
    /// Authority id trusted by the registry.
    pub authority_id: spec::OperatorAuthorityId,
    /// Digest of the certified authority snapshot.
    pub authority_digest: ContentDigest,
    /// Operators included in the certified snapshot.
    pub operators: Vec<CertifiedOperatorAuthorityMemberEvidence>,
}

impl CertifiedOperatorAuthoritySnapshotEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "authority_digest": self.authority_digest.as_str(),
            "authority_id": self.authority_id.as_str(),
            "operators": self
                .operators
                .iter()
                .map(CertifiedOperatorAuthorityMemberEvidence::json)
                .collect::<Vec<_>>(),
        })
    }
}

/// Operator member included in certified operator authority snapshot evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedOperatorAuthorityMemberEvidence {
    /// Stable operator id.
    pub operator_id: spec::OperatorId,
    /// Public operator identity.
    pub public_identity: spec::OperatorPublicIdentity,
}

impl CertifiedOperatorAuthorityMemberEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "operator_id": self.operator_id.as_str(),
            "public_identity": self.public_identity.as_str(),
        })
    }
}

/// Descriptor family covered by a certificate.
pub type CertifiedDescriptorFamily = spec::DescriptorFamily;

/// Descriptor identity and digest covered by a certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedDescriptorEvidence {
    /// Descriptor family.
    pub descriptor_family: CertifiedDescriptorFamily,
    /// Descriptor identity.
    pub descriptor_id: DescriptorId,
    /// Canonical digest of the descriptor identity payload.
    pub descriptor_digest: ContentDigest,
}

impl CertifiedDescriptorEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "descriptor_digest": self.descriptor_digest.as_str(),
            "descriptor_family": self.descriptor_family.as_str(),
            "descriptor_id": self.descriptor_id.as_str(),
        })
    }
}

/// Canonical persisted bytes for a certified spec and its certificate.
///
/// This is a storage container only. Parsing it yields [`UntrustedSpecCertificateParts`], not
/// runtime authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedSpecCertificateParts {
    spec_bytes: Vec<u8>,
    certificate_bytes: Vec<u8>,
}

impl PersistedSpecCertificateParts {
    /// Builds persisted canonical bytes from an in-memory certified authority.
    pub fn from_certified(certified: &CertifiedTypedSpec) -> Result<Self> {
        Ok(Self {
            spec_bytes: certified
                .validated_spec()
                .spec()
                .canonical_json()
                .map_err(|error| CertifyError::Spec(error.to_string()))?
                .to_vec(),
            certificate_bytes: certified.certificate.canonical_json()?.to_vec(),
        })
    }

    /// Wraps untrusted persisted bytes for parsing and later verification.
    pub fn from_untrusted_bytes(spec_bytes: Vec<u8>, certificate_bytes: Vec<u8>) -> Self {
        Self {
            spec_bytes,
            certificate_bytes,
        }
    }

    /// Returns the persisted spec bytes.
    pub fn spec_bytes(&self) -> &[u8] {
        &self.spec_bytes
    }

    /// Returns the persisted certificate bytes.
    pub fn certificate_bytes(&self) -> &[u8] {
        &self.certificate_bytes
    }

    /// Parses the persisted bytes as hostile data.
    pub fn parse_untrusted(&self) -> Result<UntrustedSpecCertificateParts> {
        let spec = spec::UntrustedTypedSpec::from_json_slice(&self.spec_bytes)
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let certificate = CertifiedSpecCertificate::from_json_slice(&self.certificate_bytes)?;
        Ok(UntrustedSpecCertificateParts { spec, certificate })
    }
}

/// Parsed persisted spec and certificate data that has not been verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntrustedSpecCertificateParts {
    pub(super) spec: spec::UntrustedTypedSpec,
    pub(super) certificate: CertifiedSpecCertificate,
}

impl UntrustedSpecCertificateParts {
    /// Returns the parsed typed spec data.
    pub fn spec(&self) -> &spec::TypedExecutionSpec {
        self.spec.spec()
    }

    /// Returns the parsed certificate data.
    pub fn certificate(&self) -> &CertifiedSpecCertificate {
        &self.certificate
    }
}
