use mfm_ids::ContentDigest;

use crate::*;

/// Descriptor catalog watermark bound into query receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DescriptorCatalogWatermark(u64);

impl DescriptorCatalogWatermark {
    /// Creates a descriptor catalog watermark.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the watermark value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Fact projection generation or rebuild id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactProjectionGeneration(u64);

impl FactProjectionGeneration {
    /// Creates a fact projection generation.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the generation value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Store-wide commit coordinate bound into query receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreCommitOrder(u64);

impl StoreCommitOrder {
    /// Empty store frontier used when a query includes no committed facts.
    pub const EMPTY: Self = Self(0);

    /// First committed store coordinate.
    pub const FIRST: Self = Self(1);

    /// Creates a store commit coordinate.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the coordinate value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Advances the coordinate, returning an error-free `None` only on overflow.
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Store read frontier type for fact query receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreReadFrontierType {
    /// Receipt was evaluated over a complete authorized snapshot.
    Snapshot,
    /// Receipt was evaluated over an authorized prefix frontier.
    Prefix,
}

impl_fact_tag!(StoreReadFrontierType, "store read frontier type", pub(crate), "Returns the canonical store read frontier type tag.", {
    Self::Snapshot => "snapshot",
    Self::Prefix => "prefix",
});

/// Semantic read frontier bound into a fact query receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreReadFrontier {
    pub(crate) store_scope: StoreScopeRef,
    pub(crate) query_scope: FactQueryScope,
    pub(crate) descriptor_catalog_watermark: DescriptorCatalogWatermark,
    pub(crate) projection_generation: FactProjectionGeneration,
    pub(crate) store_commit_order: StoreCommitOrder,
}

impl StoreReadFrontier {
    /// Creates a store read frontier.
    pub fn new(
        store_scope: StoreScopeRef,
        query_scope: FactQueryScope,
        descriptor_catalog_watermark: DescriptorCatalogWatermark,
        projection_generation: FactProjectionGeneration,
        store_commit_order: StoreCommitOrder,
    ) -> Self {
        Self {
            store_scope,
            query_scope,
            descriptor_catalog_watermark,
            projection_generation,
            store_commit_order,
        }
    }

    /// Returns the store scope.
    pub const fn store_scope(&self) -> &StoreScopeRef {
        &self.store_scope
    }

    /// Returns the query scope.
    pub const fn query_scope(&self) -> &FactQueryScope {
        &self.query_scope
    }

    /// Returns the descriptor catalog watermark.
    pub const fn descriptor_catalog_watermark(&self) -> DescriptorCatalogWatermark {
        self.descriptor_catalog_watermark
    }

    /// Returns the projection generation.
    pub const fn projection_generation(&self) -> FactProjectionGeneration {
        self.projection_generation
    }

    /// Returns the maximum committed store coordinate included by the query.
    pub const fn store_commit_order(&self) -> StoreCommitOrder {
        self.store_commit_order
    }
}

/// Store receipt authentication scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreReceiptAuthenticationScheme {
    /// Local Ed25519 signature over a SHA-256 JCS receipt hash.
    LocalEd25519Sha256JcsV1,
}

impl_fact_tag!(StoreReceiptAuthenticationScheme, "store receipt authentication scheme", pub, "Returns the canonical authentication scheme tag.", {
    Self::LocalEd25519Sha256JcsV1 => "local_ed25519_sha256_jcs_v1",
});

/// Store-owned receipt authentication metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreReceiptAuthentication {
    pub(crate) store_identity: StoreIdentity,
    pub(crate) scheme: StoreReceiptAuthenticationScheme,
    pub(crate) key_id: Option<StoreKeyId>,
    pub(crate) signature_or_mac: Vec<u8>,
}

