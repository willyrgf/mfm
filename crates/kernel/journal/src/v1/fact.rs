use mfm_canonical::CanonicalValue;
use mfm_facts::FactSelectionRequest;
use mfm_ids::{
    ContentRef, FactContentIdentityDigest, FactLogicalIdentityDigest, FactQueryDigest, StoreEpoch,
    StoreScopeId, TenantScopeId,
};

use super::codec::{
    array_field, cv_array, cv_content_ref, cv_decimal, cv_string, define_schema_value,
    domain_digest, object, required_field, store_epoch_field, store_scope_id_field, string_field,
    tenant_scope_id_field, u32_field, unsigned_field,
};
use super::{AuthorizationRef, Result, TransitionRef, ValueRef};

define_schema_value! {
    /// Store-authored fixed envelope for one emitted fact claim.
    pub struct FactClaimEnvelope => "mfm.fact-claim-envelope.v1";
    /// One fact emitted atomically by a successful transition settlement.
    pub struct FactEmission => "mfm.fact-emission.v1";
    /// Exact fact coordinate inside a producing transition.
    pub struct FactRef => "mfm.fact-ref.v1";
    /// Store routing copy for all facts emitted by one transition.
    pub struct FactPublicationRouting => "mfm.fact-publication-routing.v1";
    /// Closed tenant coordinate assigned to one journal commit.
    pub struct TenantFactCoordinate => "mfm.tenant-fact-coordinate.v1";
    /// Exact tenant fact frontier.
    pub struct TenantFactFrontier => "mfm.tenant-fact-frontier.v1";
    /// Complete response at an exact fact frontier.
    pub struct FactSelectionResponse => "mfm.fact-selection-response.v1";
    /// One query result in a fact-selection response.
    pub struct FactSelectionResult => "mfm.fact-selection-result.v1";
    /// One fully rehydrated selected fact.
    pub struct SelectedFact => "mfm.selected-fact.v1";
    /// Completeness claim attached to fact selection.
    pub struct FactSelectionCompleteness => "mfm.fact-selection-completeness.v1";
    /// Semantic fact-content identity preimage.
    pub struct FactContentIdentityPreimage => "mfm.fact-content-identity-preimage.v1";
    /// Semantic fact-logical identity preimage.
    pub struct FactLogicalIdentityPreimage => "mfm.fact-logical-identity-preimage.v1";
}

/// Typed fields of one fixed fact claim envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactClaimEnvelopeFields {
    /// Frozen fact descriptor.
    pub fact_descriptor_ref: ContentRef,
    /// Full retained fact subject authority.
    pub subject_ref: ValueRef,
    /// Full retained fact response authority.
    pub response_ref: ValueRef,
}

impl FactClaimEnvelope {
    /// Returns the deterministic retained contract for store-authored fact claims.
    pub fn retained_contract() -> Result<super::RetainedValueContract> {
        super::runtime_retained_contract(
            "mfm.fact-claim-envelope.v1",
            "fact-claim-envelope",
            "mfm.runtime.fact-claim-envelope",
        )
    }

    /// Constructs one store-authored fixed fact claim envelope.
    pub fn new(
        fact_descriptor_ref: &ContentRef,
        subject_ref: &ValueRef,
        response_ref: &ValueRef,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.fact-claim-envelope.v1".to_owned()),
            ),
            ("fact_descriptor_ref", cv_content_ref(fact_descriptor_ref)?),
            ("subject_ref", subject_ref.canonical_value()?),
            ("response_ref", response_ref.canonical_value()?),
        ])?)
    }

    /// Projects every fixed fact claim field.
    pub fn fields(&self) -> Result<FactClaimEnvelopeFields> {
        Ok(FactClaimEnvelopeFields {
            fact_descriptor_ref: super::codec::content_ref_field(self, "fact_descriptor_ref")?,
            subject_ref: ValueRef::from_canonical_value(required_field(self, "subject_ref")?)?,
            response_ref: ValueRef::from_canonical_value(required_field(self, "response_ref")?)?,
        })
    }
}

