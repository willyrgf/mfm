use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_canonical::{CanonicalValue, RecoverabilityContractV2};
use mfm_ids::{ArtifactId, ContentDigest, ContentRef, FieldPath, ObjectEvidenceDigest, SchemaId};
use mfm_journal::v1::{
    ArtifactAdmissionIntent, ArtifactAdmissionMode, ArtifactIdPreimage, AuthorityUse,
    ObjectEvidencePreimage, ObjectPathBinding, ProducerBinding, ValueRef,
};
use mfm_spec::v1::RetainedValueContract;

use super::{Result, StoreError};

/// Exact immutable object-authority identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectAuthorityKey {
    artifact_id: ArtifactId,
    content_digest: ContentDigest,
    evidence_hash: ObjectEvidenceDigest,
    evidence_contract_ref: ContentRef,
    canonical_value_ref: Vec<u8>,
}

impl ObjectAuthorityKey {
    const fn new(
        artifact_id: ArtifactId,
        content_digest: ContentDigest,
        evidence_hash: ObjectEvidenceDigest,
        evidence_contract_ref: ContentRef,
        canonical_value_ref: Vec<u8>,
    ) -> Self {
        Self {
            artifact_id,
            content_digest,
            evidence_hash,
            evidence_contract_ref,
            canonical_value_ref,
        }
    }

    /// Returns the artifact semantic identity.
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// Returns the digest of exact retained bytes.
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    /// Returns the exact object-evidence digest.
    pub const fn evidence_hash(&self) -> &ObjectEvidenceDigest {
        &self.evidence_hash
    }

    /// Returns the exact contract used to rederive the evidence digest.
    pub const fn evidence_contract_ref(&self) -> &ContentRef {
        &self.evidence_contract_ref
    }

    /// Returns the complete canonical logical authority.
    pub fn canonical_value_ref(&self) -> &[u8] {
        &self.canonical_value_ref
    }

    /// Reconstructs the complete exact key from one canonical value reference.
    #[doc(hidden)]
    pub fn from_value_ref(value_ref: &ValueRef) -> Result<Self> {
        let fields = value_ref.fields()?;
        Ok(Self::new(
            fields.artifact_id,
            fields.content_digest,
            fields.evidence_hash,
            fields.evidence_contract_ref,
            value_ref.as_bytes().to_vec(),
        ))
    }
}

/// Exact retained bytes staged alongside an append candidate.
///
/// Staging bytes grants no object authority. Authority becomes visible only with the atomic
/// journal append that carries the matching admission intent and path binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StagedObject {
    value_ref: ValueRef,
    bytes: Vec<u8>,
    key: ObjectAuthorityKey,
}

impl StagedObject {
    /// Verifies exact byte identity and complete retained-value metadata.
    pub(super) fn new(value_ref: ValueRef, bytes: Vec<u8>) -> Result<Self> {
        let key = validate_retained_object(&value_ref, &bytes)?;
        Ok(Self {
            value_ref,
            bytes,
            key,
        })
    }

    /// Returns the full producer-bound value reference.
    pub(super) const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }

    /// Returns the exact retained bytes.
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn validate_retained_object(value_ref: &ValueRef, bytes: &[u8]) -> Result<ObjectAuthorityKey> {
    let fields = value_ref.fields()?;
    let byte_length =
        u64::try_from(bytes.len()).map_err(|_| StoreError::ObjectContentMismatch {
            artifact_id: fields.artifact_id.clone(),
        })?;
    let contract = RecoverabilityContractV2::embedded()?;
    let content_digest = contract.raw_content_digest(bytes);
    let artifact_id = ArtifactIdPreimage::new(
        &fields.schema_id,
        &fields.content_digest,
        &fields.semantic_type_id,
    )?
    .artifact_id()?;
    let evidence_hash = ObjectEvidencePreimage::new(
        &fields.artifact_id,
        &fields.content_digest,
        &fields.schema_id,
        fields.byte_length,
        &fields.media_type,
        &fields.evidence_contract_ref,
    )?
    .evidence_hash()?;
    if byte_length != fields.byte_length
        || content_digest != fields.content_digest
        || artifact_id != fields.artifact_id
        || evidence_hash != fields.evidence_hash
    {
        return Err(StoreError::ObjectContentMismatch {
            artifact_id: fields.artifact_id,
        });
    }
    Ok(ObjectAuthorityKey::new(
        fields.artifact_id,
        fields.content_digest,
        fields.evidence_hash,
        fields.evidence_contract_ref,
        value_ref.as_bytes().to_vec(),
    ))
}

