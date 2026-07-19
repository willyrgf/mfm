use mfm_canonical::{
    sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue, PlainCanonicalJsonBytes,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, SchemaId, SemanticTypeId};
use mfm_values::{FieldDescriptor, MfmValue, SchemaDescriptor, SchemaShape, ValueError};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::extraction::{extract_field_scalar, validate_extracted_scalar};
use crate::*;

/// Canonical digest domain for [`FactContentIdentity`].
pub const FACT_CONTENT_IDENTITY_DIGEST_DOMAIN: &str = "mfm.fact.content-identity.v1";

/// Verified semantic identity for fact content, independent of its stored claim occurrence.
///
/// A fact claim, internal reference, artifact id, run coordinate, and store order identify where
/// a fact was observed. This value instead identifies the descriptor, complete canonical typed
/// subject material, response schema, and canonical response content. Construct it only with
/// [`derive_fact_content_identity`] or one of the verification helpers, which recompute all four
/// components from hydrated canonical material. Direct deserialization is intentionally rejected:
/// the compact serialized fields do not carry enough evidence to establish that relationship.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactContentIdentity {
    fact_descriptor_hash: ContentDigest,
    subject_material_hash: ContentDigest,
    response_schema_id: SchemaId,
    response_hash: ContentDigest,
}

impl Serialize for FactContentIdentity {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        FactContentIdentityValueWire {
            fact_descriptor_hash: self.fact_descriptor_hash.as_str().to_owned(),
            subject_material_hash: self.subject_material_hash.as_str().to_owned(),
            response_schema_id: self.response_schema_id.as_str().to_owned(),
            response_hash: self.response_hash.as_str().to_owned(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FactContentIdentity {
    fn deserialize<D>(_deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Err(de::Error::custom(
            "FactContentIdentity must be reconstructed from verified hydrated fact material",
        ))
    }
}

impl MfmValue for FactContentIdentity {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        mfm_values::framework_value_descriptor(
            Self::semantic_id()?,
            "mfm.fact.content_identity",
            SchemaShape::named_struct(vec![
                FieldDescriptor::required("fact_descriptor_hash", SchemaShape::String),
                FieldDescriptor::required("response_hash", SchemaShape::String),
                FieldDescriptor::required("response_schema_id", SchemaShape::String),
                FieldDescriptor::required("subject_material_hash", SchemaShape::String),
            ])?,
            "mfm_facts::FactContentIdentity",
        )
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.fact",
            "content-identity",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"semantic:mfm.fact:content-identity:1"),
        )
        .map_err(|error| ValueError::Identity(error.to_string()))
    }
}

impl FactContentIdentity {
    fn from_verified_components(
        fact_descriptor_hash: ContentDigest,
        subject_material_hash: ContentDigest,
        response_schema_id: SchemaId,
        response_hash: ContentDigest,
    ) -> Self {
        Self {
            fact_descriptor_hash,
            subject_material_hash,
            response_schema_id,
            response_hash,
        }
    }

    /// Returns the digest of the canonical fact descriptor bytes.
    pub const fn fact_descriptor_hash(&self) -> &ContentDigest {
        &self.fact_descriptor_hash
    }

    /// Returns the digest of the complete canonical typed subject material.
    pub const fn subject_material_hash(&self) -> &ContentDigest {
        &self.subject_material_hash
    }

    /// Returns the schema id of the canonical hydrated response.
    pub const fn response_schema_id(&self) -> &SchemaId {
        &self.response_schema_id
    }

    /// Returns the digest of the canonical hydrated response bytes.
    pub const fn response_hash(&self) -> &ContentDigest {
        &self.response_hash
    }
}

/// Opaque persisted evidence for a checked [`FactContentIdentity`].
///
/// This carrier deliberately does not expose an identity or make its compact components
/// authoritative. Collection code may create it only from a verified identity. A later consumer
/// must rederive identity from a descriptor, typed subject, and hydrated response through
/// [`Self::verify_against_typed_values`] before it can obtain a [`FactContentIdentity`].
///
/// It exists for typed persisted values such as receipts: those values must deserialize before
/// their later consumer has access to the hydrated fact material needed to verify the compact
/// identity wire. It is fact-layer infrastructure, not a domain-specific identity type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactContentIdentityEvidence {
    fact_descriptor_hash: String,
    subject_material_hash: String,
    response_schema_id: String,
    response_hash: String,
}