/// Typed fields of a fact emission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactEmissionFields {
    /// Dense actual ordinal within the producing settlement.
    pub emission_ordinal: u32,
    /// Certified fact-slot ordinal that authorized this emission.
    pub fact_slot_ordinal: u32,
    /// Frozen fact descriptor.
    pub fact_descriptor_ref: ContentRef,
    /// Full retained claim authority.
    pub claim_ref: ValueRef,
    /// Semantic identity of descriptor, subject, and response.
    pub fact_content_identity: FactContentIdentityDigest,
}

impl FactEmission {
    /// Constructs and validates one fact emission.
    pub fn new(
        emission_ordinal: u32,
        fact_slot_ordinal: u32,
        fact_descriptor_ref: &ContentRef,
        claim_ref: &ValueRef,
        fact_content_identity: &FactContentIdentityDigest,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "emission_ordinal",
                CanonicalValue::Unsigned(u64::from(emission_ordinal)),
            ),
            (
                "fact_slot_ordinal",
                CanonicalValue::Unsigned(u64::from(fact_slot_ordinal)),
            ),
            ("fact_descriptor_ref", cv_content_ref(fact_descriptor_ref)?),
            ("claim_ref", claim_ref.canonical_value()?),
            (
                "fact_content_identity",
                cv_string(fact_content_identity.as_str()),
            ),
        ])?)
    }

    /// Projects all typed fact-emission fields.
    pub fn fields(&self) -> Result<FactEmissionFields> {
        Ok(FactEmissionFields {
            emission_ordinal: u32_field(self, "emission_ordinal")?,
            fact_slot_ordinal: u32_field(self, "fact_slot_ordinal")?,
            fact_descriptor_ref: super::codec::content_ref_field(self, "fact_descriptor_ref")?,
            claim_ref: ValueRef::from_canonical_value(required_field(self, "claim_ref")?)?,
            fact_content_identity: FactContentIdentityDigest::parse(string_field(
                self,
                "fact_content_identity",
            )?)?,
        })
    }
}

/// Typed fields of an exact fact reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRefFields {
    /// Producing transition.
    pub transition_ref: TransitionRef,
    /// Emission ordinal within the producing settlement.
    pub emission_ordinal: u32,
}

impl FactRef {
    /// Constructs and validates an exact fact reference.
    pub fn new(transition_ref: &TransitionRef, emission_ordinal: u32) -> Result<Self> {
        Self::from_canonical_value(object([
            ("transition_ref", transition_ref.canonical_value()?),
            (
                "emission_ordinal",
                CanonicalValue::Unsigned(u64::from(emission_ordinal)),
            ),
        ])?)
    }

    /// Projects the exact fact-reference fields.
    pub fn fields(&self) -> Result<FactRefFields> {
        Ok(FactRefFields {
            transition_ref: TransitionRef::from_canonical_value(required_field(
                self,
                "transition_ref",
            )?)?,
            emission_ordinal: u32_field(self, "emission_ordinal")?,
        })
    }
}

/// Closed tenant fact coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantFactCoordinateFields {
    /// This commit has no tenant fact coordinate.
    None,
    /// This transition publishes every embedded emission at one dense order.
    FactPublication {
        /// Admitted tenant.
        tenant_scope_id: TenantScopeId,
        /// Positive dense publication order.
        fact_order: u64,
    },
    /// This authorization freezes a selection barrier without advancing it.
    FactSelectionBarrier {
        /// Admitted tenant.
        tenant_scope_id: TenantScopeId,
        /// Exact current frontier, including zero.
        frontier_fact_order: u64,
    },
}