pub(super) fn derive_value_ref(
    value_contract: &RetainedValueContract,
    producer_binding: &ProducerBinding,
    bytes: &[u8],
) -> Result<ValueRef> {
    let byte_length =
        u64::try_from(bytes.len()).map_err(|_| StoreError::InvalidObjectAuthority {
            message: "retained object byte length cannot be represented",
        })?;
    let contract = RecoverabilityContractV2::embedded()?;
    let content_digest = contract.raw_content_digest(bytes);
    let artifact_id = ArtifactIdPreimage::new(
        value_contract.schema_id(),
        &content_digest,
        value_contract.semantic_type_id(),
    )?
    .artifact_id()?;
    let evidence_hash = ObjectEvidencePreimage::new(
        &artifact_id,
        &content_digest,
        value_contract.schema_id(),
        byte_length,
        value_contract.media_type(),
        value_contract.evidence_contract_ref(),
    )?
    .evidence_hash()?;
    ValueRef::new(
        &artifact_id,
        &content_digest,
        &evidence_hash,
        value_contract.schema_id(),
        value_contract.semantic_type_id(),
        value_contract.role(),
        byte_length,
        value_contract.media_type(),
        value_contract.evidence_contract_ref(),
        producer_binding,
    )
    .map_err(Into::into)
}

pub(super) fn validate_value_contract(
    value_contract: &RetainedValueContract,
    value_ref: &ValueRef,
) -> Result<()> {
    let fields = value_ref.fields()?;
    if value_contract.schema_id() == &fields.schema_id
        && value_contract.semantic_type_id() == &fields.semantic_type_id
        && value_contract.role() == &fields.role
        && value_contract.media_type() == fields.media_type
        && value_contract.evidence_contract_ref() == &fields.evidence_contract_ref
    {
        Ok(())
    } else {
        Err(StoreError::InvalidObjectAuthority {
            message: "retained value disagrees with its certified contract",
        })
    }
}

/// One immutable object admitted by a committed journal append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedObject {
    key: ObjectAuthorityKey,
    value_ref: ValueRef,
    bytes: Arc<[u8]>,
}

/// Untrusted portable bytes plus every full logical authority over that one content payload.
///
/// Portable decoders must retain all value references because one content payload may be used in
/// several certified roles with distinct producer bindings. Construction validates the shared
/// schema/content identity and requires a canonical nonempty authority list, but grants no live
/// object or store authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntrustedObjectPayload {
    schema_id: SchemaId,
    content_digest: ContentDigest,
    bytes: Arc<[u8]>,
    value_refs: Vec<ValueRef>,
}