impl FactContentIdentityEvidence {
    /// Copies compact wire components from an identity already verified against hydrated material.
    pub fn from_verified(identity: &FactContentIdentity) -> Self {
        Self {
            fact_descriptor_hash: identity.fact_descriptor_hash().as_str().to_owned(),
            subject_material_hash: identity.subject_material_hash().as_str().to_owned(),
            response_schema_id: identity.response_schema_id().as_str().to_owned(),
            response_hash: identity.response_hash().as_str().to_owned(),
        }
    }

    /// Re-derives identity from hydrated typed fact material and returns it only on exact match.
    ///
    /// `Ok(None)` means the hydrated fact is valid fact material but not the fact content pinned
    /// by this evidence. Callers use that outcome to filter a candidate before any ordering.
    pub fn verify_against_typed_values<S, R>(
        &self,
        descriptor: &FactDescriptor,
        subject: &S,
        response: &R,
    ) -> Result<Option<FactContentIdentity>>
    where
        S: Serialize,
        R: Serialize,
    {
        let identity =
            derive_fact_content_identity_from_typed_values(descriptor, subject, response)?;
        Ok((Self::from_verified(&identity) == *self).then_some(identity))
    }

    /// Returns whether an indexed reference carries the same compact content components.
    ///
    /// This is a bounded query-narrowing check only. Consumers must still hydrate the response
    /// artifact and call [`Self::verify_against_typed_values`] before trusting the content.
    pub fn matches_internal_ref(&self, reference: &InternalFactRef) -> bool {
        self.fact_descriptor_hash == reference.fact_descriptor_hash().as_str()
            && self.subject_material_hash == reference.subject_material_hash().as_str()
            && self.response_schema_id == reference.response_schema_id().as_str()
            && self.response_hash == reference.response_hash().as_str()
    }

    pub(crate) fn matches_descriptor_hash(&self, descriptor_hash: &ContentDigest) -> bool {
        self.fact_descriptor_hash == descriptor_hash.as_str()
    }

    pub(crate) fn canonical_query_value(&self) -> Result<CanonicalValue> {
        CanonicalValue::object([
            (
                "fact_descriptor_hash",
                CanonicalValue::String(self.fact_descriptor_hash.clone()),
            ),
            (
                "subject_material_hash",
                CanonicalValue::String(self.subject_material_hash.clone()),
            ),
            (
                "response_schema_id",
                CanonicalValue::String(self.response_schema_id.clone()),
            ),
            (
                "response_hash",
                CanonicalValue::String(self.response_hash.clone()),
            ),
        ])
        .map_err(|error| FactError::canonical(error.to_string()))
    }
}

impl<'de> Deserialize<'de> for FactContentIdentityEvidence {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FactContentIdentityValueWire::deserialize(deserializer)?;
        validate_fact_content_identity_wire(&wire).map_err(de::Error::custom)?;
        Ok(Self {
            fact_descriptor_hash: wire.fact_descriptor_hash,
            subject_material_hash: wire.subject_material_hash,
            response_schema_id: wire.response_schema_id,
            response_hash: wire.response_hash,
        })
    }
}

impl MfmValue for FactContentIdentityEvidence {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        mfm_values::framework_value_descriptor(
            Self::semantic_id()?,
            "mfm.fact.content_identity_evidence",
            SchemaShape::named_struct(vec![
                FieldDescriptor::required("fact_descriptor_hash", SchemaShape::String),
                FieldDescriptor::required("response_hash", SchemaShape::String),
                FieldDescriptor::required("response_schema_id", SchemaShape::String),
                FieldDescriptor::required("subject_material_hash", SchemaShape::String),
            ])?,
            "mfm_facts::FactContentIdentityEvidence",
        )
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.fact",
            "content-identity-evidence",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"semantic:mfm.fact:content-identity-evidence:1"),
        )
        .map_err(|error| ValueError::Identity(error.to_string()))
    }
}