impl TenantFactCoordinate {
    /// Constructs the coordinate-free variant.
    pub fn none() -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "none",
            std::iter::empty::<(String, CanonicalValue)>(),
        )?)
    }

    /// Constructs a dense publication coordinate.
    pub fn fact_publication(tenant_scope_id: &TenantScopeId, fact_order: u64) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "fact_publication",
            [
                ("tenant_scope_id", cv_string(tenant_scope_id.as_str())),
                ("fact_order", cv_decimal(fact_order)?),
            ],
        )?)
    }

    /// Constructs a non-advancing selection barrier.
    pub fn fact_selection_barrier(
        tenant_scope_id: &TenantScopeId,
        frontier_fact_order: u64,
    ) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "fact_selection_barrier",
            [
                ("tenant_scope_id", cv_string(tenant_scope_id.as_str())),
                ("frontier_fact_order", cv_decimal(frontier_fact_order)?),
            ],
        )?)
    }

    /// Projects the closed coordinate variant and typed fields.
    pub fn fields(&self) -> Result<TenantFactCoordinateFields> {
        match super::codec::tag(self)?.as_str() {
            "none" => Ok(TenantFactCoordinateFields::None),
            "fact_publication" => Ok(TenantFactCoordinateFields::FactPublication {
                tenant_scope_id: tenant_scope_id_field(self, "tenant_scope_id")?,
                fact_order: unsigned_field(self, "fact_order")?,
            }),
            "fact_selection_barrier" => Ok(TenantFactCoordinateFields::FactSelectionBarrier {
                tenant_scope_id: tenant_scope_id_field(self, "tenant_scope_id")?,
                frontier_fact_order: unsigned_field(self, "frontier_fact_order")?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed tenant fact frontier fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantFactFrontierFields {
    /// Store deployment identity.
    pub store_scope_id: StoreScopeId,
    /// Store epoch.
    pub store_epoch: StoreEpoch,
    /// Admitted tenant.
    pub tenant_scope_id: TenantScopeId,
    /// Dense frontier, including zero.
    pub fact_order: u64,
}

impl TenantFactFrontier {
    /// Constructs and validates an exact tenant fact frontier.
    pub fn new(
        store_scope_id: &StoreScopeId,
        store_epoch: StoreEpoch,
        tenant_scope_id: &TenantScopeId,
        fact_order: u64,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("store_scope_id", cv_string(store_scope_id.as_str())),
            ("store_epoch", cv_decimal(store_epoch.get())?),
            ("tenant_scope_id", cv_string(tenant_scope_id.as_str())),
            ("fact_order", cv_decimal(fact_order)?),
        ])?)
    }

    /// Projects all exact frontier fields.
    pub fn fields(&self) -> Result<TenantFactFrontierFields> {
        Ok(TenantFactFrontierFields {
            store_scope_id: store_scope_id_field(self, "store_scope_id")?,
            store_epoch: store_epoch_field(self, "store_epoch")?,
            tenant_scope_id: tenant_scope_id_field(self, "tenant_scope_id")?,
            fact_order: unsigned_field(self, "fact_order")?,
        })
    }
}

/// Typed fact-publication routing fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactPublicationRoutingFields {
    /// Admitted tenant.
    pub tenant_scope_id: TenantScopeId,
    /// Dense publication order.
    pub fact_order: u64,
    /// Fact-producing transition.
    pub transition_ref: TransitionRef,
    /// Complete ordered fact set.
    pub fact_refs: Vec<FactRef>,
}

impl FactPublicationRouting {
    /// Constructs the exact routing copy for one fact-producing transition.
    pub fn new(
        tenant_scope_id: &TenantScopeId,
        fact_order: u64,
        transition_ref: &TransitionRef,
        fact_refs: &[FactRef],
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("tenant_scope_id", cv_string(tenant_scope_id.as_str())),
            ("fact_order", cv_decimal(fact_order)?),
            ("transition_ref", transition_ref.canonical_value()?),
            (
                "fact_refs",
                cv_array(fact_refs.iter().map(|value| value.canonical_value()))?,
            ),
        ])?)
    }

    /// Projects every routing field.
    pub fn fields(&self) -> Result<FactPublicationRoutingFields> {
        Ok(FactPublicationRoutingFields {
            tenant_scope_id: tenant_scope_id_field(self, "tenant_scope_id")?,
            fact_order: unsigned_field(self, "fact_order")?,
            transition_ref: TransitionRef::from_canonical_value(required_field(
                self,
                "transition_ref",
            )?)?,
            fact_refs: array_field(self, "fact_refs")?
                .into_iter()
                .map(FactRef::from_canonical_value)
                .collect::<Result<_>>()?,
        })
    }
}

