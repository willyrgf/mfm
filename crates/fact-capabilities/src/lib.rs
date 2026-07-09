#![warn(missing_docs)]
//! Generic internal fact capability contracts.
//!
//! This crate defines the state/adapter-facing authority contracts for:
//! - internal Control and Platform reads from the MFM fact index
//! - managed Platform fact recording (shared write role for collectors)
//!
//! Concrete store implementations, SQL query execution, app wiring, and public
//! fact DTO services live outside this crate. Platform index reads still require
//! certified evidence and retained response artifacts; they are not a
//! public-facts authority path.
//!
//! ```rust
//! use mfm_capabilities::CapabilitySpec;
//! use mfm_fact_capabilities::{FactIndexReadCapability, FactRecordCapability};
//!
//! assert_eq!(FactIndexReadCapability::name(), "mfm.fact.index.read");
//! assert_eq!(FactRecordCapability::name(), "mfm.fact.record");
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ManagedPlatformWriteRole, ReadExternalRole,
};
use mfm_facts::{
    CanonicalFactQueryPlan, FactAudience, FactQueryEvidence, FactQueryReceipt, FactQueryResultRow,
    FactSelectionEvidence,
};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm};

pub use mfm_facts::FactQueryReceiptTrustRootMaterial;

/// Result type for fact-index read capability contracts.
pub type Result<T> = std::result::Result<T, FactIndexReadError>;

/// Boxed future returned by fact-index read providers.
pub type FactIndexReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<FactIndexReadResponse>> + Send + 'a>>;

/// Boxed future returned by batch fact-index read providers.
pub type FactIndexReadBatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<FactIndexReadResponse>>> + Send + 'a>>;

/// Internal fact-index read authority.
pub struct FactIndexReadCapability;

impl CapabilitySpec for FactIndexReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.fact",
            "index.read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.fact.capability:index.read"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.fact.index.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.fact.index.read"
    }
}

/// Shared managed Platform-write capability for recording fact claims.
///
/// Used by BTC and EVM collector record states. Family monomorphism lives in the
/// fact types and observe transports, not in a second capability identity.
pub struct FactRecordCapability;

impl CapabilitySpec for FactRecordCapability {
    type Role = ManagedPlatformWriteRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.fact",
            "record",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.fact.capability:record"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.fact.record.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.fact.record"
    }
}

/// Provider interface for internal fact-index reads.
pub trait FactIndexReadProvider: Send + Sync {
    /// Returns the stable identity of the concrete process-level fact-index implementation.
    fn implementation_id(&self) -> &'static str;

    /// Executes an already-compiled canonical fact query plan.
    ///
    /// Default path runs a one-element [`Self::read_fact_index_batch`] so single and multi
    /// reads share one provider implementation.
    fn read_fact_index<'a>(&'a self, request: &'a FactIndexReadRequest) -> FactIndexReadFuture<'a> {
        Box::pin(async move {
            let mut responses = self
                .read_fact_index_batch(std::slice::from_ref(request))
                .await?;
            responses.pop().ok_or_else(|| {
                FactIndexReadError::redacted_provider_failure(
                    "fact-index batch returned no response for single request",
                )
            })
        })
    }

    /// Executes multiple plans under **one shared store read snapshot**.
    ///
    /// Multi-holding portfolio selection must use this entry point so candidate sets share
    /// one selection frontier. Implementations must not open independent snapshots per
    /// request when `requests.len() > 1`. Empty input returns an empty vec.
    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> FactIndexReadBatchFuture<'a>;
}

/// Request to read internal Control or Platform facts from the fact index.
///
/// Certified states may query both audiences through this single capability.
/// Platform reads still require certified evidence and retained response
/// artifacts; they are not a public-facts authority path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexReadRequest {
    plan: CanonicalFactQueryPlan,
}

impl FactIndexReadRequest {
    /// Creates a fact-index read request from an already-compiled plan.
    ///
    /// Accepts plans scoped to [`FactAudience::Control`] or
    /// [`FactAudience::Platform`]. Other audiences fail closed.
    pub fn new(plan: CanonicalFactQueryPlan) -> Result<Self> {
        if !is_supported_fact_index_audience(plan.query_scope().audience()) {
            return Err(FactIndexReadError::InvalidRequest {
                reason: FactIndexInvalidRequest::UnsupportedAudience,
            });
        }
        Ok(Self { plan })
    }

    /// Returns the compiled canonical query plan.
    pub const fn plan(&self) -> &CanonicalFactQueryPlan {
        &self.plan
    }
}

/// One row returned by an internal fact-index read.
pub type FactIndexReadRow = FactQueryResultRow;