/// Recomputes the semantic content identity for verified fact material.
///
/// The supplied response must be canonical JSON and must conform to descriptor-declared response
/// fields. Subject material is checked against the descriptor before its digest is accepted. This
/// function intentionally accepts no caller-authored component hashes.
pub fn derive_fact_content_identity(
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV2,
    response_bytes: &[u8],
) -> Result<FactContentIdentity> {
    validate_descriptor(descriptor)?;
    validate_subject_material(descriptor, subject_material)?;
    let response = parse_canonical_fact_response_bytes(descriptor, response_bytes)?;
    validate_response(descriptor, &response)?;
    let response_bytes = PlainCanonicalJsonBytes::from_canonical_json_slice(response_bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;

    Ok(FactContentIdentity::from_verified_components(
        fact_descriptor_hash(descriptor)?,
        subject_material_hash(subject_material)?,
        descriptor.response_schema_id().clone(),
        response_bytes.content_digest(),
    ))
}

/// Derives fact-content identity from concrete typed fact subject and response values.
///
/// This is the typed-value counterpart to [`derive_fact_content_identity`]. It first derives
/// descriptor-checked subject material and canonical response bytes, then delegates to the
/// hydrated-material boundary. Callers must use concrete fact material that has already passed
/// the corresponding typed fact contract; this helper accepts no caller-authored identity
/// components.
pub fn derive_fact_content_identity_from_typed_values<S, R>(
    descriptor: &FactDescriptor,
    subject: &S,
    response: &R,
) -> Result<FactContentIdentity>
where
    S: Serialize,
    R: Serialize,
{
    let subject =
        serde_json::to_value(subject).map_err(|error| FactError::canonical(error.to_string()))?;
    let subject = typed_fact_subject_value(descriptor, &subject)?;
    let subject_material = extract_subject_material(descriptor, &subject)?;
    let response =
        serde_json::to_string(response).map_err(|error| FactError::canonical(error.to_string()))?;
    let response = PlainCanonicalJsonBytes::from_json_str(&response)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    derive_fact_content_identity(descriptor, &subject_material, response.as_bytes())
}

/// Re-derives and verifies a serialized identity against concrete typed fact material.
///
/// This is intentionally the only admission path for persisted compact identity fields. The
/// identity wire is parsed only so it can be compared with an identity re-derived from the
/// descriptor-checked subject and canonical response; it is never trusted as a constructor.
pub fn verify_serialized_fact_content_identity_from_typed_values<S, R>(
    serialized_identity: &serde_json::Value,
    descriptor: &FactDescriptor,
    subject: &S,
    response: &R,
) -> Result<FactContentIdentity>
where
    S: Serialize,
    R: Serialize,
{
    let supplied: FactContentIdentityValueWire =
        serde_json::from_value(serialized_identity.clone())
            .map_err(|error| FactError::canonical(error.to_string()))?;
    let supplied = fact_content_identity_from_wire(supplied)?;
    let derived = derive_fact_content_identity_from_typed_values(descriptor, subject, response)?;
    if supplied != derived {
        return Err(FactError::descriptor(
            "serialized fact content identity did not match verified typed fact material",
        ));
    }
    Ok(derived)
}

/// Recomputes and verifies semantic content identity for a recorded fact claim.
///
/// The caller must hydrate the response artifact before invoking this helper. The claim's compact
/// hashes are checked only after the descriptor, subject material, response schema, and response
/// bytes have been recomputed from that hydrated material.
pub fn verify_fact_claim_content_identity(
    claim: &FactClaim,
    descriptor: &FactDescriptor,
    response_bytes: &[u8],
) -> Result<FactContentIdentity> {
    let subject_material =
        parse_canonical_fact_subject_material_bytes(claim.subject().subject_material().as_bytes())?;
    let identity = derive_fact_content_identity(descriptor, &subject_material, response_bytes)?;

    if claim.fact_descriptor_hash() != identity.fact_descriptor_hash() {
        return Err(FactError::descriptor(
            "fact claim descriptor hash does not match hydrated fact content",
        ));
    }
    if claim.subject().subject_material_hash() != identity.subject_material_hash() {
        return Err(FactError::descriptor(
            "fact claim subject material hash does not match hydrated fact content",
        ));
    }
    if claim.response().response_schema_id() != identity.response_schema_id() {
        return Err(FactError::descriptor(
            "fact claim response schema id does not match hydrated fact content",
        ));
    }
    if claim.response().response_hash() != identity.response_hash() {
        return Err(FactError::descriptor(
            "fact claim response hash does not match hydrated fact content",
        ));
    }

    Ok(identity)
}

/// Recomputes and verifies semantic content identity for an indexed fact reference.
///
/// Internal references intentionally retain only compact hashes, so callers must provide the
/// complete canonical typed subject material and hydrated canonical response bytes that the
/// reference claims to identify.
pub fn verify_internal_fact_ref_content_identity(
    reference: &InternalFactRef,
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV2,
    response_bytes: &[u8],
) -> Result<FactContentIdentity> {
    let identity = derive_fact_content_identity(descriptor, subject_material, response_bytes)?;

    if reference.fact_descriptor_hash() != identity.fact_descriptor_hash() {
        return Err(FactError::descriptor(
            "internal fact ref descriptor hash does not match hydrated fact content",
        ));
    }
    if reference.subject_material_hash() != identity.subject_material_hash() {
        return Err(FactError::descriptor(
            "internal fact ref subject material hash does not match hydrated fact content",
        ));
    }
    if reference.response_schema_id() != identity.response_schema_id() {
        return Err(FactError::descriptor(
            "internal fact ref response schema id does not match hydrated fact content",
        ));
    }
    if reference.response_hash() != identity.response_hash() {
        return Err(FactError::descriptor(
            "internal fact ref response hash does not match hydrated fact content",
        ));
    }

    Ok(identity)
}

/// Returns canonical bytes used to derive a fact-content identity digest.
pub fn canonical_fact_content_identity_bytes(
    identity: &FactContentIdentity,
) -> Result<CanonicalJsonBytes> {
    canonical_fact_content_identity_value(identity)
        .map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for a checked fact-content identity.
pub fn fact_content_identity_digest(identity: &FactContentIdentity) -> Result<ContentDigest> {
    Ok(canonical_fact_content_identity_bytes(identity)?.content_digest())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactContentIdentityValueWire {
    fact_descriptor_hash: String,
    subject_material_hash: String,
    response_schema_id: String,
    response_hash: String,
}

fn fact_content_identity_from_wire(
    wire: FactContentIdentityValueWire,
) -> Result<FactContentIdentity> {
    validate_fact_content_identity_wire(&wire)?;
    let fact_descriptor_hash = wire
        .fact_descriptor_hash
        .parse()
        .map_err(|error: mfm_ids::IdentityError| FactError::canonical(error.to_string()))?;
    let subject_material_hash = wire
        .subject_material_hash
        .parse()
        .map_err(|error: mfm_ids::IdentityError| FactError::canonical(error.to_string()))?;
    let response_schema_id = wire
        .response_schema_id
        .parse()
        .map_err(|error: mfm_ids::IdentityError| FactError::canonical(error.to_string()))?;
    let response_hash = wire
        .response_hash
        .parse()
        .map_err(|error: mfm_ids::IdentityError| FactError::canonical(error.to_string()))?;
    Ok(FactContentIdentity::from_verified_components(
        fact_descriptor_hash,
        subject_material_hash,
        response_schema_id,
        response_hash,
    ))
}

fn validate_fact_content_identity_wire(wire: &FactContentIdentityValueWire) -> Result<()> {
    wire.fact_descriptor_hash
        .parse::<ContentDigest>()
        .map_err(|error| FactError::canonical(error.to_string()))?;
    wire.subject_material_hash
        .parse::<ContentDigest>()
        .map_err(|error| FactError::canonical(error.to_string()))?;
    wire.response_schema_id
        .parse::<SchemaId>()
        .map_err(|error| FactError::canonical(error.to_string()))?;
    wire.response_hash
        .parse::<ContentDigest>()
        .map_err(|error| FactError::canonical(error.to_string()))?;
    Ok(())
}

fn canonical_fact_content_identity_value(identity: &FactContentIdentity) -> Result<CanonicalValue> {
    CanonicalValue::object([
        (
            "version",
            CanonicalValue::String(FACT_CONTENT_IDENTITY_DIGEST_DOMAIN.to_owned()),
        ),
        (
            "fact_descriptor_hash",
            CanonicalValue::String(identity.fact_descriptor_hash().as_str().to_owned()),
        ),
        (
            "subject_material_hash",
            CanonicalValue::String(identity.subject_material_hash().as_str().to_owned()),
        ),
        (
            "response_schema_id",
            CanonicalValue::String(identity.response_schema_id().as_str().to_owned()),
        ),
        (
            "response_hash",
            CanonicalValue::String(identity.response_hash().as_str().to_owned()),
        ),
    ])
    .map_err(|error| FactError::canonical(error.to_string()))
}

fn validate_subject_material(
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV2,
) -> Result<()> {
    let typed_subject = crate::codec::typed_subject_from_material(descriptor, subject_material)?;
    let checked = extract_subject_material(descriptor, &typed_subject)?;
    if canonical_fact_subject_material_bytes(subject_material)?
        != canonical_fact_subject_material_bytes(&checked)?
    {
        return Err(FactError::descriptor(
            "subject material does not match its checked typed subject",
        ));
    }
    Ok(())
}

fn validate_response(descriptor: &FactDescriptor, response: &CanonicalValue) -> Result<()> {
    for field in descriptor
        .fields()
        .iter()
        .filter(|field| matches!(field.extraction(), FactFieldExtraction::Response(_)))
    {
        match extract_field_scalar(field, &CanonicalValue::Null, response, None)? {
            Some(value) => validate_extracted_scalar(field, &value)?,
            None if field.required() => {
                return Err(FactError::field(
                    field.field_id().clone(),
                    "required response field is missing",
                ));
            }
            None => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use mfm_canonical::CanonicalValue;
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion,
        DigestAlgorithm, DigestBytes, EventId, NodeId, RunId,
    };
    use mfm_values::{MfmValue, NumberPolicy, SecretPolicy};

    use super::*;

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn schema_id(name: &str, byte: u8) -> SchemaId {
        SchemaId::new(
            name,
            "mfm.test.v1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
        .expect("schema id")
    }

    fn test_descriptor(
        descriptor_schema_id: SchemaId,
        response_schema_id: SchemaId,
    ) -> FactDescriptor {
        FactDescriptor::new(
            FactKind::new("chain.head").expect("fact kind"),
            descriptor_schema_id,
            schema_id("mfm.test.subject", 2),
            response_schema_id,
            vec![
                FactFieldDescriptor::new(
                    FactFieldId::new("subject.chain").expect("field id"),
                    FactFieldValueType::String,
                    FactFieldExtraction::Subject(CanonicalValuePath::new("chain").expect("path")),
                    FactFieldPolicy::new(
                        vec![FactQueryOperator::Equal],
                        FactFieldExposure::Returnable,
                    )
                    .required(),
                )
                .expect("subject field"),
                FactFieldDescriptor::new(
                    FactFieldId::new("result.height").expect("field id"),
                    FactFieldValueType::UnsignedInteger,
                    FactFieldExtraction::Response(CanonicalValuePath::new("height").expect("path")),
                    FactFieldPolicy::new(
                        vec![FactQueryOperator::Equal],
                        FactFieldExposure::Returnable,
                    )
                    .required(),
                )
                .expect("response field"),
            ],
            Vec::new(),
        )
        .expect("descriptor")
    }

    fn subject_material(descriptor: &FactDescriptor, chain: &str) -> FactSubjectMaterialV2 {
        extract_subject_material(
            descriptor,
            &CanonicalValue::object([("chain", CanonicalValue::String(chain.to_owned()))])
                .expect("subject"),
        )
        .expect("subject material")
    }

    fn producer() -> FactProducerProvenance {
        FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.test",
                "fact-read",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([11; 32]),
            )
            .expect("capability kind"),
            CapabilityVersion::new("mfm.test.capability.v1").expect("capability version"),
            AdapterKind::new(
                "mfm.test",
                "fact-adapter",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([12; 32]),
            )
            .expect("adapter kind"),
            AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        )
    }

    fn claim(
        descriptor: &FactDescriptor,
        material: &FactSubjectMaterialV2,
        descriptor_hash: ContentDigest,
        response_hash: ContentDigest,
    ) -> FactClaim {
        FactClaim::new(FactClaimParts {
            visibility: FactVisibility::indexed_default(FactAudience::Platform),
            fact_kind: descriptor.fact_kind().clone(),
            fact_descriptor_hash: descriptor_hash,
            subject: fact_subject_evidence_from_material(descriptor, material)
                .expect("subject evidence"),
            observed_at: None,
            request: None,
            response: FactResponseEvidence::new(
                descriptor.response_schema_id().clone(),
                response_hash,
                ArtifactId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([13; 32]),
                ),
                digest(14),
            ),
            producer: producer(),
        })
        .expect("claim")
    }

    fn reference(claim: &FactClaim, run_byte: u8) -> InternalFactRef {
        InternalFactRef::from_claim(
            FactClaimId::new(
                RunId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([run_byte; 32]),
                ),
                1,
                0,
            )
            .expect("claim id"),
            EventId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([run_byte.wrapping_add(1); 32]),
            ),
            "2026-07-15T00:00:00Z".to_owned(),
            NodeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([run_byte.wrapping_add(2); 32]),
            ),
            claim,
        )
        .expect("indexed ref")
        .expect("platform ref")
    }

    #[test]
    fn identity_ignores_claim_occurrence_and_changes_for_every_semantic_component() {
        let descriptor = test_descriptor(
            schema_id("mfm.test.descriptor", 1),
            schema_id("mfm.test.response", 3),
        );
        let material = subject_material(&descriptor, "bitcoin");
        let response = br#"{"height":42}"#;
        let identity =
            derive_fact_content_identity(&descriptor, &material, response).expect("identity");

        let recorded = claim(
            &descriptor,
            &material,
            fact_descriptor_hash(&descriptor).expect("descriptor hash"),
            PlainCanonicalJsonBytes::from_canonical_json_slice(response)
                .expect("response")
                .content_digest(),
        );
        assert_eq!(
            verify_internal_fact_ref_content_identity(
                &reference(&recorded, 20),
                &descriptor,
                &material,
                response,
            )
            .expect("first ref"),
            verify_internal_fact_ref_content_identity(
                &reference(&recorded, 30),
                &descriptor,
                &material,
                response,
            )
            .expect("second ref")
        );

        let descriptor_changed = test_descriptor(
            schema_id("mfm.test.descriptor.changed", 4),
            schema_id("mfm.test.response", 3),
        );
        assert_ne!(
            identity,
            derive_fact_content_identity(&descriptor_changed, &material, response)
                .expect("changed descriptor")
        );
        assert_ne!(
            identity,
            derive_fact_content_identity(
                &descriptor,
                &subject_material(&descriptor, "ethereum"),
                response,
            )
            .expect("changed subject")
        );
        assert_ne!(
            identity,
            derive_fact_content_identity(
                &test_descriptor(
                    schema_id("mfm.test.descriptor", 1),
                    schema_id("mfm.test.response.changed", 5)
                ),
                &material,
                response,
            )
            .expect("changed response schema")
        );
        assert_ne!(
            identity,
            derive_fact_content_identity(&descriptor, &material, br#"{"height":43}"#)
                .expect("changed response")
        );
    }

    #[test]
    fn canonical_identity_has_a_dedicated_digest_domain_and_rejects_direct_decode() {
        let descriptor = test_descriptor(
            schema_id("mfm.test.descriptor", 1),
            schema_id("mfm.test.response", 3),
        );
        let identity = derive_fact_content_identity(
            &descriptor,
            &subject_material(&descriptor, "bitcoin"),
            br#"{"height":42}"#,
        )
        .expect("identity");
        let bytes = canonical_fact_content_identity_bytes(&identity).expect("canonical bytes");

        assert!(bytes.as_str().contains(FACT_CONTENT_IDENTITY_DIGEST_DOMAIN));
        let serialized = serde_json::to_vec(&identity).expect("typed value JSON");
        assert!(serde_json::from_slice::<FactContentIdentity>(&serialized).is_err());
        assert_eq!(
            fact_content_identity_digest(&identity).expect("identity digest"),
            bytes.content_digest()
        );
    }

    #[test]
    fn typed_value_identity_verification_rederives_compact_identity() {
        let descriptor = test_descriptor(
            schema_id("mfm.test.descriptor", 1),
            schema_id("mfm.test.response", 3),
        );
        let subject = serde_json::json!({ "chain": "bitcoin" });
        let response = serde_json::json!({ "height": 42 });

        let identity =
            derive_fact_content_identity_from_typed_values(&descriptor, &subject, &response)
                .expect("typed identity");
        assert_eq!(
            identity,
            derive_fact_content_identity(
                &descriptor,
                &subject_material(&descriptor, "bitcoin"),
                br#"{"height":42}"#,
            )
            .expect("hydrated identity")
        );

        let serialized = serde_json::to_value(&identity).expect("serialized identity");
        assert_eq!(
            verify_serialized_fact_content_identity_from_typed_values(
                &serialized,
                &descriptor,
                &subject,
                &response,
            )
            .expect("verified identity"),
            identity
        );

        let mut tampered = serialized;
        tampered["response_hash"] = serde_json::json!(digest(99).as_str());
        assert!(verify_serialized_fact_content_identity_from_typed_values(
            &tampered,
            &descriptor,
            &subject,
            &response,
        )
        .is_err());
    }

    #[test]
    fn opaque_identity_evidence_requires_rederivation_before_it_yields_an_identity() {
        let descriptor = test_descriptor(
            schema_id("mfm.test.descriptor", 1),
            schema_id("mfm.test.response", 3),
        );
        let subject = serde_json::json!({ "chain": "bitcoin" });
        let response = serde_json::json!({ "height": 42 });
        let identity =
            derive_fact_content_identity_from_typed_values(&descriptor, &subject, &response)
                .expect("typed identity");
        let evidence = FactContentIdentityEvidence::from_verified(&identity);
        let decoded: FactContentIdentityEvidence =
            serde_json::from_value(serde_json::to_value(&evidence).expect("serialize evidence"))
                .expect("decode opaque evidence");

        assert_eq!(
            decoded
                .verify_against_typed_values(&descriptor, &subject, &response)
                .expect("rederive evidence"),
            Some(identity)
        );

        let mut tampered = serde_json::to_value(&evidence).expect("serialize evidence");
        tampered["response_hash"] = serde_json::json!(digest(99).as_str());
        let tampered: FactContentIdentityEvidence =
            serde_json::from_value(tampered).expect("well-formed opaque evidence");
        assert_eq!(
            tampered
                .verify_against_typed_values(&descriptor, &subject, &response)
                .expect("rederive tampered evidence"),
            None
        );
    }

    #[test]
    fn verification_rejects_claim_or_ref_hashes_that_disagree_with_hydrated_content() {
        let descriptor = test_descriptor(
            schema_id("mfm.test.descriptor", 1),
            schema_id("mfm.test.response", 3),
        );
        let material = subject_material(&descriptor, "bitcoin");
        let response = br#"{"height":42}"#;
        let bad_claim = claim(&descriptor, &material, digest(99), digest(98));

        assert!(verify_fact_claim_content_identity(&bad_claim, &descriptor, response).is_err());
        assert!(verify_internal_fact_ref_content_identity(
            &reference(&bad_claim, 20),
            &descriptor,
            &material,
            response,
        )
        .is_err());
    }

    #[test]
    fn forged_identity_payload_and_noncanonical_response_fail_closed() {
        let descriptor = test_descriptor(
            schema_id("mfm.test.descriptor", 1),
            schema_id("mfm.test.response", 3),
        );
        let material = subject_material(&descriptor, "bitcoin");
        assert!(
            derive_fact_content_identity(&descriptor, &material, br#"{"height":42.0}"#).is_err()
        );
        let forged = serde_json::json!({
            "fact_descriptor_hash": digest(97).as_str(),
            "subject_material_hash": digest(98).as_str(),
            "response_schema_id": schema_id("mfm.test.forged", 99).as_str(),
            "response_hash": digest(100).as_str(),
        });
        assert!(serde_json::from_value::<FactContentIdentity>(forged).is_err());

        let descriptor = FactContentIdentity::schema_descriptor().expect("value descriptor");
        assert_eq!(
            descriptor.identity.persisted_surface.secrets,
            SecretPolicy::NoSecrets
        );
        assert_eq!(
            descriptor.identity.persisted_surface.numbers,
            NumberPolicy::NoFloats
        );
    }
}
