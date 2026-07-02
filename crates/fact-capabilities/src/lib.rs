#![warn(missing_docs)]
//! Generic internal fact-index read capability contracts.
//!
//! This crate defines the state/adapter-facing authority contract for internal
//! reads from the MFM fact index. Concrete store implementations, SQL query
//! execution, app wiring, and public fact DTO services live outside this crate.
//!
//! ```rust
//! use mfm_capabilities::CapabilitySpec;
//! use mfm_fact_capabilities::FactIndexReadCapability;
//!
//! assert_eq!(FactIndexReadCapability::name(), "mfm.fact.index.read");
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{CapabilityError, CapabilitySpec, ReadExternalRole};
use mfm_facts::{
    CanonicalFactQueryPlan, FactAudience, FactQueryEvidence, FactQueryReceipt,
    FactSelectionEvidence, InternalFactRef, ReturnedFieldValueSummary,
    StoreReceiptAuthenticationScheme,
};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm};

/// Result type for fact-index read capability contracts.
pub type Result<T> = std::result::Result<T, FactIndexReadError>;

/// Boxed future returned by fact-index read providers.
pub type FactIndexReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<FactIndexReadResponse>> + Send + 'a>>;

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

/// Provider interface for internal fact-index reads.
pub trait FactIndexReadProvider: Send + Sync {
    /// Executes an already-compiled canonical fact query plan.
    fn read_fact_index<'a>(&'a self, request: &'a FactIndexReadRequest) -> FactIndexReadFuture<'a>;
}

/// Request to read internal Control facts from the fact index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexReadRequest {
    plan: CanonicalFactQueryPlan,
}

