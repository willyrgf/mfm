//! Portable structured-run export format and offline verification.
//!
//! `mfm-replay` owns the only portable format, encoder/decoder, canonical
//! validation, digest rules, and offline validator. Application code requests
//! projection through this module and does not define a second format.

use mfm_canonical::{sha256_digest_bytes, RecoverabilityContract};
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, RunId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, CommittedBatch, JournalHead, RunRecord, SemanticHead,
};
use mfm_store::structured::{
    recorded_evidence_from_verified, verify_offline_recorded_history, ExportRunEvidence,
    ProgramVerifier, PublicPhysicalBindingVerifier, RawRunHistory, RecordedRunEvidence,
    StructuredStoreError,
};
use serde::{Deserialize, Serialize};

use crate::structured::{project_replay_result, StructuredReplayError, StructuredReplayResult};

/// Exact media type of one structured portable run export document.
pub const PORTABLE_RUN_EXPORT_MEDIA_TYPE: &str =
    "application/vnd.mfm.structured-run-export.v1+json";

/// Current portable-export contract version string retained inside the document.
pub const PORTABLE_EXPORT_VERSION: &str = "mfm.structured-portable-run-export.v1";

/// Annex schema contract for the portable export document.
pub const PORTABLE_EXPORT_SCHEMA_CONTRACT: &str = "mfm.portable-run-export-stream.v1";

/// Maximum canonical portable-export document bytes.
pub const MAX_PORTABLE_EXPORT_BYTES: u64 = 512 * 1024 * 1024;

/// Maximum committed batches in one portable export.
pub const MAX_PORTABLE_BATCHES: usize = 1_048_576;

/// Maximum objects retained across every batch of one portable export.
pub const MAX_PORTABLE_OBJECTS: usize = 1_048_576;

/// Maximum recursive source-run identities listed in one portable export.
pub const MAX_PORTABLE_SOURCE_RUNS: usize = 4_096;

/// Closed portable-export scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    /// Export through the current semantic head, including every full atomic batch selected.
    Semantic,
    /// Export through the exact current physical journal head.
    Audit,
}

impl ExportKind {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Audit => "audit",
        }
    }
}

/// Explicit semantic and physical fixation retained by one portable export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableFixation {
    /// Semantic head fixed by the export.
    pub semantic_head: SemanticHead,
    /// Physical journal head fixed by the export.
    pub journal_head: JournalHead,
    /// Store lineage of every included batch.
    pub store_scope_id: StoreScopeId,
    /// Writer epoch of every included batch.
    pub store_epoch: StoreEpoch,
}

/// One strict portable structured-run export document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableRunExport {
    version: String,
    kind: ExportKind,
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    fixation: PortableFixation,
    /// Exact source-run identities authorized and required by this export.
    source_run_ids: Vec<RunId>,
    /// Exact canonical committed-batch envelopes through the fixed head.
    batches: Vec<CommittedBatch>,
    /// Top-level digest over every required member except this field.
    digest: ContentDigest,
}

impl PortableRunExport {
    /// Builds one portable export from sealed export evidence after authorization.
    pub fn from_export_evidence(
        evidence: &ExportRunEvidence,
        kind: ExportKind,
    ) -> Result<Self, PortableExportError> {
        let batches = select_batches(evidence, kind)?;
        if batches.is_empty() {
            return Err(PortableExportError::Invalid);
        }
        let first = &batches[0];
        let last = batches.last().expect("non-empty batches");
        let store_scope_id = first.store_scope_id.clone();
        let store_epoch = first.store_epoch;
        for batch in &batches {
            if batch.store_scope_id != store_scope_id || batch.store_epoch != store_epoch {
                return Err(PortableExportError::Invalid);
            }
        }
        let source_run_ids = evidence
            .direct_source_run_ids()
            .map_err(|_| PortableExportError::Invalid)?
            .into_iter()
            .collect::<Vec<_>>();
        if source_run_ids.len() > MAX_PORTABLE_SOURCE_RUNS {
            return Err(PortableExportError::TooLarge);
        }
        let mut document = Self {
            version: PORTABLE_EXPORT_VERSION.to_owned(),
            kind,
            store_scope_id: store_scope_id.clone(),
            tenant_scope_id: evidence.admission().tenant_scope_id.clone(),
            run_id: evidence.run_id().clone(),
            fixation: PortableFixation {
                semantic_head: evidence.semantic_head().clone(),
                journal_head: last.head.clone(),
                store_scope_id,
                store_epoch,
            },
            source_run_ids,
            batches,
            digest: placeholder_digest()?,
        };
        document.digest = document.compute_digest()?;
        document.validate_structure()?;
        Ok(document)
    }