/// Response from an internal fact-index read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexReadResponse {
    rows: Vec<FactIndexReadRow>,
    receipt: FactQueryReceipt,
    trust_root: FactQueryReceiptTrustRootMaterial,
}

impl FactIndexReadResponse {
    /// Creates a response and validates rows against the authenticated receipt shape.
    pub fn new(
        rows: Vec<FactIndexReadRow>,
        receipt: FactQueryReceipt,
        trust_root: FactQueryReceiptTrustRootMaterial,
    ) -> Result<Self> {
        mfm_facts::validate_fact_query_result_rows(&rows, &receipt)
            .map_err(|reason| FactIndexReadError::Receipt { reason })?;
        Ok(Self {
            rows,
            receipt,
            trust_root,
        })
    }

    /// Creates a response whose row list is derived directly from the receipt.
    pub fn from_receipt(
        receipt: FactQueryReceipt,
        trust_root: FactQueryReceiptTrustRootMaterial,
    ) -> Self {
        let rows = mfm_facts::fact_query_result_rows_from_receipt(&receipt);
        Self {
            rows,
            receipt,
            trust_root,
        }
    }

    /// Returns returned rows in receipt order.
    pub fn rows(&self) -> &[FactIndexReadRow] {
        &self.rows
    }

    /// Returns the store-authenticated query receipt.
    pub const fn receipt(&self) -> &FactQueryReceipt {
        &self.receipt
    }

    /// Returns the trust root material needed to verify the receipt later.
    pub const fn trust_root(&self) -> &FactQueryReceiptTrustRootMaterial {
        &self.trust_root
    }

    /// Builds replay evidence for pinning this read into a consuming run.
    pub fn into_evidence(
        self,
        request: &FactIndexReadRequest,
        selection: FactSelectionEvidence,
    ) -> Result<FactIndexReadEvidence> {
        FactIndexReadEvidence::new(
            FactQueryEvidence::new(request.plan().clone(), self.receipt, selection),
            self.trust_root,
        )
    }
}

/// Replay evidence and trust-root material produced from a fact-index read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexReadEvidence {
    query_evidence: FactQueryEvidence,
    trust_root: FactQueryReceiptTrustRootMaterial,
}

impl FactIndexReadEvidence {
    /// Creates read evidence and validates the replay evidence shape.
    pub fn new(
        query_evidence: FactQueryEvidence,
        trust_root: FactQueryReceiptTrustRootMaterial,
    ) -> Result<Self> {
        mfm_facts::validate_fact_query_evidence(&query_evidence).map_err(|_| {
            FactIndexReadError::Evidence {
                reason: FactIndexEvidenceFailure::InvalidShape,
            }
        })?;
        Ok(Self {
            query_evidence,
            trust_root,
        })
    }

    /// Returns pinned query replay evidence.
    pub const fn query_evidence(&self) -> &FactQueryEvidence {
        &self.query_evidence
    }

    /// Returns public trust-root material for receipt verification.
    pub const fn trust_root(&self) -> &FactQueryReceiptTrustRootMaterial {
        &self.trust_root
    }
}

/// Closed invalid-request reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactIndexInvalidRequest {
    /// The compiled plan audience is outside the certified fact-index allowlist.
    UnsupportedAudience,
}

/// Returns whether an audience may be used with certified fact-index reads.
///
/// Allowlist: Control (operational cursors) and Platform (reportable holdings).
const fn is_supported_fact_index_audience(audience: FactAudience) -> bool {
    matches!(audience, FactAudience::Control | FactAudience::Platform)
}

/// Closed provider failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactIndexProviderFailure {
    /// Provider failed without exposing SQL, URLs, store internals, or raw backend diagnostics.
    Failed,
}

/// Closed receipt mismatch reasons.
pub type FactIndexReceiptFailure = mfm_facts::FactQueryResultMismatch;

/// Closed evidence validation reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactIndexEvidenceFailure {
    /// Query evidence failed facts-kernel shape validation.
    InvalidShape,
}

/// Redaction-safe fact-index read capability error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FactIndexReadError {
    /// Request failed contract validation.
    #[error("fact-index read request was invalid")]
    InvalidRequest {
        /// Closed invalid-request reason.
        reason: FactIndexInvalidRequest,
    },
    /// Provider failed without exposing backend details.
    #[error("fact-index read provider failed")]
    Provider {
        /// Closed provider failure reason.
        reason: FactIndexProviderFailure,
    },
    /// Provider response did not match its authenticated receipt.
    #[error("fact-index read receipt was invalid")]
    Receipt {
        /// Closed receipt failure reason.
        reason: FactIndexReceiptFailure,
    },
    /// Replay evidence failed contract validation.
    #[error("fact-index read evidence was invalid")]
    Evidence {
        /// Closed evidence failure reason.
        reason: FactIndexEvidenceFailure,
    },
}

