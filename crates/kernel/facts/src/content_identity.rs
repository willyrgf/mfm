use std::collections::BTreeMap;

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
/// a fact was observed. This value instead identifies the descriptor, descriptor-derived subject
/// material, response schema, and canonical response content. Construct it only with
/// [`derive_fact_content_identity`] or one of the verification helpers, which recompute all four
/// components from hydrated canonical material.
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
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FactContentIdentityValueWire::deserialize(deserializer)?;
        parse_identity_components(
            wire.fact_descriptor_hash,
            wire.subject_material_hash,
            wire.response_schema_id,
            wire.response_hash,
        )
        .map_err(de::Error::custom)
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

    /// Returns the digest of canonical descriptor-derived subject material.
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

/// Recomputes the semantic content identity for verified fact material.
///
/// The supplied response must be canonical JSON and must conform to descriptor-declared response
/// fields. Subject material is checked against the descriptor before its digest is accepted. This
/// function intentionally accepts no caller-authored component hashes.
pub fn derive_fact_content_identity(
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV1,
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
/// descriptor-derived subject material and hydrated canonical response bytes that the reference
/// claims to identify.
pub fn verify_internal_fact_ref_content_identity(
    reference: &InternalFactRef,
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV1,
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

/// Parses canonical bytes for a fact-content identity digest payload.
pub fn parse_canonical_fact_content_identity_bytes(bytes: &[u8]) -> Result<FactContentIdentity> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let wire = serde_json::from_slice::<FactContentIdentityWire>(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    if wire.version != FACT_CONTENT_IDENTITY_DIGEST_DOMAIN {
        return Err(FactError::canonical(
            "fact content identity canonical version is unsupported",
        ));
    }

    let identity = parse_identity_components(
        wire.fact_descriptor_hash,
        wire.subject_material_hash,
        wire.response_schema_id,
        wire.response_hash,
    )?;
    let canonical = canonical_fact_content_identity_bytes(&identity)?;
    if canonical.as_bytes() != bytes {
        return Err(FactError::canonical(
            "fact content identity bytes are not canonical",
        ));
    }
    Ok(identity)
}

/// Derives the content digest for a checked fact-content identity.
pub fn fact_content_identity_digest(identity: &FactContentIdentity) -> Result<ContentDigest> {
    Ok(canonical_fact_content_identity_bytes(identity)?.content_digest())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactContentIdentityWire {
    version: String,
    fact_descriptor_hash: String,
    subject_material_hash: String,
    response_schema_id: String,
    response_hash: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactContentIdentityValueWire {
    fact_descriptor_hash: String,
    subject_material_hash: String,
    response_schema_id: String,
    response_hash: String,
}

fn parse_identity_components(
    fact_descriptor_hash: String,
    subject_material_hash: String,
    response_schema_id: String,
    response_hash: String,
) -> Result<FactContentIdentity> {
    let fact_descriptor_hash = ContentDigest::parse(&fact_descriptor_hash)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let subject_material_hash = ContentDigest::parse(&subject_material_hash)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let response_schema_id = SchemaId::parse(&response_schema_id)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let response_hash = ContentDigest::parse(&response_hash)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    Ok(FactContentIdentity::from_verified_components(
        fact_descriptor_hash,
        subject_material_hash,
        response_schema_id,
        response_hash,
    ))
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
    subject_material: &FactSubjectMaterialV1,
) -> Result<()> {
    let expected = descriptor
        .fields()
        .iter()
        .filter(|field| matches!(field.extraction(), FactFieldExtraction::Subject(_)))
        .map(|field| (field.field_id().clone(), field))
        .collect::<BTreeMap<_, _>>();
    let actual = subject_material
        .values()
        .iter()
        .map(|value| (value.field_id().clone(), value))
        .collect::<BTreeMap<_, _>>();

    if expected.len() != actual.len() {
        return Err(FactError::descriptor(
            "subject material fields do not exactly match descriptor subject fields",
        ));
    }

    for (field_id, field) in expected {
        let value = actual.get(&field_id).ok_or_else(|| {
            FactError::field(
                field_id.clone(),
                "required subject material field is missing",
            )
        })?;
        if value.value_type() != field.value_type() {
            return Err(FactError::field(
                field_id,
                "subject material value type does not match descriptor field",
            ));
        }
        validate_extracted_scalar(field, value.value())?;
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

    fn subject_material(descriptor: &FactDescriptor, chain: &str) -> FactSubjectMaterialV1 {
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
        material: &FactSubjectMaterialV1,
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
    fn canonical_identity_round_trips_and_has_a_dedicated_digest_domain() {
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
        assert_eq!(
            parse_canonical_fact_content_identity_bytes(bytes.as_bytes()).expect("parsed identity"),
            identity
        );
        let serialized = serde_json::to_vec(&identity).expect("typed value JSON");
        assert_eq!(
            serde_json::from_slice::<FactContentIdentity>(&serialized).expect("typed value decode"),
            identity
        );
        assert_eq!(
            fact_content_identity_digest(&identity).expect("identity digest"),
            bytes.content_digest()
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
    fn malformed_identity_payload_and_noncanonical_response_fail_closed() {
        let descriptor = test_descriptor(
            schema_id("mfm.test.descriptor", 1),
            schema_id("mfm.test.response", 3),
        );
        let material = subject_material(&descriptor, "bitcoin");
        assert!(
            derive_fact_content_identity(&descriptor, &material, br#"{"height":42.0}"#).is_err()
        );
        assert!(parse_canonical_fact_content_identity_bytes(
            br#"{"version":"mfm.fact.content-identity.v1","fact_descriptor_hash":"not-a-digest","subject_material_hash":"content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","response_schema_id":"schema:mfm.test.response:mfm.test.v1:sha256-jcs-v1:0303030303030303030303030303030303030303030303030303030303030303","response_hash":"content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#,
        )
        .is_err());

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