    /// Encodes the exact canonical portable-export document bytes.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, PortableExportError> {
        let canonical = canonical_json(self).map_err(|_| PortableExportError::Invalid)?;
        let bytes = canonical.as_bytes().to_vec();
        if bytes.len() as u64 > MAX_PORTABLE_EXPORT_BYTES {
            return Err(PortableExportError::TooLarge);
        }
        Ok(bytes)
    }

    /// Returns the content reference for the exact encoded bytes.
    pub fn content_ref(&self) -> Result<ContentRef, PortableExportError> {
        let bytes = self.to_canonical_bytes()?;
        let contract =
            RecoverabilityContract::embedded().map_err(|_| PortableExportError::Invalid)?;
        let schema_id = contract
            .schema_id(PORTABLE_EXPORT_SCHEMA_CONTRACT)
            .map_err(|_| PortableExportError::Invalid)?
            .clone();
        ContentRef::new(schema_id, contract.raw_content_digest(&bytes))
            .map_err(|_| PortableExportError::Invalid)
    }

    /// Strictly decodes and validates one portable export without ambient IO.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self, PortableExportError> {
        if bytes.is_empty() || bytes.len() as u64 > MAX_PORTABLE_EXPORT_BYTES {
            return Err(PortableExportError::TooLarge);
        }
        // Bounded structural precheck before full typed allocation.
        let depth = json_depth(bytes).ok_or(PortableExportError::Invalid)?;
        if depth > 64 {
            return Err(PortableExportError::TooLarge);
        }
        let contract =
            RecoverabilityContract::embedded().map_err(|_| PortableExportError::Invalid)?;
        let validated = contract
            .strict_decode(PORTABLE_EXPORT_SCHEMA_CONTRACT, bytes)
            .map_err(|_| PortableExportError::Invalid)?;
        let export: Self = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| PortableExportError::Invalid)?;
        export.validate_structure()?;
        let recomputed = export.compute_digest()?;
        if export.digest != recomputed {
            return Err(PortableExportError::Invalid);
        }
        let canonical = export.to_canonical_bytes()?;
        if canonical.as_slice() != bytes {
            return Err(PortableExportError::Invalid);
        }
        Ok(export)
    }

    /// Offline verification using only bundle bytes and an explicit trust snapshot.
    pub fn verify_offline(
        bytes: &[u8],
        trust: &ReplayTrustSnapshot<'_>,
    ) -> Result<StructuredReplayResult, PortableExportError> {
        let export = Self::strict_decode(bytes)?;
        let evidence = export.fold_with_trust(trust)?;
        project_replay_result(&evidence).map_err(|_| PortableExportError::Invalid)
    }

    /// Folds this export through the store's sole offline entry point.
    pub fn fold_with_trust(
        &self,
        trust: &ReplayTrustSnapshot<'_>,
    ) -> Result<RecordedRunEvidence, PortableExportError> {
        self.validate_structure()?;
        let recomputed = self.compute_digest()?;
        if self.digest != recomputed {
            return Err(PortableExportError::Invalid);
        }
        let raw = RawRunHistory {
            run_id: self.run_id.clone(),
            batches: self.batches.clone(),
        };
        let verified = verify_offline_recorded_history(
            raw,
            trust.program_verifier,
            trust.physical_binding_verifier,
        )
        .map_err(classify_fold_error)?;
        if verified.run_id() != &self.run_id
            || verified.admission().tenant_scope_id != self.tenant_scope_id
            || verified.journal_head() != &self.fixation.journal_head
            || verified.semantic_head() != &self.fixation.semantic_head
        {
            return Err(PortableExportError::Invalid);
        }
        Ok(recorded_evidence_from_verified(verified))
    }

    fn compute_digest(&self) -> Result<ContentDigest, PortableExportError> {
        let body = PortableDigestBody {
            version: &self.version,
            kind: self.kind,
            store_scope_id: &self.store_scope_id,
            tenant_scope_id: &self.tenant_scope_id,
            run_id: &self.run_id,
            fixation: &self.fixation,
            source_run_ids: &self.source_run_ids,
            batches: &self.batches,
        };
        let canonical = canonical_json(&body).map_err(|_| PortableExportError::Invalid)?;
        Ok(ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(canonical.as_bytes()),
        ))
    }

    fn validate_structure(&self) -> Result<(), PortableExportError> {
        if self.version != PORTABLE_EXPORT_VERSION
            || self.batches.is_empty()
            || self.batches.len() > MAX_PORTABLE_BATCHES
            || self.source_run_ids.len() > MAX_PORTABLE_SOURCE_RUNS
            || self.store_scope_id != self.fixation.store_scope_id
        {
            return Err(PortableExportError::Invalid);
        }
        let mut object_count = 0usize;
        let mut previous: Option<JournalHead> = None;
        for batch in &self.batches {
            if batch.store_scope_id != self.fixation.store_scope_id
                || batch.store_epoch != self.fixation.store_epoch
                || batch.predecessor != previous
                || batch.records.is_empty()
            {
                return Err(PortableExportError::Invalid);
            }
            object_count = object_count
                .checked_add(batch.objects.len())
                .ok_or(PortableExportError::TooLarge)?;
            if object_count > MAX_PORTABLE_OBJECTS {
                return Err(PortableExportError::TooLarge);
            }
            previous = Some(batch.head.clone());
        }
        let last = self.batches.last().expect("non-empty");
        if last.head != self.fixation.journal_head {
            return Err(PortableExportError::Invalid);
        }
        // Semantic selection must retain the full atomic batch that carries the semantic
        // cutoff, including an adjacent RunClosed when present.
        match self.kind {
            ExportKind::Semantic => {
                let semantic_sequence = match &self.fixation.semantic_head {
                    SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
                    SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
                };
                if last.head.run_sequence != semantic_sequence
                    && !self.batches.iter().any(|batch| {
                        batch.head.run_sequence == semantic_sequence
                            && batch
                                .records
                                .iter()
                                .any(|record| matches!(record.record, RunRecord::RunClosed(_)))
                    })
                {
                    // The export head may equal semantic cutoff sequence; when the terminal
                    // transition shares a batch with RunClosed, the last batch includes both.
                    if last.head.run_sequence < semantic_sequence {
                        return Err(PortableExportError::Invalid);
                    }
                }
            }
            ExportKind::Audit => {}
        }
        if self
            .source_run_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(PortableExportError::Invalid);
        }
        Ok(())
    }
}