impl UntrustedObjectPayload {
    /// Validates one untrusted portable object payload.
    pub fn new(
        schema_id: SchemaId,
        content_digest: ContentDigest,
        bytes: Arc<[u8]>,
        value_refs: Vec<ValueRef>,
    ) -> Result<Self> {
        if value_refs.is_empty()
            || value_refs
                .windows(2)
                .any(|pair| pair[0].as_bytes() >= pair[1].as_bytes())
        {
            return Err(StoreError::InvalidObjectAuthority {
                message: "portable object value references must be nonempty, sorted, and unique",
            });
        }
        for value_ref in &value_refs {
            let fields = value_ref.fields()?;
            if fields.schema_id != schema_id || fields.content_digest != content_digest {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "portable object authority disagrees with shared payload identity",
                });
            }
            validate_retained_object(value_ref, &bytes)?;
        }
        Ok(Self {
            schema_id,
            content_digest,
            bytes,
            value_refs,
        })
    }

    /// Returns the shared retained-value schema.
    pub const fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns the digest of the exact shared bytes.
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    /// Returns the exact shared retained bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns every full logical authority in canonical byte order.
    pub fn value_refs(&self) -> &[ValueRef] {
        &self.value_refs
    }

    pub(super) fn into_committed(self) -> Result<Vec<CommittedObject>> {
        let Self {
            bytes, value_refs, ..
        } = self;
        value_refs
            .into_iter()
            .map(|value_ref| CommittedObject::from_shared_persisted(value_ref, Arc::clone(&bytes)))
            .collect()
    }
}

impl CommittedObject {
    /// Reconstructs and verifies one exact immutable object loaded by a durable backend.
    #[doc(hidden)]
    pub fn from_persisted(value_ref: ValueRef, bytes: Vec<u8>) -> Result<Self> {
        Self::from_shared_persisted(value_ref, Arc::from(bytes))
    }

    fn from_shared_persisted(value_ref: ValueRef, bytes: Arc<[u8]>) -> Result<Self> {
        let key = validate_retained_object(&value_ref, &bytes)?;
        Ok(Self {
            key,
            value_ref,
            bytes,
        })
    }

    /// Returns the exact authority identity.
    pub const fn key(&self) -> &ObjectAuthorityKey {
        &self.key
    }

    /// Returns the complete producer-bound value reference.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }

    /// Returns exact retained bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// One content-deduplicated payload with every logical authority created or consumed by a graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedObjectPayload {
    schema_id: SchemaId,
    content_digest: ContentDigest,
    bytes: Vec<u8>,
    value_refs: Vec<ValueRef>,
}

impl PreparedObjectPayload {
    pub(super) fn new(
        schema_id: SchemaId,
        content_digest: ContentDigest,
        bytes: Vec<u8>,
        mut value_refs: Vec<ValueRef>,
    ) -> Result<Self> {
        value_refs.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        if value_refs.is_empty()
            || value_refs
                .windows(2)
                .any(|pair| pair[0].as_bytes() == pair[1].as_bytes())
        {
            return Err(StoreError::InvalidObjectAuthority {
                message: "prepared payload authorities must be nonempty and unique",
            });
        }
        for value_ref in &value_refs {
            let fields = value_ref.fields()?;
            if fields.schema_id != schema_id || fields.content_digest != content_digest {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "prepared payload authority disagrees with shared content identity",
                });
            }
            StagedObject::new(value_ref.clone(), bytes.clone())?;
        }
        Ok(Self {
            schema_id,
            content_digest,
            bytes,
            value_refs,
        })
    }

    /// Returns the shared exact schema.
    pub const fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns the digest of the one retained payload.
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    /// Returns exact retained bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns all full logical authorities in canonical byte order.
    pub fn value_refs(&self) -> &[ValueRef] {
        &self.value_refs
    }
}

/// Complete store-derived recursive object graph for one append.
///
/// Callers provide only producer-free roots. The store derives this sealed graph by recursively
/// closing retained references, authoring producer bindings, and rejecting payload omissions,
/// extras, and conflicting content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedObjectGraph {
    bindings: Vec<ObjectPathBinding>,
    admission_intents: Vec<ArtifactAdmissionIntent>,
    payloads: Vec<PreparedObjectPayload>,
}

#[derive(Clone)]
pub(super) struct PreparedAuthority {
    value_ref: ValueRef,
    bytes: Vec<u8>,
    mode: ArtifactAdmissionMode,
}