impl FactIndexReadRequest {
    /// Creates a fact-index read request from an already-compiled plan.
    pub fn new(plan: CanonicalFactQueryPlan) -> Result<Self> {
        if plan.query_scope().audience() != FactAudience::Control {
            return Err(FactIndexReadError::InvalidRequest {
                reason: FactIndexInvalidRequest::NonControlAudience,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexReadRow {
    fact_ref: InternalFactRef,
    returned_fields: Vec<ReturnedFieldValueSummary>,
}

impl FactIndexReadRow {
    /// Creates a returned row from its trusted internal ref and pinned summaries.
    pub fn new(fact_ref: InternalFactRef, returned_fields: Vec<ReturnedFieldValueSummary>) -> Self {
        Self {
            fact_ref,
            returned_fields,
        }
    }

    /// Returns the trusted internal fact ref.
    pub const fn fact_ref(&self) -> &InternalFactRef {
        &self.fact_ref
    }

    /// Returns field summaries in receipt order for this row.
    pub fn returned_fields(&self) -> &[ReturnedFieldValueSummary] {
        &self.returned_fields
    }
}

/// Store-neutral public trust-root material for fact query receipt authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryReceiptTrustRootMaterial {
    store_identity: mfm_facts::StoreIdentity,
    scheme: StoreReceiptAuthenticationScheme,
    key_id: mfm_facts::StoreKeyId,
    verifying_key: [u8; 32],
}

impl FactQueryReceiptTrustRootMaterial {
    /// Creates trust-root material from store-owned public verification data.
    pub fn new(
        store_identity: mfm_facts::StoreIdentity,
        scheme: StoreReceiptAuthenticationScheme,
        key_id: mfm_facts::StoreKeyId,
        verifying_key: [u8; 32],
    ) -> Self {
        Self {
            store_identity,
            scheme,
            key_id,
            verifying_key,
        }
    }

    /// Returns the store identity bound to this trust root.
    pub const fn store_identity(&self) -> &mfm_facts::StoreIdentity {
        &self.store_identity
    }

    /// Returns the authentication scheme bound to this trust root.
    pub const fn scheme(&self) -> StoreReceiptAuthenticationScheme {
        self.scheme
    }

    /// Returns the non-secret key id bound to this trust root.
    pub const fn key_id(&self) -> &mfm_facts::StoreKeyId {
        &self.key_id
    }

    /// Returns the raw public verifying key bytes.
    pub const fn verifying_key(&self) -> &[u8; 32] {
        &self.verifying_key
    }
}

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
        validate_rows_match_receipt(&rows, &receipt)?;
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
        let rows = rows_from_receipt(&receipt);
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
    /// The compiled plan was not scoped to the internal Control audience.
    NonControlAudience,
}

/// Closed provider failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactIndexProviderFailure {
    /// Provider failed without exposing SQL, URLs, store internals, or raw backend diagnostics.
    Failed,
}

/// Closed receipt mismatch reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactIndexReceiptFailure {
    /// Row count differed from the returned refs in the receipt.
    RowCountMismatch,
    /// A row ref differed from the corresponding receipt ref.
    RowRefMismatch,
    /// Returned field summaries were supplied without receipt-pinned summaries.
    UnpinnedFieldSummaries,
    /// Receipt summary count differed from the returned refs in the receipt.
    SummaryCountMismatch,
    /// A receipt summary claim id differed from the corresponding row ref.
    SummaryClaimMismatch,
    /// A row's returned fields differed from receipt-pinned summaries.
    SummaryValueMismatch,
}

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

fn rows_from_receipt(receipt: &FactQueryReceipt) -> Vec<FactIndexReadRow> {
    receipt
        .returned_refs()
        .iter()
        .enumerate()
        .map(|(index, fact_ref)| {
            let returned_fields = receipt
                .returned_field_summaries()
                .and_then(|summaries| summaries.summaries().get(index))
                .map(|summary| summary.fields().to_vec())
                .unwrap_or_default();
            FactIndexReadRow::new(fact_ref.clone(), returned_fields)
        })
        .collect()
}

fn validate_rows_match_receipt(
    rows: &[FactIndexReadRow],
    receipt: &FactQueryReceipt,
) -> Result<()> {
    if rows.len() != receipt.returned_refs().len() {
        return Err(receipt_error(FactIndexReceiptFailure::RowCountMismatch));
    }

    for (row, returned_ref) in rows.iter().zip(receipt.returned_refs()) {
        if row.fact_ref() != returned_ref {
            return Err(receipt_error(FactIndexReceiptFailure::RowRefMismatch));
        }
    }

    let Some(summaries) = receipt.returned_field_summaries() else {
        if rows.iter().any(|row| !row.returned_fields().is_empty()) {
            return Err(receipt_error(
                FactIndexReceiptFailure::UnpinnedFieldSummaries,
            ));
        }
        return Ok(());
    };

    if summaries.summaries().len() != rows.len() {
        return Err(receipt_error(FactIndexReceiptFailure::SummaryCountMismatch));
    }

    for (row, summary) in rows.iter().zip(summaries.summaries()) {
        if summary.fact_claim_id() != row.fact_ref().fact_claim_id() {
            return Err(receipt_error(FactIndexReceiptFailure::SummaryClaimMismatch));
        }
        if summary.fields() != row.returned_fields() {
            return Err(receipt_error(FactIndexReceiptFailure::SummaryValueMismatch));
        }
    }

    Ok(())
}

fn receipt_error(reason: FactIndexReceiptFailure) -> FactIndexReadError {
    FactIndexReadError::Receipt { reason }
}

#[cfg(test)]
mod tests {
    use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
    use mfm_capabilities::{CapabilityRole, CapabilitySpec};
    use mfm_facts::{
        DescriptorCatalogWatermark, FactCanonicalScalar, FactCanonicalizerVersion, FactClaimId,
        FactFieldId, FactFieldValueType, FactOrdering, FactOrderingName, FactOrderingTerm,
        FactProjectionGeneration, FactQueryCompilerVersion, FactQueryScope, FactVisibility,
        FactVisibilityScope, InternalFactRefParts, NullOrdering, QueryResultCardinality,
        ReturnedFactFieldSummary, ReturnedFieldSummaries, ScopeDecisionEvidence, SortDirection,
        StoreCommitWatermark, StoreIdentity, StoreKeyId, StoreReadFrontier, StoreReadFrontierType,
        StoreReceiptAuthentication, StoreScopeRef,
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
    fn request_rejects_public_platform_scope() {
        let error = FactIndexReadRequest::new(plan(FactAudience::Platform))
            .expect_err("platform scope must be rejected");

        assert_eq!(
            error,
            FactIndexReadError::InvalidRequest {
                reason: FactIndexInvalidRequest::NonControlAudience,
            }
        );
    }

    #[test]
    fn response_rows_receipt_and_evidence_shape_are_pinned() {
        let request = FactIndexReadRequest::new(plan(FactAudience::Control)).expect("request");
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

        let selection = FactSelectionEvidence::new(digest(0x71), vec![0], None).expect("selection");
        let evidence = response
            .into_evidence(&request, selection)
            .expect("evidence");

        assert_eq!(evidence.query_evidence().plan(), request.plan());
        assert_eq!(evidence.query_evidence().receipt(), &receipt);
        assert_eq!(evidence.trust_root().key_id().as_str(), "fact.read.key");
    }

    #[test]
    fn response_rejects_unpinned_field_summaries() {
        let fact_ref = internal_fact_ref(1);
        let row = FactIndexReadRow::new(fact_ref.clone(), vec![summary_value(42)]);
        let receipt = receipt_without_summaries(&plan(FactAudience::Control), fact_ref);

        let error =
            FactIndexReadResponse::new(vec![row], receipt, trust_root()).expect_err("unpinned");

        assert_eq!(
            error,
            FactIndexReadError::Receipt {
                reason: FactIndexReceiptFailure::UnpinnedFieldSummaries,
            }
        );
    }

    #[test]
    fn response_rejects_mismatched_row_ref() {
        let receipt = receipt_without_summaries(&plan(FactAudience::Control), internal_fact_ref(1));
        let row = FactIndexReadRow::new(internal_fact_ref(2), Vec::new());

        let error =
            FactIndexReadResponse::new(vec![row], receipt, trust_root()).expect_err("mismatch");

        assert_eq!(
            error,
            FactIndexReadError::Receipt {
                reason: FactIndexReceiptFailure::RowRefMismatch,
            }
        );
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
        let query = CanonicalValue::object([(
            "kind",
            CanonicalValue::String("collector.checkpoint".to_owned()),
        )])
        .expect("canonical query");
        CanonicalFactQueryPlan::new(
            StoreScopeRef::new("mfm.store.default").expect("store scope"),
            FactQueryScope::new(audience, FactVisibilityScope::Default),
            FactQueryCompilerVersion::new("mfm.facts.query.v1").expect("compiler"),
            FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
            digest(0x20),
            ScopeDecisionEvidence::new(digest(0x21)),
            CanonicalJsonBytes::from_value(&query),
            FactOrdering::new(
                FactOrderingName::new("metadata.recorded_at.desc").expect("ordering"),
                vec![FactOrderingTerm::new(
                    FactFieldId::new("metadata.recorded_at").expect("field"),
                    SortDirection::Descending,
                    NullOrdering::Last,
                    true,
                )],
            )
            .expect("fact ordering"),
            Some(1),
        )
        .expect("plan")
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
        let returned_refs = vec![fact_ref];
        let result_set_digest = mfm_facts::fact_query_result_set_digest(
            &returned_refs,
            returned_field_summaries.as_ref(),
        )
        .expect("result set digest");
        let read_frontier = StoreReadFrontier::new(
            StoreScopeRef::new("mfm.store.default").expect("store scope"),
            FactQueryScope::new(FactAudience::Control, FactVisibilityScope::Default),
            DescriptorCatalogWatermark::new(1),
            FactProjectionGeneration::new(1),
            11,
            StoreCommitWatermark::new(11),
        );
        let plan_hash = mfm_facts::fact_query_plan_hash(plan).expect("plan hash");
        let receipt_hash = mfm_facts::fact_query_receipt_body_hash_from_parts(
            &plan_hash,
            &read_frontier,
            StoreReadFrontierType::Snapshot,
            &returned_refs,
            returned_field_summaries.as_ref(),
            &result_set_digest,
            QueryResultCardinality::Exact(1),
        )
        .expect("receipt hash");
        FactQueryReceipt::new(
            read_frontier,
            StoreReadFrontierType::Snapshot,
            returned_refs,
            returned_field_summaries,
            result_set_digest,
            QueryResultCardinality::Exact(1),
            receipt_hash,
            StoreReceiptAuthentication::new(
                StoreIdentity::new("store.default").expect("store identity"),
                StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
                Some(StoreKeyId::new("fact.read.key").expect("key id")),
                vec![0x11; 64],
            )
            .expect("auth"),
        )
    }

    fn summary_value(value: u64) -> ReturnedFieldValueSummary {
        ReturnedFieldValueSummary::new(
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
            observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
            visibility: FactVisibility::indexed_default(FactAudience::Control),
            fact_kind: mfm_facts::FactKind::new("collector.checkpoint").expect("kind"),
            fact_descriptor_hash: digest(seed + 2),
            fact_subject_namespace_hash: digest(seed + 3),
            fact_key: mfm_facts::FactKey::from_digest(digest(seed + 4)),
            subject_material_hash: digest(seed + 5),
            request_schema_id: None,
            request_hash: None,
            response_schema_id: schema_id(seed + 6),
            response_hash: digest(seed + 7),
            artifact_id: ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 8),
            ),
            artifact_evidence_hash: digest(seed + 9),
            capability_kind: CapabilityKind::new(
                "mfm.fact",
                "index.read",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 10),
            )
            .expect("capability kind"),
            capability_version: CapabilityVersion::new("mfm.fact.index.read.v1")
                .expect("capability version"),
            adapter_kind: AdapterKind::new(
                "mfm.fact",
                "index.adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 11),
            )
            .expect("adapter kind"),
            adapter_version: AdapterVersion::new("mfm.fact.index.adapter.v1")
                .expect("adapter version"),
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