/// Explicit trust material for offline portable verification.
pub struct ReplayTrustSnapshot<'a> {
    program_verifier: &'a dyn ProgramVerifier,
    physical_binding_verifier: &'a dyn PublicPhysicalBindingVerifier,
}

impl<'a> ReplayTrustSnapshot<'a> {
    /// Binds concrete callback-free verifiers without live store or network access.
    pub fn new(
        program_verifier: &'a dyn ProgramVerifier,
        physical_binding_verifier: &'a dyn PublicPhysicalBindingVerifier,
    ) -> Self {
        Self {
            program_verifier,
            physical_binding_verifier,
        }
    }
}

/// Stable portable-export failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortableExportError {
    /// The document is malformed, non-canonical, or fails offline fold verification.
    #[error("portable export is invalid")]
    Invalid,
    /// The document exceeds a frozen byte, count, or depth bound.
    #[error("portable export exceeds frozen bounds")]
    TooLarge,
}

impl From<PortableExportError> for StructuredReplayError {
    fn from(error: PortableExportError) -> Self {
        match error {
            PortableExportError::Invalid => StructuredReplayError::InvalidRecordedHistory,
            PortableExportError::TooLarge => StructuredReplayError::InvalidRecordedHistory,
        }
    }
}

#[derive(Serialize)]
struct PortableDigestBody<'a> {
    version: &'a str,
    kind: ExportKind,
    store_scope_id: &'a StoreScopeId,
    tenant_scope_id: &'a TenantScopeId,
    run_id: &'a RunId,
    fixation: &'a PortableFixation,
    source_run_ids: &'a [RunId],
    batches: &'a [CommittedBatch],
}