impl PreparedAuthority {
    pub(super) fn produced(value_ref: ValueRef, bytes: Vec<u8>) -> Result<Self> {
        StagedObject::new(value_ref.clone(), bytes.clone())?;
        Ok(Self {
            value_ref,
            bytes,
            mode: ArtifactAdmissionMode::AdmitOrVerifyExact,
        })
    }

    pub(super) fn preexisting(value_ref: ValueRef, bytes: Vec<u8>) -> Result<Self> {
        StagedObject::new(value_ref.clone(), bytes.clone())?;
        Ok(Self {
            value_ref,
            bytes,
            mode: ArtifactAdmissionMode::RequireExisting,
        })
    }

    pub(super) const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl PreparedObjectGraph {
    pub(super) fn prepare_for_record_values(
        record_values: &[CanonicalValue],
        authorities: Vec<PreparedAuthority>,
    ) -> Result<Self> {
        let modes = authorities
            .iter()
            .map(|authority| {
                Ok((
                    ObjectAuthorityKey::from_value_ref(&authority.value_ref)?,
                    authority.mode,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let mut referenced = Vec::<(u32, FieldPath, ValueRef)>::new();
        for (ordinal, value) in record_values.iter().enumerate() {
            collect_value_ref_paths(
                value,
                "",
                u32::try_from(ordinal).map_err(|_| StoreError::SequenceOverflow)?,
                &mut referenced,
            )?;
        }
        let bindings = referenced
            .into_iter()
            .map(|(ordinal, field_path, value_ref)| {
                let key = ObjectAuthorityKey::from_value_ref(&value_ref)?;
                let mode = modes.get(&key).ok_or(StoreError::InvalidObjectAuthority {
                    message: "candidate value reference is absent from its prepared object graph",
                })?;
                let authority_use = match mode {
                    ArtifactAdmissionMode::RequireExisting => AuthorityUse::Preexisting,
                    ArtifactAdmissionMode::AdmitOrVerifyExact => AuthorityUse::ProducedHere,
                };
                let fields = value_ref.fields()?;
                ObjectPathBinding::new(
                    ordinal,
                    &field_path,
                    authority_use,
                    &value_ref,
                    &fields.evidence_contract_ref,
                )
                .map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()?;
        Self::prepare(bindings, authorities, record_values.len())
    }

    pub(super) fn prepare(
        bindings: Vec<ObjectPathBinding>,
        authorities: Vec<PreparedAuthority>,
        record_count: usize,
    ) -> Result<Self> {
        let mut exact =
            BTreeMap::<ObjectAuthorityKey, (ValueRef, Vec<u8>, ArtifactAdmissionMode)>::new();
        for authority in authorities {
            let key = ObjectAuthorityKey::from_value_ref(&authority.value_ref)?;
            if let Some((existing_ref, existing_bytes, existing_mode)) = exact.get(&key) {
                if existing_ref.as_bytes() != authority.value_ref.as_bytes()
                    || existing_bytes != &authority.bytes
                    || *existing_mode != authority.mode
                {
                    return Err(StoreError::InvalidObjectAuthority {
                        message: "one logical authority has conflicting graph material",
                    });
                }
                continue;
            }
            exact.insert(key, (authority.value_ref, authority.bytes, authority.mode));
        }

        let mut admission_intents = Vec::with_capacity(exact.len());
        let mut grouped = BTreeMap::<(SchemaId, ContentDigest), (Vec<u8>, Vec<ValueRef>)>::new();
        for (value_ref, bytes, mode) in exact.into_values() {
            let fields = value_ref.fields()?;
            admission_intents.push(ArtifactAdmissionIntent::new(
                &value_ref,
                &fields.evidence_contract_ref,
                mode,
            )?);
            let payload = grouped
                .entry((fields.schema_id, fields.content_digest))
                .or_insert_with(|| (bytes.clone(), Vec::new()));
            if payload.0 != bytes {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "one content identity has conflicting retained bytes",
                });
            }
            payload.1.push(value_ref);
        }
        let payloads = grouped
            .into_iter()
            .map(|((schema_id, content_digest), (bytes, value_refs))| {
                PreparedObjectPayload::new(schema_id, content_digest, bytes, value_refs)
            })
            .collect::<Result<Vec<_>>>()?;
        Self::from_parts(bindings, admission_intents, payloads, record_count)
    }

    pub(super) fn from_parts(
        mut bindings: Vec<ObjectPathBinding>,
        mut admission_intents: Vec<ArtifactAdmissionIntent>,
        mut payloads: Vec<PreparedObjectPayload>,
        record_count: usize,
    ) -> Result<Self> {
        bindings.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        admission_intents.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        validate_canonical_order(
            bindings.iter().map(ObjectPathBinding::as_bytes),
            "object path bindings are not in canonical order",
        )?;
        validate_canonical_order(
            admission_intents
                .iter()
                .map(ArtifactAdmissionIntent::as_bytes),
            "artifact admission intents are not in canonical order",
        )?;
        validate_object_envelope(&bindings, &admission_intents, record_count)?;
        payloads.sort_by(|left, right| {
            (&left.schema_id, &left.content_digest).cmp(&(&right.schema_id, &right.content_digest))
        });
        if payloads.windows(2).any(|pair| {
            pair[0].schema_id == pair[1].schema_id
                && pair[0].content_digest == pair[1].content_digest
        }) {
            return Err(StoreError::InvalidObjectAuthority {
                message: "prepared object graph contains duplicate content payloads",
            });
        }

        let mut modes = BTreeMap::<ObjectAuthorityKey, ArtifactAdmissionMode>::new();
        for intent in &admission_intents {
            let fields = intent.fields()?;
            let key = ObjectAuthorityKey::from_value_ref(&fields.value_ref)?;
            if modes.insert(key, fields.mode).is_some() {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "prepared object graph contains a duplicate admission intent",
                });
            }
        }

        let mut payload_authorities = BTreeSet::new();
        for payload in &payloads {
            for value_ref in payload.value_refs() {
                if !payload_authorities.insert(ObjectAuthorityKey::from_value_ref(value_ref)?) {
                    return Err(StoreError::InvalidObjectAuthority {
                        message: "one logical authority occurs in more than one object payload",
                    });
                }
            }
        }
        if modes.keys().cloned().collect::<BTreeSet<_>>() != payload_authorities {
            return Err(StoreError::InvalidObjectAuthority {
                message: "object graph payload authorities and admission intents differ",
            });
        }

        Ok(Self {
            bindings,
            admission_intents,
            payloads,
        })
    }

    /// Returns exact candidate-record path bindings in canonical order.
    pub fn bindings(&self) -> &[ObjectPathBinding] {
        &self.bindings
    }

    /// Returns exact authority admission intents in canonical order.
    pub fn admission_intents(&self) -> &[ArtifactAdmissionIntent] {
        &self.admission_intents
    }

    /// Returns content-deduplicated payloads and every full logical authority.
    pub fn payloads(&self) -> &[PreparedObjectPayload] {
        &self.payloads
    }

    pub(super) fn bytes_for(&self, value_ref: &ValueRef) -> Result<&[u8]> {
        let key = ObjectAuthorityKey::from_value_ref(value_ref)?;
        self.payloads
            .iter()
            .find_map(|payload| {
                payload
                    .value_refs()
                    .iter()
                    .any(|candidate| {
                        ObjectAuthorityKey::from_value_ref(candidate)
                            .is_ok_and(|candidate_key| candidate_key == key)
                    })
                    .then_some(payload.bytes())
            })
            .ok_or(StoreError::ObjectNotReachable)
    }

    pub(super) fn validate_record_count(&self, record_count: usize) -> Result<()> {
        for binding in &self.bindings {
            let ordinal = usize::try_from(binding.fields()?.record_ordinal).map_err(|_| {
                StoreError::InvalidObjectAuthority {
                    message: "object binding record ordinal cannot be represented",
                }
            })?;
            if ordinal >= record_count {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "object binding record ordinal is outside the candidate batch",
                });
            }
        }
        Ok(())
    }
}

fn collect_value_ref_paths(
    value: &CanonicalValue,
    path: &str,
    record_ordinal: u32,
    output: &mut Vec<(u32, FieldPath, ValueRef)>,
) -> Result<()> {
    if !path.is_empty() {
        if let Ok(value_ref) = ValueRef::from_canonical_value(value.clone()) {
            let field_path =
                FieldPath::new(path).map_err(|_| StoreError::InvalidObjectAuthority {
                    message: "candidate retained-object field path is invalid",
                })?;
            output.push((record_ordinal, field_path, value_ref));
            return Ok(());
        }
    }
    match value {
        CanonicalValue::Object(object) => {
            for (field, nested) in object.entries() {
                let nested_path = if path.is_empty() {
                    field.to_owned()
                } else {
                    format!("{path}.{field}")
                };
                collect_value_ref_paths(nested, &nested_path, record_ordinal, output)?;
            }
        }
        CanonicalValue::Array(values) => {
            for (index, nested) in values.iter().enumerate() {
                let nested_path = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}.{index}")
                };
                collect_value_ref_paths(nested, &nested_path, record_ordinal, output)?;
            }
        }
        CanonicalValue::Null
        | CanonicalValue::Bool(_)
        | CanonicalValue::Bytes(_)
        | CanonicalValue::Signed(_)
        | CanonicalValue::Unsigned(_)
        | CanonicalValue::Decimal(_)
        | CanonicalValue::String(_) => {}
    }
    Ok(())
}