impl StoreReceiptAuthentication {
    /// Creates receipt authentication metadata.
    pub fn new(
        store_identity: StoreIdentity,
        scheme: StoreReceiptAuthenticationScheme,
        key_id: Option<StoreKeyId>,
        signature_or_mac: Vec<u8>,
    ) -> Result<Self> {
        match scheme {
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1 => {
                if key_id.is_none() {
                    return Err(FactError::descriptor(
                        "local Ed25519 receipt authentication requires a key id",
                    ));
                }
                if signature_or_mac.len() != 64 {
                    return Err(FactError::descriptor(
                        "local Ed25519 receipt authentication requires a 64-byte signature",
                    ));
                }
            }
        }
        Ok(Self {
            store_identity,
            scheme,
            key_id,
            signature_or_mac,
        })
    }

    /// Returns the store identity.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the authentication scheme.
    pub const fn scheme(&self) -> StoreReceiptAuthenticationScheme {
        self.scheme
    }

    /// Returns the optional key id.
    pub const fn key_id(&self) -> Option<&StoreKeyId> {
        self.key_id.as_ref()
    }

    /// Returns signature or MAC bytes.
    pub fn signature_or_mac(&self) -> &[u8] {
        &self.signature_or_mac
    }
}

/// Returned summaries for one fact ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedFactFieldSummary {
    pub(crate) fact_claim_id: FactClaimId,
    pub(crate) fields: Vec<FactFieldValue>,
}

impl ReturnedFactFieldSummary {
    /// Creates returned field summaries for one fact.
    pub fn new(fact_claim_id: FactClaimId, fields: Vec<FactFieldValue>) -> Self {
        Self {
            fact_claim_id,
            fields,
        }
    }

    /// Returns the fact claim id.
    pub const fn fact_claim_id(&self) -> &FactClaimId {
        &self.fact_claim_id
    }

    /// Returns summarized fields in retained order.
    pub fn fields(&self) -> &[FactFieldValue] {
        &self.fields
    }
}

/// Returned field summaries pinned in a query receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedFieldSummaries {
    pub(crate) summaries: Vec<ReturnedFactFieldSummary>,
}

impl ReturnedFieldSummaries {
    /// Creates returned field summaries.
    pub fn new(summaries: Vec<ReturnedFactFieldSummary>) -> Self {
        Self { summaries }
    }

    /// Returns fact summaries in receipt order.
    pub fn summaries(&self) -> &[ReturnedFactFieldSummary] {
        &self.summaries
    }
}

/// Cardinality statement for a query result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QueryResultCardinality {
    /// The receipt represents the exact number of matching rows.
    Exact(u64),
    /// The receipt hit a limit and represents at least this many matching rows.
    AtLeast(u64),
}

/// Store-owned receipt for a canonical fact query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryReceipt {
    pub(crate) read_frontier: StoreReadFrontier,
    pub(crate) frontier_type: StoreReadFrontierType,
    pub(crate) returned_refs: Vec<InternalFactRef>,
    pub(crate) returned_field_summaries: Option<ReturnedFieldSummaries>,
    pub(crate) result_set_digest: ContentDigest,
    pub(crate) result_cardinality: QueryResultCardinality,
    pub(crate) store_receipt_hash: ContentDigest,
    pub(crate) store_receipt_authentication: StoreReceiptAuthentication,
}

impl FactQueryReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        read_frontier: StoreReadFrontier,
        frontier_type: StoreReadFrontierType,
        returned_refs: Vec<InternalFactRef>,
        returned_field_summaries: Option<ReturnedFieldSummaries>,
        result_set_digest: ContentDigest,
        result_cardinality: QueryResultCardinality,
        store_receipt_hash: ContentDigest,
        store_receipt_authentication: StoreReceiptAuthentication,
    ) -> Self {
        Self {
            read_frontier,
            frontier_type,
            returned_refs,
            returned_field_summaries,
            result_set_digest,
            result_cardinality,
            store_receipt_hash,
            store_receipt_authentication,
        }
    }

    /// Returns the read frontier.
    pub const fn read_frontier(&self) -> &StoreReadFrontier {
        &self.read_frontier
    }

    /// Returns the frontier type.
    pub const fn frontier_type(&self) -> StoreReadFrontierType {
        self.frontier_type
    }

    /// Returns refs in receipt order.
    pub fn returned_refs(&self) -> &[InternalFactRef] {
        &self.returned_refs
    }

    /// Returns optional field summaries.
    pub const fn returned_field_summaries(&self) -> Option<&ReturnedFieldSummaries> {
        self.returned_field_summaries.as_ref()
    }

    /// Returns the result-set digest.
    pub const fn result_set_digest(&self) -> &ContentDigest {
        &self.result_set_digest
    }

    /// Returns result cardinality.
    pub const fn result_cardinality(&self) -> QueryResultCardinality {
        self.result_cardinality
    }

    /// Returns the store receipt hash.
    pub const fn store_receipt_hash(&self) -> &ContentDigest {
        &self.store_receipt_hash
    }

    /// Returns store receipt authentication.
    pub const fn store_receipt_authentication(&self) -> &StoreReceiptAuthentication {
        &self.store_receipt_authentication
    }
}