fn select_batches(
    evidence: &ExportRunEvidence,
    kind: ExportKind,
) -> Result<Vec<CommittedBatch>, PortableExportError> {
    let batches = evidence.batches();
    if batches.is_empty() {
        return Err(PortableExportError::Invalid);
    }
    match kind {
        ExportKind::Audit => Ok(batches.to_vec()),
        ExportKind::Semantic => {
            let cutoff = match evidence.semantic_head() {
                SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
                SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
            };
            let selected = batches
                .iter()
                .filter(|batch| batch.head.run_sequence <= cutoff)
                .cloned()
                .collect::<Vec<_>>();
            // Include the full atomic batch that contains the semantic cutoff. When
            // RunClosed shares that batch, it is retained with the transition.
            if selected.is_empty()
                || selected
                    .last()
                    .is_none_or(|batch| batch.head.run_sequence != cutoff)
            {
                return Err(PortableExportError::Invalid);
            }
            Ok(selected)
        }
    }
}

fn placeholder_digest() -> Result<ContentDigest, PortableExportError> {
    Ok(ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        sha256_digest_bytes(b"mfm.portable-export.digest-placeholder"),
    ))
}

fn classify_fold_error(error: StructuredStoreError) -> PortableExportError {
    match error {
        StructuredStoreError::RunNotFound
        | StructuredStoreError::InvalidHistory
        | StructuredStoreError::CandidateRejected
        | StructuredStoreError::Certification
        | StructuredStoreError::StaleHead
        | StructuredStoreError::AppendConflict => PortableExportError::Invalid,
        StructuredStoreError::BackendUnavailable | StructuredStoreError::AcknowledgementUnknown => {
            PortableExportError::Invalid
        }
    }
}

fn json_depth(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0usize;
    let mut max = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for &byte in bytes {
        if in_string {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.checked_add(1)?;
                max = max.max(depth);
            }
            b'}' | b']' => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    Some(max)
}

#[cfg(test)]
mod tests {
    use super::{json_depth, PortableExportError, PortableRunExport, MAX_PORTABLE_EXPORT_BYTES};

    #[test]
    fn oversized_and_overdeep_documents_fail_before_full_decode() {
        let oversize = vec![b'{'; (MAX_PORTABLE_EXPORT_BYTES as usize) + 1];
        assert_eq!(
            PortableRunExport::strict_decode(&oversize),
            Err(PortableExportError::TooLarge)
        );
        let deep = format!("{}{}", "{".repeat(65), "}".repeat(65));
        assert_eq!(json_depth(deep.as_bytes()), Some(65));
        assert_eq!(
            PortableRunExport::strict_decode(deep.as_bytes()),
            Err(PortableExportError::TooLarge)
        );
        assert!(PortableRunExport::strict_decode(b"").is_err());
        assert!(PortableRunExport::strict_decode(br#"{"kind":"semantic"}"#).is_err());
    }
}