/// Typed fields of a complete fact-selection response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSelectionResponseFields {
    /// Digest of the exact state-authored request.
    pub request_digest: FactQueryDigest,
    /// Authoritative tenant frontier used by the selection.
    pub frontier: TenantFactFrontier,
    /// Exactly one result for each authored query.
    pub results: Vec<FactSelectionResult>,
}

impl FactSelectionResponse {
    /// Constructs a complete fact-selection response.
    pub fn new(
        request_digest: &FactQueryDigest,
        frontier: &TenantFactFrontier,
        results: &[FactSelectionResult],
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.fact-selection-response.v1".to_owned()),
            ),
            ("request_digest", cv_string(request_digest.as_str())),
            ("frontier", frontier.canonical_value()?),
            (
                "results",
                cv_array(results.iter().map(|value| value.canonical_value()))?,
            ),
        ])?)
    }

    /// Projects every exact fact-selection response field.
    pub fn fields(&self) -> Result<FactSelectionResponseFields> {
        Ok(FactSelectionResponseFields {
            request_digest: FactQueryDigest::parse(string_field(self, "request_digest")?)?,
            frontier: TenantFactFrontier::from_canonical_value(required_field(self, "frontier")?)?,
            results: array_field(self, "results")?
                .into_iter()
                .map(FactSelectionResult::from_canonical_value)
                .collect::<Result<_>>()?,
        })
    }

    /// Validates digest identity and exact ordinal coverage against an authored request.
    pub fn validate_request(&self, request: &FactSelectionRequest) -> Result<()> {
        let fields = self.fields()?;
        let request_digest = request
            .request_digest()
            .map_err(super::JournalError::FactSelectionRequest)?;
        if fields.request_digest != request_digest
            || fields.results.len() != request.queries().len()
        {
            return Err(super::JournalError::FactSelectionCoverage);
        }
        for (expected, result) in fields.results.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| super::JournalError::FactSelectionCoverage)?;
            let result = result.fields()?;
            if result.query_ordinal != expected {
                return Err(super::JournalError::FactSelectionCoverage);
            }
            for selected in result.selected {
                selected.validate_reference_relation()?;
            }
        }
        Ok(())
    }
}

/// Typed fields of one query result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSelectionResultFields {
    /// Ordinal of the corresponding authored query.
    pub query_ordinal: u32,
    /// Fully rehydrated selected facts in query-defined order.
    pub selected: Vec<SelectedFact>,
}

impl FactSelectionResult {
    /// Constructs one explicit query result, including an explicit empty result.
    pub fn new(query_ordinal: u32, selected: &[SelectedFact]) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "query_ordinal",
                CanonicalValue::Unsigned(u64::from(query_ordinal)),
            ),
            (
                "selected",
                cv_array(selected.iter().map(|value| value.canonical_value()))?,
            ),
        ])?)
    }

    /// Projects the query ordinal and complete ordered selection.
    pub fn fields(&self) -> Result<FactSelectionResultFields> {
        Ok(FactSelectionResultFields {
            query_ordinal: u32_field(self, "query_ordinal")?,
            selected: array_field(self, "selected")?
                .into_iter()
                .map(SelectedFact::from_canonical_value)
                .collect::<Result<_>>()?,
        })
    }
}

/// Typed fields of one selected fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedFactFields {
    /// Exact emitted fact coordinate.
    pub fact_ref: FactRef,
    /// Exact producing transition, repeated for direct validation.
    pub producing_transition_ref: TransitionRef,
    /// Frozen descriptor contract.
    pub descriptor_ref: ContentRef,
    /// Full retained fact subject authority.
    pub subject_ref: ValueRef,
    /// Full retained fact response authority.
    pub response_ref: ValueRef,
    /// Semantic identity of descriptor, subject, and response.
    pub content_identity: FactContentIdentityDigest,
}