/// One row returned by a canonical fact query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryResultRow {
    pub(crate) fact_ref: InternalFactRef,
    pub(crate) returned_fields: Vec<FactFieldValue>,
}

impl FactQueryResultRow {
    /// Creates a fact query result row.
    pub fn new(fact_ref: InternalFactRef, returned_fields: Vec<FactFieldValue>) -> Self {
        Self {
            fact_ref,
            returned_fields,
        }
    }

    /// Returns the internal fact reference for this row.
    pub const fn fact_ref(&self) -> &InternalFactRef {
        &self.fact_ref
    }

    /// Returns requested returned field summaries present on this row.
    pub fn returned_fields(&self) -> &[FactFieldValue] {
        &self.returned_fields
    }
}

/// Unsigned material for a fact-query receipt body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryReceiptMaterial {
    pub(crate) read_frontier: StoreReadFrontier,
    pub(crate) frontier_type: StoreReadFrontierType,
    pub(crate) returned_refs: Vec<InternalFactRef>,
    pub(crate) returned_field_summaries: Option<ReturnedFieldSummaries>,
    pub(crate) result_set_digest: ContentDigest,
    pub(crate) result_cardinality: QueryResultCardinality,
    pub(crate) store_receipt_hash: ContentDigest,
}

impl FactQueryReceiptMaterial {
    fn from_result_parts(
        plan_hash: &ContentDigest,
        read_frontier: StoreReadFrontier,
        frontier_type: StoreReadFrontierType,
        returned_refs: Vec<InternalFactRef>,
        returned_field_summaries: Option<ReturnedFieldSummaries>,
        result_cardinality: QueryResultCardinality,
    ) -> Result<Self> {
        let result_set_digest =
            fact_query_result_set_digest(&returned_refs, returned_field_summaries.as_ref())?;
        let store_receipt_hash = fact_query_receipt_body_hash_from_parts(
            plan_hash,
            &read_frontier,
            frontier_type,
            &returned_refs,
            returned_field_summaries.as_ref(),
            &result_set_digest,
            result_cardinality,
        )?;
        Ok(Self {
            read_frontier,
            frontier_type,
            returned_refs,
            returned_field_summaries,
            result_set_digest,
            result_cardinality,
            store_receipt_hash,
        })
    }

    /// Builds unsigned receipt material from query result rows.
    pub fn from_rows(
        plan_hash: &ContentDigest,
        read_frontier: StoreReadFrontier,
        frontier_type: StoreReadFrontierType,
        rows: &[FactQueryResultRow],
        include_returned_field_summaries: bool,
        limit: Option<u64>,
    ) -> Result<Self> {
        let returned_refs = returned_refs_for_rows(rows);
        let returned_field_summaries =
            returned_field_summaries_for_rows(rows, include_returned_field_summaries);
        let result_cardinality = cardinality_for_rows(rows.len(), limit)?;
        Self::from_result_parts(
            plan_hash,
            read_frontier,
            frontier_type,
            returned_refs,
            returned_field_summaries,
            result_cardinality,
        )
    }

    /// Returns the receipt body hash that should be authenticated by the store.
    pub const fn store_receipt_hash(&self) -> &ContentDigest {
        &self.store_receipt_hash
    }