pub(super) fn validate_object_envelope(
    bindings: &[ObjectPathBinding],
    admission_intents: &[ArtifactAdmissionIntent],
    record_count: usize,
) -> Result<()> {
    validate_canonical_order(
        bindings.iter().map(ObjectPathBinding::as_bytes),
        "object path bindings are not in canonical order",
    )?;
    validate_canonical_order(
        admission_intents
            .iter()
            .map(ArtifactAdmissionIntent::as_bytes),
        "artifact admission intents are not in canonical order",
    )?;
    let mut modes = BTreeMap::<ObjectAuthorityKey, ArtifactAdmissionMode>::new();
    for intent in admission_intents {
        let fields = intent.fields()?;
        let key = ObjectAuthorityKey::from_value_ref(&fields.value_ref)?;
        if modes.insert(key, fields.mode).is_some() {
            return Err(StoreError::InvalidObjectAuthority {
                message: "object graph contains a duplicate admission intent",
            });
        }
    }
    let mut paths = BTreeSet::<(u32, mfm_ids::FieldPath)>::new();
    let mut bound_modes = BTreeMap::<ObjectAuthorityKey, ArtifactAdmissionMode>::new();
    for binding in bindings {
        let fields = binding.fields()?;
        if usize::try_from(fields.record_ordinal)
            .ok()
            .filter(|ordinal| *ordinal < record_count)
            .is_none()
        {
            return Err(StoreError::InvalidObjectAuthority {
                message: "object binding record ordinal is outside the candidate batch",
            });
        }
        if !paths.insert((fields.record_ordinal, fields.field_path)) {
            return Err(StoreError::InvalidObjectAuthority {
                message: "one candidate field path has more than one object binding",
            });
        }
        let key = ObjectAuthorityKey::from_value_ref(&fields.value_ref)?;
        let expected_mode = match fields.authority_use {
            AuthorityUse::Preexisting => ArtifactAdmissionMode::RequireExisting,
            AuthorityUse::ProducedHere => ArtifactAdmissionMode::AdmitOrVerifyExact,
        };
        if modes.get(&key) != Some(&expected_mode) {
            return Err(StoreError::InvalidObjectAuthority {
                message: "object path authority use disagrees with its admission intent",
            });
        }
        if bound_modes
            .insert(key, expected_mode)
            .is_some_and(|prior| prior != expected_mode)
        {
            return Err(StoreError::InvalidObjectAuthority {
                message: "one logical authority has conflicting path uses",
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
#[cfg(any(test, feature = "test-support"))]
pub(super) struct ObjectAuthorityState {
    objects: BTreeMap<ObjectAuthorityKey, CommittedObject>,
}

#[cfg(any(test, feature = "test-support"))]
impl ObjectAuthorityState {
    pub(super) fn from_committed(
        objects: impl IntoIterator<Item = CommittedObject>,
    ) -> Result<Self> {
        let mut state = Self::default();
        for object in objects {
            let key = object.key.clone();
            if state.objects.insert(key, object).is_some() {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "loaded object authority contains a duplicate exact identity",
                });
            }
        }
        Ok(state)
    }

    pub(super) fn apply(&mut self, prepared: &PreparedObjectGraph) -> Result<()> {
        let mut payloads = BTreeMap::<ObjectAuthorityKey, (&ValueRef, Arc<[u8]>)>::new();
        for payload in &prepared.payloads {
            let bytes = Arc::<[u8]>::from(payload.bytes());
            for value_ref in payload.value_refs() {
                payloads.insert(
                    ObjectAuthorityKey::from_value_ref(value_ref)?,
                    (value_ref, Arc::clone(&bytes)),
                );
            }
        }
        for intent in &prepared.admission_intents {
            let fields = intent.fields()?;
            let value_fields = fields.value_ref.fields()?;
            let key = ObjectAuthorityKey::from_value_ref(&fields.value_ref)?;
            let candidate = payloads
                .get(&key)
                .ok_or(StoreError::InvalidObjectAuthority {
                    message: "object admission intent has no exact graph payload",
                })?;
            match (fields.mode, self.objects.get(&key)) {
                (ArtifactAdmissionMode::RequireExisting, Some(existing)) => {
                    if existing.value_ref.as_bytes() != candidate.0.as_bytes()
                        || existing.bytes.as_ref() != candidate.1.as_ref()
                    {
                        return Err(StoreError::ObjectAuthorityConflict {
                            artifact_id: value_fields.artifact_id,
                        });
                    }
                }
                (ArtifactAdmissionMode::RequireExisting, None) => {
                    return Err(StoreError::MissingObjectAuthority {
                        artifact_id: value_fields.artifact_id,
                    });
                }
                (ArtifactAdmissionMode::AdmitOrVerifyExact, Some(existing)) => {
                    if existing.bytes.as_ref() != candidate.1.as_ref()
                        || existing.value_ref.as_bytes() != candidate.0.as_bytes()
                    {
                        return Err(StoreError::ObjectAuthorityConflict {
                            artifact_id: value_fields.artifact_id,
                        });
                    }
                }
                (ArtifactAdmissionMode::AdmitOrVerifyExact, None) => {
                    let object = CommittedObject::from_shared_persisted(
                        (*candidate.0).clone(),
                        Arc::clone(&candidate.1),
                    )?;
                    self.objects.insert(key, object);
                }
            }
        }
        Ok(())
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &CommittedObject> {
        self.objects.values()
    }
}

fn validate_canonical_order<'a>(
    values: impl IntoIterator<Item = &'a [u8]>,
    message: &'static str,
) -> Result<()> {
    let mut previous: Option<&[u8]> = None;
    for value in values {
        if previous.is_some_and(|prior| prior >= value) {
            return Err(StoreError::InvalidObjectAuthority { message });
        }
        previous = Some(value);
    }
    Ok(())
}

#[cfg(test)]
#[path = "objects_tests.rs"]
mod tests;