impl SelectedFact {
    /// Constructs one fully rehydrated selected fact.
    pub fn new(
        fact_ref: &FactRef,
        producing_transition_ref: &TransitionRef,
        descriptor_ref: &ContentRef,
        subject_ref: &ValueRef,
        response_ref: &ValueRef,
        content_identity: &FactContentIdentityDigest,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("fact_ref", fact_ref.canonical_value()?),
            (
                "producing_transition_ref",
                producing_transition_ref.canonical_value()?,
            ),
            (
                "descriptor_ref",
                super::codec::cv_content_ref(descriptor_ref)?,
            ),
            ("subject_ref", subject_ref.canonical_value()?),
            ("response_ref", response_ref.canonical_value()?),
            ("content_identity", cv_string(content_identity.as_str())),
        ])?)
    }

    /// Projects every exact selected-fact field.
    pub fn fields(&self) -> Result<SelectedFactFields> {
        Ok(SelectedFactFields {
            fact_ref: FactRef::from_canonical_value(required_field(self, "fact_ref")?)?,
            producing_transition_ref: TransitionRef::from_canonical_value(required_field(
                self,
                "producing_transition_ref",
            )?)?,
            descriptor_ref: super::codec::content_ref_field(self, "descriptor_ref")?,
            subject_ref: ValueRef::from_canonical_value(required_field(self, "subject_ref")?)?,
            response_ref: ValueRef::from_canonical_value(required_field(self, "response_ref")?)?,
            content_identity: FactContentIdentityDigest::parse(string_field(
                self,
                "content_identity",
            )?)?,
        })
    }

    /// Validates the repeated producer reference against the exact fact coordinate.
    pub fn validate_reference_relation(&self) -> Result<()> {
        let fields = self.fields()?;
        if fields.fact_ref.fields()?.transition_ref == fields.producing_transition_ref {
            Ok(())
        } else {
            Err(super::JournalError::ReferenceMismatch)
        }
    }
}

/// Closed reason that a fact-selection result carries no completeness upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactSelectionUnverifiedReason {
    /// The response was restored from a portable bundle.
    PortableBundle,
    /// The authoritative store prefix could not be verified.
    PrefixVerificationUnavailable,
}