impl FactIndexReadError {
    /// Builds a redacted provider failure, discarding raw backend diagnostics.
    pub fn redacted_provider_failure(_source: impl fmt::Display) -> Self {
        Self::Provider {
            reason: FactIndexProviderFailure::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use mfm_capabilities::{CapabilityRole, CapabilitySpec};
    use mfm_facts::{
        DescriptorCatalogWatermark, FactCanonicalScalar, FactClaimId, FactFieldId, FactFieldValue,
        FactFieldValueType, FactOrderingName, FactOrderingPolicy, FactOrderingTerm,
        FactProducerProvenance, FactProjectionGeneration, FactQueryScope, FactResponseEvidence,
        FactSubjectRef, FactVisibility, FactVisibilityScope, InternalFactRef, InternalFactRefParts,
        NullOrdering, ReturnedFactFieldSummary, ReturnedFieldSummaries, ScopeDecisionEvidence,
        SortDirection, StoreCommitWatermark, StoreIdentity, StoreKeyId, StoreReadFrontier,
        StoreReadFrontierType, StoreReceiptAuthentication, StoreReceiptAuthenticationScheme,
        StoreScopeRef,
    };
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
        DigestAlgorithm, DigestBytes, EventId, RunId, SchemaId,
    };

    use super::*;

    #[test]
    fn capability_identity_is_read_external() {
        let descriptor = FactIndexReadCapability::descriptor().expect("descriptor");

        assert_eq!(FactIndexReadCapability::name(), "mfm.fact.index.read");
        assert_eq!(descriptor.role, CapabilityRole::ReadExternal);
        assert_eq!(descriptor.version.as_str(), "mfm.fact.index.read.v1");
    }

    #[test]
    fn request_accepts_control_and_platform_audiences() {
        for audience in [FactAudience::Control, FactAudience::Platform] {
            let request =
                FactIndexReadRequest::new(plan(audience)).expect("supported audience request");
            assert_eq!(request.plan().query_scope().audience(), audience);
            assert!(is_supported_fact_index_audience(audience));
        }
    }

    #[test]
    fn request_allowlist_covers_control_and_platform() {
        // Exhaustive over current FactAudience variants: both are allowed.
        // When a new audience is added, is_supported_fact_index_audience must
        // stay fail-closed (return false) until deliberately allowlisted.
        assert!(is_supported_fact_index_audience(FactAudience::Control));
        assert!(is_supported_fact_index_audience(FactAudience::Platform));
    }

    #[test]
    fn response_rows_receipt_and_evidence_shape_are_pinned_for_control_and_platform() {
        for audience in [FactAudience::Control, FactAudience::Platform] {
            let request = FactIndexReadRequest::new(plan(audience)).expect("request");
            let receipt = receipt_with_summary(request.plan(), internal_fact_ref(1));
            let response = FactIndexReadResponse::from_receipt(receipt.clone(), trust_root());

            assert_eq!(response.rows().len(), 1);
            assert_eq!(response.rows()[0].fact_ref(), &receipt.returned_refs()[0]);
            assert_eq!(response.rows()[0].returned_fields().len(), 1);
            assert_eq!(response.receipt(), &receipt);
            assert_eq!(
                response.trust_root().scheme(),
                StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1
            );

            let selection =
                FactSelectionEvidence::new(digest(0x71), vec![0], None).expect("selection");
            let evidence = response
                .into_evidence(&request, selection)
                .expect("evidence");

            assert_eq!(evidence.query_evidence().plan(), request.plan());
            assert_eq!(evidence.query_evidence().receipt(), &receipt);
            assert_eq!(evidence.trust_root().key_id().as_str(), "fact.read.key");
            assert_eq!(
                evidence.query_evidence().plan().query_scope().audience(),
                audience
            );
        }
    }

    #[test]
    fn response_rejects_rows_that_do_not_match_receipt_shape() {
        enum Case {
            UnpinnedFieldSummaries,
            RowRefMismatch,
        }

        for (case, expected) in [
            (
                Case::UnpinnedFieldSummaries,
                FactIndexReceiptFailure::UnpinnedFieldSummaries,
            ),
            (
                Case::RowRefMismatch,
                FactIndexReceiptFailure::RowRefMismatch,
            ),
        ] {
            let (row, receipt) = match case {
                Case::UnpinnedFieldSummaries => {
                    let fact_ref = internal_fact_ref(1);
                    (
                        FactIndexReadRow::new(fact_ref.clone(), vec![summary_value(42)]),
                        receipt_without_summaries(&plan(FactAudience::Control), fact_ref),
                    )
                }
                Case::RowRefMismatch => (
                    FactIndexReadRow::new(internal_fact_ref(2), Vec::new()),
                    receipt_without_summaries(&plan(FactAudience::Control), internal_fact_ref(1)),
                ),
            };

            let error =
                FactIndexReadResponse::new(vec![row], receipt, trust_root()).expect_err("shape");

            assert_eq!(error, FactIndexReadError::Receipt { reason: expected });
        }
    }

    #[test]
    fn provider_failure_redacts_backend_diagnostics() {
        let error = FactIndexReadError::redacted_provider_failure(concat!(
            "postgres://user:password@db.internal/mfm select * from fact_index ",
            "http://store.internal/?token=secret"
        ));
        let rendered = format!("{error:?} {error}");

        assert_eq!(
            error,
            FactIndexReadError::Provider {
                reason: FactIndexProviderFailure::Failed,
            }
        );
        assert!(!rendered.contains("postgres://"));
        assert!(!rendered.contains("select *"));
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains("store.internal"));
        assert!(!rendered.contains("secret"));
    }