    /// Converts unsigned material into an authenticated receipt.
    pub fn into_receipt(
        self,
        store_receipt_authentication: StoreReceiptAuthentication,
    ) -> FactQueryReceipt {
        FactQueryReceipt::from_parts(
            self.read_frontier,
            self.frontier_type,
            self.returned_refs,
            self.returned_field_summaries,
            self.result_set_digest,
            self.result_cardinality,
            self.store_receipt_hash,
            store_receipt_authentication,
        )
    }
}

fn returned_refs_for_rows(rows: &[FactQueryResultRow]) -> Vec<InternalFactRef> {
    rows.iter().map(|row| row.fact_ref().clone()).collect()
}

fn returned_field_summaries_for_rows(
    rows: &[FactQueryResultRow],
    include_summaries: bool,
) -> Option<ReturnedFieldSummaries> {
    include_summaries.then(|| {
        ReturnedFieldSummaries::new(
            rows.iter()
                .map(|row| {
                    ReturnedFactFieldSummary::new(
                        row.fact_ref().fact_claim_id().clone(),
                        row.returned_fields().to_vec(),
                    )
                })
                .collect(),
        )
    })
}

fn cardinality_for_rows(row_count: usize, limit: Option<u64>) -> Result<QueryResultCardinality> {
    let row_count = u64::try_from(row_count)
        .map_err(|_| FactError::descriptor("fact query result row count overflowed u64"))?;
    match limit {
        Some(limit) if row_count == limit => Ok(QueryResultCardinality::AtLeast(row_count)),
        _ => Ok(QueryResultCardinality::Exact(row_count)),
    }
}

/// Result of a canonical fact query, paired with its authenticated receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryResult {
    pub(crate) rows: Vec<FactQueryResultRow>,
    pub(crate) receipt: FactQueryReceipt,
}

impl FactQueryResult {
    /// Creates a fact query result and validates row/receipt alignment.
    pub fn new(rows: Vec<FactQueryResultRow>, receipt: FactQueryReceipt) -> Result<Self> {
        validate_fact_query_result_rows(&rows, &receipt)
            .map_err(fact_query_result_mismatch_error)?;
        Ok(Self { rows, receipt })
    }

    /// Returns matching fact query rows in plan ordering.
    pub fn rows(&self) -> &[FactQueryResultRow] {
        &self.rows
    }

    /// Returns the authenticated store receipt for this query result.
    pub const fn receipt(&self) -> &FactQueryReceipt {
        &self.receipt
    }
}

/// Closed mismatch reason when query result rows do not match their authenticated receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactQueryResultMismatch {
    /// Row count differed from the returned refs in the receipt.
    RowCountMismatch,
    /// A row ref differed from the corresponding receipt ref.
    RowRefMismatch,
    /// Returned fields were supplied without receipt-pinned summaries.
    UnpinnedFieldSummaries,
    /// Receipt summary count differed from the returned refs in the receipt.
    SummaryCountMismatch,
    /// A receipt summary claim id differed from the corresponding row ref.
    SummaryClaimMismatch,
    /// A row's returned fields differed from receipt-pinned summaries.
    SummaryValueMismatch,
}

/// Builds query result rows directly from receipt-returned refs and summaries.
pub fn fact_query_result_rows_from_receipt(receipt: &FactQueryReceipt) -> Vec<FactQueryResultRow> {
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
            FactQueryResultRow::new(fact_ref.clone(), returned_fields)
        })
        .collect()
}

