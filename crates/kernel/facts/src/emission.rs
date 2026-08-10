use std::collections::BTreeSet;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, SemanticTypeId, StableId};
use mfm_values::MediaType;
use serde::Serialize;

use crate::{FactError, Result, MAX_FACT_EMISSIONS};

const MAX_CANONICAL_VALUE_BYTES: usize = mfm_canonical::limits::MAX_CANONICAL_JSON_BYTES;

/// Exact producer-independent retained material proposed for one fact component.
///
/// This type carries the complete certified metadata needed to mint a
/// producer-bound journal `ValueRef`, but it grants no object or producer
/// authority itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedFactValue {
    content_ref: ContentRef,
    semantic_type_id: SemanticTypeId,
    role: StableId,
    media_type: MediaType,
    evidence_contract_ref: ContentRef,
    canonical: PlainCanonicalJsonBytes,
}

impl Serialize for ProposedFactValue {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            content_ref: &'a ContentRef,
            semantic_type_id: &'a SemanticTypeId,
            role: &'a StableId,
            media_type: &'a MediaType,
            evidence_contract_ref: &'a ContentRef,
            canonical_json: &'a str,
        }
        Wire {
            content_ref: &self.content_ref,
            semantic_type_id: &self.semantic_type_id,
            role: &self.role,
            media_type: &self.media_type,
            evidence_contract_ref: &self.evidence_contract_ref,
            canonical_json: self.canonical.as_str(),
        }
        .serialize(serializer)
    }
}

impl ProposedFactValue {
    /// Constructs exact canonical material and derives its raw-byte content identity.
    pub fn new(
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        role: StableId,
        media_type: MediaType,
        evidence_contract_ref: ContentRef,
        canonical: PlainCanonicalJsonBytes,
    ) -> Result<Self> {
        if canonical.as_bytes().len() > MAX_CANONICAL_VALUE_BYTES {
            return Err(FactError::Emission(
                "canonical fact value exceeds the canonical JSON byte bound",
            ));
        }
        let digest = ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(canonical.as_bytes()),
        );
        let content_ref = ContentRef::new(schema_id, digest).map_err(|_| FactError::Identity)?;
        Ok(Self {
            content_ref,
            semantic_type_id,
            role,
            media_type,
            evidence_contract_ref,
            canonical,
        })
    }

    /// Returns the exact retained schema.
    pub const fn schema_id(&self) -> &SchemaId {
        self.content_ref.schema_id()
    }

    /// Returns the raw-byte content identity.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }

    /// Returns the exact semantic type identity.
    pub const fn semantic_type_id(&self) -> &SemanticTypeId {
        &self.semantic_type_id
    }

    /// Returns the exact producer-independent retained role.
    pub const fn role(&self) -> &StableId {
        &self.role
    }

    /// Returns the reviewed media type.
    pub const fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// Returns the exact object-evidence contract.
    pub const fn evidence_contract_ref(&self) -> &ContentRef {
        &self.evidence_contract_ref
    }

    /// Returns the exact canonical non-secret bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    fn identity(&self) -> ProposedFactValueIdentity {
        ProposedFactValueIdentity {
            content_ref: self.content_ref.clone(),
            semantic_type_id: self.semantic_type_id.clone(),
            role: self.role.clone(),
            media_type: self.media_type.clone(),
            evidence_contract_ref: self.evidence_contract_ref.clone(),
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ProposedFactValueIdentity {
    content_ref: ContentRef,
    semantic_type_id: SemanticTypeId,
    role: StableId,
    media_type: MediaType,
    evidence_contract_ref: ContentRef,
}

/// One journal-independent typed fact proposed by successful state settlement.
///
/// The proposal explicitly separates subject and response material. Runtime
/// alone derives their producer-bound retained authority, the fixed
/// `mfm.fact-claim-envelope.v1`, and the journal-owned emission after
/// store-owned preparation checks the exact certified fact slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactProposal {
    fact_slot_ordinal: u32,
    descriptor_ref: ContentRef,
    subject: ProposedFactValue,
    response: ProposedFactValue,
}

impl FactProposal {
    /// Constructs one explicit subject/response fact proposal.
    pub fn new(
        fact_slot_ordinal: u32,
        descriptor_ref: ContentRef,
        subject: ProposedFactValue,
        response: ProposedFactValue,
    ) -> Result<Self> {
        Ok(Self {
            fact_slot_ordinal,
            descriptor_ref,
            subject,
            response,
        })
    }

    /// Returns the certified homogeneous fact slot selected by this proposal.
    pub const fn fact_slot_ordinal(&self) -> u32 {
        self.fact_slot_ordinal
    }

    /// Returns the exact fact descriptor.
    pub const fn descriptor_ref(&self) -> &ContentRef {
        &self.descriptor_ref
    }

    /// Returns exact subject material.
    pub const fn subject(&self) -> &ProposedFactValue {
        &self.subject
    }

    /// Returns exact response material.
    pub const fn response(&self) -> &ProposedFactValue {
        &self.response
    }
}

/// Slot-grouped, ordered, duplicate-free proposals from one successful settlement.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FactSet(Vec<FactProposal>);

impl FactSet {
    /// Returns an empty fact set.
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    /// Returns a one-fact set.
    pub fn one(fact: FactProposal) -> Self {
        Self(vec![fact])
    }

    /// Constructs a bounded set in nondecreasing fact-slot order.
    ///
    /// A duplicate is the same descriptor plus exact subject and response
    /// content and retained-value contracts.
    /// Order within each slot group is preserved and becomes the journal
    /// emission order after runtime validation.
    pub fn try_from_iter(facts: impl IntoIterator<Item = FactProposal>) -> Result<Self> {
        let mut values = Vec::new();
        let mut identities = BTreeSet::new();
        let mut previous_slot = None;
        for fact in facts {
            if values.len() == MAX_FACT_EMISSIONS {
                return Err(FactError::Emission(
                    "successful settlement emits more than 4096 facts",
                ));
            }
            if previous_slot.is_some_and(|previous| previous > fact.fact_slot_ordinal) {
                return Err(FactError::Emission(
                    "fact proposals are not grouped in nondecreasing slot order",
                ));
            }
            previous_slot = Some(fact.fact_slot_ordinal);
            if !identities.insert((
                fact.descriptor_ref.clone(),
                fact.subject.identity(),
                fact.response.identity(),
            )) {
                return Err(FactError::Emission("duplicate fact proposal"));
            }
            values.push(fact);
        }
        Ok(Self(values))
    }

    /// Returns proposals in callback order.
    pub fn as_slice(&self) -> &[FactProposal] {
        &self.0
    }

    /// Consumes this set in callback order.
    pub fn into_vec(self) -> Vec<FactProposal> {
        self.0
    }
}