    fn plan(audience: FactAudience) -> CanonicalFactQueryPlan {
        let descriptor = mfm_facts::FactDescriptor::new(
            mfm_facts::FactKind::new("collector.checkpoint").expect("kind"),
            mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
            schema_id(0x30),
            schema_id(0x31),
            vec![
                mfm_facts::FactFieldDescriptor::new(
                    FactFieldId::new("subject.chain").expect("field"),
                    mfm_facts::FactFieldValueType::String,
                    mfm_facts::FactFieldExtraction::Subject(
                        mfm_facts::CanonicalValuePath::new("chain").expect("path"),
                    ),
                    mfm_facts::FactFieldPolicy::new(
                        vec![mfm_facts::FactQueryOperator::Equal],
                        mfm_facts::FactFieldExposure::Returnable,
                    )
                    .required(),
                )
                .expect("subject field"),
                mfm_facts::FactFieldDescriptor::new(
                    FactFieldId::new("metadata.recorded_at").expect("field"),
                    mfm_facts::FactFieldValueType::Timestamp,
                    mfm_facts::FactFieldExtraction::Metadata(
                        mfm_facts::FactMetadataField::RecordedAt,
                    ),
                    mfm_facts::FactFieldPolicy::new(
                        vec![mfm_facts::FactQueryOperator::Equal],
                        mfm_facts::FactFieldExposure::QueryOnly,
                    )
                    .sortable()
                    .required(),
                )
                .expect("metadata field"),
                mfm_facts::FactFieldDescriptor::new(
                    FactFieldId::new("result.height").expect("field"),
                    mfm_facts::FactFieldValueType::UnsignedInteger,
                    mfm_facts::FactFieldExtraction::Response(
                        mfm_facts::CanonicalValuePath::new("height").expect("path"),
                    ),
                    mfm_facts::FactFieldPolicy::new(
                        vec![mfm_facts::FactQueryOperator::Equal],
                        mfm_facts::FactFieldExposure::Returnable,
                    )
                    .required(),
                )
                .expect("result field"),
            ],
            vec![FactOrderingPolicy::new(
                FactOrderingName::new("metadata.recorded_at.desc").expect("ordering"),
                vec![FactOrderingTerm::new(
                    FactFieldId::new("metadata.recorded_at").expect("field"),
                    SortDirection::Descending,
                    NullOrdering::Last,
                    true,
                )],
            )
            .expect("fact ordering")],
        )
        .expect("descriptor");
        let input = mfm_facts::FactQueryInput::new(
            StoreScopeRef::new("mfm.store.default").expect("store scope"),
            FactQueryScope::new(audience, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(digest(0x21)),
            vec![mfm_facts::FactQueryPredicate::new(
                FactFieldId::new("subject.chain").expect("field"),
                mfm_facts::FactQueryOperator::Equal,
                FactCanonicalScalar::string("bitcoin"),
            )],
            vec![FactFieldId::new("result.height").expect("field")],
            FactOrderingName::new("metadata.recorded_at.desc").expect("ordering"),
            Some(1),
        )
        .expect("query input");
        mfm_facts::compile_fact_query_plan(&descriptor, input).expect("plan")
    }

    fn receipt_with_summary(
        plan: &CanonicalFactQueryPlan,
        fact_ref: InternalFactRef,
    ) -> FactQueryReceipt {
        let returned_field_summaries =
            ReturnedFieldSummaries::new(vec![ReturnedFactFieldSummary::new(
                fact_ref.fact_claim_id().clone(),
                vec![summary_value(42)],
            )]);
        receipt(plan, fact_ref, Some(returned_field_summaries))
    }

    fn receipt_without_summaries(
        plan: &CanonicalFactQueryPlan,
        fact_ref: InternalFactRef,
    ) -> FactQueryReceipt {
        receipt(plan, fact_ref, None)
    }

    fn receipt(
        plan: &CanonicalFactQueryPlan,
        fact_ref: InternalFactRef,
        returned_field_summaries: Option<ReturnedFieldSummaries>,
    ) -> FactQueryReceipt {
        let returned_fields = returned_field_summaries
            .as_ref()
            .and_then(|summaries| summaries.summaries().first())
            .map(|summary| summary.fields().to_vec())
            .unwrap_or_default();
        let rows = [FactQueryResultRow::new(fact_ref, returned_fields)];
        let read_frontier = StoreReadFrontier::new(
            StoreScopeRef::new("mfm.store.default").expect("store scope"),
            plan.query_scope().clone(),
            DescriptorCatalogWatermark::new(1),
            FactProjectionGeneration::new(1),
            11,
            StoreCommitWatermark::new(11),
        );
        let plan_hash = mfm_facts::fact_query_plan_hash(plan).expect("plan hash");
        let material = mfm_facts::FactQueryReceiptMaterial::from_rows(
            &plan_hash,
            read_frontier,
            StoreReadFrontierType::Snapshot,
            &rows,
            returned_field_summaries.is_some(),
            None,
        )
        .expect("receipt material");
        material.into_receipt(
            StoreReceiptAuthentication::new(
                StoreIdentity::new("store.default").expect("store identity"),
                StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
                Some(StoreKeyId::new("fact.read.key").expect("key id")),
                vec![0x11; 64],
            )
            .expect("auth"),
        )
    }

    fn summary_value(value: u64) -> FactFieldValue {
        FactFieldValue::new(
            FactFieldId::new("result.height").expect("field"),
            FactFieldValueType::UnsignedInteger,
            FactCanonicalScalar::UnsignedInteger(value),
        )
        .expect("summary")
    }

    fn trust_root() -> FactQueryReceiptTrustRootMaterial {
        FactQueryReceiptTrustRootMaterial::new(
            StoreIdentity::new("store.default").expect("store identity"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            StoreKeyId::new("fact.read.key").expect("key id"),
            [7_u8; 32],
        )
    }

    fn internal_fact_ref(seed: u8) -> InternalFactRef {
        InternalFactRef::new(InternalFactRefParts {
            fact_claim_id: FactClaimId::new(run_id(seed), 9, 0).expect("claim id"),
            source_event_id: EventId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 1),
            ),
            recorded_at: "2026-07-02T00:00:00Z".to_owned(),
            producer_node_id: mfm_ids::NodeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 2),
            ),
            observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
            visibility: FactVisibility::indexed_default(FactAudience::Control),
            fact_kind: mfm_facts::FactKind::new("collector.checkpoint").expect("kind"),
            fact_descriptor_hash: digest(seed + 2),
            subject: FactSubjectRef::new(
                digest(seed + 3),
                mfm_facts::FactKey::from_digest(digest(seed + 4)),
                digest(seed + 5),
            ),
            request: None,
            response: FactResponseEvidence::new(
                schema_id(seed + 6),
                digest(seed + 7),
                ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 8)),
                digest(seed + 9),
            ),
            producer: FactProducerProvenance::new(
                CapabilityKind::new(
                    "mfm.fact",
                    "index.read",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_bytes(seed + 10),
                )
                .expect("capability kind"),
                CapabilityVersion::new("mfm.fact.index.read.v1").expect("capability version"),
                AdapterKind::new(
                    "mfm.fact",
                    "index.adapter",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_bytes(seed + 11),
                )
                .expect("adapter kind"),
                AdapterVersion::new("mfm.fact.index.adapter.v1").expect("adapter version"),
            ),
        })
        .expect("fact ref")
    }

    fn schema_id(seed: u8) -> SchemaId {
        SchemaId::new(
            "mfm.fact.test",
            "v1",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed),
        )
        .expect("schema id")
    }

    fn run_id(seed: u8) -> RunId {
        RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
    }

    fn digest(seed: u8) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
    }

    fn digest_bytes(seed: u8) -> DigestBytes {
        DigestBytes::from_array([seed; 32])
    }
}