impl FactSelectionUnverifiedReason {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PortableBundle => "portable_bundle",
            Self::PrefixVerificationUnavailable => "prefix_verification_unavailable",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "portable_bundle" => Ok(Self::PortableBundle),
            "prefix_verification_unavailable" => Ok(Self::PrefixVerificationUnavailable),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Closed completeness claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactSelectionCompletenessFields {
    /// Completeness was established against the authoritative same-store writer.
    SameStoreVerified {
        /// Authorization that fixed the barrier.
        authorization_ref: AuthorizationRef,
        /// Exact verified frontier.
        frontier: Box<TenantFactFrontier>,
    },
    /// Portable or unavailable-prefix verification carries no authority upgrade.
    Unverified {
        /// Frozen safe reason code.
        reason: FactSelectionUnverifiedReason,
    },
}

impl FactSelectionCompleteness {
    /// Constructs a completeness claim verified against the authoritative same-store frontier.
    pub fn same_store_verified(
        authorization_ref: &AuthorizationRef,
        frontier: &TenantFactFrontier,
    ) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "same_store_verified",
            [
                ("authorization_ref", authorization_ref.canonical_value()?),
                ("frontier", frontier.canonical_value()?),
            ],
        )?)
    }

    /// Constructs an explicitly unverified completeness claim.
    pub fn unverified(reason: FactSelectionUnverifiedReason) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "unverified",
            [("reason", cv_string(reason.as_str()))],
        )?)
    }

    /// Projects the closed completeness variant.
    pub fn fields(&self) -> Result<FactSelectionCompletenessFields> {
        match super::codec::tag(self)?.as_str() {
            "same_store_verified" => Ok(FactSelectionCompletenessFields::SameStoreVerified {
                authorization_ref: AuthorizationRef::from_canonical_value(required_field(
                    self,
                    "authorization_ref",
                )?)?,
                frontier: Box::new(TenantFactFrontier::from_canonical_value(required_field(
                    self, "frontier",
                )?)?),
            }),
            "unverified" => Ok(FactSelectionCompletenessFields::Unverified {
                reason: FactSelectionUnverifiedReason::parse(&super::codec::string_field(
                    self, "reason",
                )?)?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of the semantic fact-content identity preimage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactContentIdentityPreimageFields {
    /// Frozen descriptor contract.
    pub fact_descriptor_ref: ContentRef,
    /// Full retained subject authority.
    pub subject_ref: ValueRef,
    /// Full retained response authority.
    pub response_ref: ValueRef,
}

impl FactContentIdentityPreimage {
    /// Constructs the producer-coordinate-free fact-content preimage.
    pub fn new(
        fact_descriptor_ref: &ContentRef,
        subject_ref: &ValueRef,
        response_ref: &ValueRef,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "fact_descriptor_ref",
                super::codec::cv_content_ref(fact_descriptor_ref)?,
            ),
            ("subject_ref", subject_ref.canonical_value()?),
            ("response_ref", response_ref.canonical_value()?),
        ])?)
    }

    /// Projects the exact fact-content preimage.
    pub fn fields(&self) -> Result<FactContentIdentityPreimageFields> {
        Ok(FactContentIdentityPreimageFields {
            fact_descriptor_ref: super::codec::content_ref_field(self, "fact_descriptor_ref")?,
            subject_ref: ValueRef::from_canonical_value(required_field(self, "subject_ref")?)?,
            response_ref: ValueRef::from_canonical_value(required_field(self, "response_ref")?)?,
        })
    }

    /// Derives the frozen semantic identity of fact content.
    pub fn fact_content_identity(&self) -> Result<FactContentIdentityDigest> {
        domain_digest("mfm.fact-content-identity.v1", self)
            .map(FactContentIdentityDigest::from_semantic_digest)
    }
}

/// Typed fields of the semantic fact-logical identity preimage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactLogicalIdentityPreimageFields {
    /// Exact producing transition.
    pub transition_ref: TransitionRef,
    /// Ordinal within the producing settlement.
    pub emission_ordinal: u32,
    /// Fact-content semantic identity.
    pub fact_content_identity: FactContentIdentityDigest,
}

impl FactLogicalIdentityPreimage {
    /// Constructs the tenant-order-free fact-logical preimage.
    pub fn new(
        transition_ref: &TransitionRef,
        emission_ordinal: u32,
        fact_content_identity: &FactContentIdentityDigest,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("transition_ref", transition_ref.canonical_value()?),
            (
                "emission_ordinal",
                CanonicalValue::Unsigned(u64::from(emission_ordinal)),
            ),
            (
                "fact_content_identity",
                cv_string(fact_content_identity.as_str()),
            ),
        ])?)
    }

    /// Projects the exact fact-logical preimage.
    pub fn fields(&self) -> Result<FactLogicalIdentityPreimageFields> {
        Ok(FactLogicalIdentityPreimageFields {
            transition_ref: TransitionRef::from_canonical_value(required_field(
                self,
                "transition_ref",
            )?)?,
            emission_ordinal: u32_field(self, "emission_ordinal")?,
            fact_content_identity: FactContentIdentityDigest::parse(string_field(
                self,
                "fact_content_identity",
            )?)?,
        })
    }

    /// Derives the frozen logical identity of one emitted fact.
    pub fn fact_logical_identity(&self) -> Result<FactLogicalIdentityDigest> {
        domain_digest("mfm.fact-logical-identity.v1", self)
            .map(FactLogicalIdentityDigest::from_semantic_digest)
    }
}