/// Validates that query result rows align with the authenticated receipt shape.
pub fn validate_fact_query_result_rows(
    rows: &[FactQueryResultRow],
    receipt: &FactQueryReceipt,
) -> std::result::Result<(), FactQueryResultMismatch> {
    if rows.len() != receipt.returned_refs().len() {
        return Err(FactQueryResultMismatch::RowCountMismatch);
    }
    for (row, fact_ref) in rows.iter().zip(receipt.returned_refs()) {
        if row.fact_ref() != fact_ref {
            return Err(FactQueryResultMismatch::RowRefMismatch);
        }
    }

    let Some(summaries) = receipt.returned_field_summaries() else {
        if rows.iter().any(|row| !row.returned_fields().is_empty()) {
            return Err(FactQueryResultMismatch::UnpinnedFieldSummaries);
        }
        return Ok(());
    };
    if summaries.summaries().len() != rows.len() {
        return Err(FactQueryResultMismatch::SummaryCountMismatch);
    }
    for (row, summary) in rows.iter().zip(summaries.summaries()) {
        if summary.fact_claim_id() != row.fact_ref().fact_claim_id() {
            return Err(FactQueryResultMismatch::SummaryClaimMismatch);
        }
        if summary.fields() != row.returned_fields() {
            return Err(FactQueryResultMismatch::SummaryValueMismatch);
        }
    }
    Ok(())
}

fn fact_query_result_mismatch_error(mismatch: FactQueryResultMismatch) -> FactError {
    let message = match mismatch {
        FactQueryResultMismatch::RowCountMismatch => {
            "fact query result rows must align one-for-one with returned refs"
        }
        FactQueryResultMismatch::RowRefMismatch => {
            "fact query result row ref does not match returned ref"
        }
        FactQueryResultMismatch::UnpinnedFieldSummaries => {
            "fact query result rows contain returned fields not pinned by receipt"
        }
        FactQueryResultMismatch::SummaryCountMismatch => {
            "fact query result summaries must align one-for-one with rows"
        }
        FactQueryResultMismatch::SummaryClaimMismatch => {
            "fact query result summary claim id does not match row ref"
        }
        FactQueryResultMismatch::SummaryValueMismatch => {
            "fact query result row fields do not match returned field summaries"
        }
    };
    FactError::descriptor(message)
}

/// State-owned evidence describing selected receipt rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSelectionEvidence {
    pub(crate) selection_policy_hash: ContentDigest,
    pub(crate) selected_indices: Vec<u64>,
    pub(crate) selected_summaries_digest: Option<ContentDigest>,
}

impl FactSelectionEvidence {
    /// Creates selection evidence and validates canonical selected indices.
    pub fn new(
        selection_policy_hash: ContentDigest,
        selected_indices: Vec<u64>,
        selected_summaries_digest: Option<ContentDigest>,
    ) -> Result<Self> {
        for window in selected_indices.windows(2) {
            if window[0] >= window[1] {
                return Err(FactError::descriptor(
                    "selected indices must be sorted and unique",
                ));
            }
        }
        Ok(Self {
            selection_policy_hash,
            selected_indices,
            selected_summaries_digest,
        })
    }

    /// Returns the selection policy hash.
    pub const fn selection_policy_hash(&self) -> &ContentDigest {
        &self.selection_policy_hash
    }

    /// Returns selected receipt indices.
    pub fn selected_indices(&self) -> &[u64] {
        &self.selected_indices
    }

    /// Returns the optional selected summaries digest.
    pub const fn selected_summaries_digest(&self) -> Option<&ContentDigest> {
        self.selected_summaries_digest.as_ref()
    }
}

/// Replay evidence for a live fact query performed by a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryEvidence {
    pub(crate) plan: CanonicalFactQueryPlan,
    pub(crate) receipt: FactQueryReceipt,
    pub(crate) selection: FactSelectionEvidence,
}

impl FactQueryEvidence {
    /// Creates fact query replay evidence.
    pub const fn new(
        plan: CanonicalFactQueryPlan,
        receipt: FactQueryReceipt,
        selection: FactSelectionEvidence,
    ) -> Self {
        Self {
            plan,
            receipt,
            selection,
        }
    }

    /// Returns the canonical query plan.
    pub const fn plan(&self) -> &CanonicalFactQueryPlan {
        &self.plan
    }

    /// Returns the store query receipt.
    pub const fn receipt(&self) -> &FactQueryReceipt {
        &self.receipt
    }

    /// Returns state-owned selection evidence.
    pub const fn selection(&self) -> &FactSelectionEvidence {
        &self.selection
    }
}
