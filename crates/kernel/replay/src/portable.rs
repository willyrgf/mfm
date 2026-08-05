//! Portable structured-run frame streams and offline verification.
//!
//! `mfm-replay` owns the only portable format, frame validation, digest rules,
//! and offline validator. The application supplies sealed export evidence and
//! never constructs or interprets the wire frames.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mfm_canonical::{sha256_digest_bytes, RecoverabilityContract};
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, RunId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, CommittedBatch, JournalHead, RecordRef, SemanticHead, TenantFactCoordinate,
    TenantFactFrontier,
};
use mfm_store::structured::{
    export_fact_routes, recorded_evidence_from_verified, verify_offline_recorded_history,
    ExportEncoderView, ExportRunEvidence, PhysicalTargetIdentity, ProgramVerifier,
    PublicPhysicalBindingVerifier, RawRunHistory, RecordedRunEvidence, StructuredStoreError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::structured::{project_replay_result, StructuredReplayError, StructuredReplayResult};

struct ReplayEncoderConsumer;

impl mfm_authority_seal::ExportEncoderConsumerSeal for ReplayEncoderConsumer {}

/// Exact media type of the current bounded structured portable frame stream.
pub const PORTABLE_RUN_EXPORT_MEDIA_TYPE: &str =
    "application/vnd.mfm.structured-run-export-stream.v2";

/// Current portable-export contract version retained in the terminal seal.
pub const PORTABLE_EXPORT_VERSION: &str = "mfm.structured-portable-run-export-stream.v2";

/// Annex identity used for the stream's external [`ContentRef`].
pub const PORTABLE_EXPORT_SCHEMA_CONTRACT: &str = "mfm.portable-run-export-stream.v1";

/// Annex identity used to validate each individual frame.
pub const PORTABLE_FRAME_SCHEMA_CONTRACT: &str = "mfm.portable-run-export-frame.v1";

/// Maximum bytes in one complete portable frame stream.
pub use mfm_canonical::limits::{
    MAX_PORTABLE_BATCHES, MAX_PORTABLE_EXPORT_BYTES, MAX_PORTABLE_FACT_ROUTES, MAX_PORTABLE_FRAMES,
    MAX_PORTABLE_FRAME_BYTES, MAX_PORTABLE_OBJECTS, MAX_PORTABLE_SOURCE_RUNS,
};

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
    /// Tenant scope authorized for this exact portable closure.
    pub tenant_scope_id: TenantScopeId,
    /// Exact physical target, fence generation, release, and current incarnation.
    pub physical_target: PhysicalTargetIdentity,
}

/// One strict portable export assembled from sealed evidence.
///
/// The fields are deliberately private. Callers can request bytes or an
/// offline verification result, but cannot inspect or rewrite export content.
#[derive(PartialEq, Eq)]
pub struct PortableRunExport {
    version: String,
    kind: ExportKind,
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    fixation: PortableFixation,
    source_run_ids: Vec<RunId>,
    source_prefixes: Vec<PortableRunPrefix>,
    batches: Vec<CommittedBatch>,
    fact_routes: Vec<PortableFactRoute>,
    closure_reference: ContentDigest,
}

impl fmt::Debug for PortableRunExport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PortableRunExport")
            .field("version", &self.version)
            .field("kind", &self.kind)
            .field("store_scope_id", &self.store_scope_id)
            .field("tenant_scope_id", &self.tenant_scope_id)
            .field("run_id", &self.run_id)
            .field("source_run_count", &self.source_run_ids.len())
            .field("batch_count", &self.batches.len())
            .field("closure_reference", &self.closure_reference)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortableRunPrefix {
    run_id: RunId,
    tenant_scope_id: TenantScopeId,
    fixation: PortableFixation,
    batches: Vec<CommittedBatch>,
    fact_frontiers: Vec<TenantFactFrontier>,
    fact_routes: Vec<PortableFactRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableFactRoute {
    consumer_record: RecordRef,
    producer_transition: RecordRef,
    publication_frontier: TenantFactFrontier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PortableFrameKind {
    Batch,
    Seal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableFrame {
    kind: PortableFrameKind,
    ordinal: u64,
    payload: Value,
    previous_frame_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableBatchPayload {
    batch: CommittedBatch,
    run_id: RunId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableRunFixation {
    fact_frontiers: Vec<TenantFactFrontier>,
    fact_routes: Vec<PortableFactRoute>,
    fixation: PortableFixation,
    run_id: RunId,
    tenant_scope_id: TenantScopeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableSeal {
    closure_reference: ContentDigest,
    final_frame_digest: ContentDigest,
    fixation: PortableFixation,
    frame_chain_digest: ContentDigest,
    kind: ExportKind,
    root_run_id: RunId,
    run_fixations: Vec<PortableRunFixation>,
    source_run_ids: Vec<RunId>,
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    total_bytes: u64,
    total_frames: u64,
    version: String,
}

#[derive(Serialize)]
struct ClosureDigestBody<'a> {
    fixation: &'a PortableFixation,
    kind: ExportKind,
    root_run_id: &'a RunId,
    run_fixations: &'a [PortableRunFixation],
    source_run_ids: &'a [RunId],
    store_scope_id: &'a StoreScopeId,
    tenant_scope_id: &'a TenantScopeId,
    version: &'a str,
}

impl PortableRunExport {
    /// Builds one export from already authorized and sealed export evidence.
    pub fn from_export_evidence(
        evidence: &ExportRunEvidence,
        kind: ExportKind,
    ) -> Result<Self, PortableExportError> {
        evidence.with_encoder_view(ReplayEncoderConsumer, |view| {
            Self::from_encoder_view(view, kind)
        })
    }

    fn from_encoder_view(
        view: ExportEncoderView<'_>,
        kind: ExportKind,
    ) -> Result<Self, PortableExportError> {
        let batches = select_batches(&view, kind)?;
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
        let root_cutoff = batches
            .last()
            .map(|batch| batch.head.run_sequence)
            .ok_or(PortableExportError::Invalid)?;
        let root_fact_routes = view
            .fact_routes()
            .iter()
            .filter(|route| route.consumer_record().run_sequence <= root_cutoff)
            .map(portable_fact_route)
            .collect::<Vec<_>>();
        let sources = view.authorized_source_prefixes().collect::<Vec<_>>();
        let mut required_source_heads = BTreeMap::<RunId, u64>::new();
        for route in view.fact_routes() {
            if route.consumer_record().run_sequence <= root_cutoff
                && route.producer_transition().run_id != *view.run_id()
            {
                required_source_heads
                    .entry(route.producer_transition().run_id.clone())
                    .and_modify(|head| {
                        *head = (*head).max(route.producer_transition().run_sequence)
                    })
                    .or_insert(route.producer_transition().run_sequence);
            }
        }
        // Expand the route graph using the same maximum-required-head rule for
        // every producer. A producer is folded once at its largest routed
        // transition, never once per selected fact.
        let mut changed = true;
        while changed {
            changed = false;
            for source in &sources {
                let Some(required_head) = required_source_heads.get(source.run_id()).copied()
                else {
                    continue;
                };
                for route in source.fact_routes() {
                    if route.consumer_record().run_sequence <= required_head
                        && route.producer_transition().run_id != *source.run_id()
                    {
                        let entry = required_source_heads
                            .entry(route.producer_transition().run_id.clone())
                            .or_insert(0);
                        if *entry < route.producer_transition().run_sequence {
                            *entry = route.producer_transition().run_sequence;
                            changed = true;
                        }
                    }
                }
            }
        }
        let mut source_run_ids = required_source_heads.keys().cloned().collect::<Vec<_>>();
        source_run_ids.sort();
        let source_prefixes = sources
            .into_iter()
            .filter(|source| required_source_heads.contains_key(source.run_id()))
            .map(|source| {
                let required_head = required_source_heads.get(source.run_id()).copied();
                portable_prefix_from_source(
                    source,
                    required_head,
                    matches!(kind, ExportKind::Semantic),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if source_run_ids.len() > MAX_PORTABLE_SOURCE_RUNS {
            return Err(PortableExportError::TooLarge);
        }
        let fixation = PortableFixation {
            semantic_head: view.semantic_head().clone(),
            journal_head: last.head.clone(),
            store_scope_id: store_scope_id.clone(),
            store_epoch,
            tenant_scope_id: view.header().tenant_scope_id().clone(),
            physical_target: view.physical_target().clone(),
        };
        let mut export = Self {
            version: PORTABLE_EXPORT_VERSION.to_owned(),
            kind,
            store_scope_id,
            tenant_scope_id: view.header().tenant_scope_id().clone(),
            run_id: view.run_id().clone(),
            fixation,
            source_run_ids,
            source_prefixes,
            batches,
            fact_routes: root_fact_routes,
            closure_reference: placeholder_digest(),
        };
        export.closure_reference = export.compute_closure_reference()?;
        export.validate_structure()?;
        Ok(export)
    }

    /// Encodes the exact bounded newline-delimited canonical frame stream.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, PortableExportError> {
        self.validate_structure()?;
        encode_frames(self)
    }

    /// Returns the content reference for the exact encoded frame stream.
    pub fn content_ref(&self) -> Result<ContentRef, PortableExportError> {
        let bytes = self.to_canonical_bytes()?;
        let contract =
            RecoverabilityContract::embedded().map_err(|_| PortableExportError::Invalid)?;
        let schema_id = contract
            .schema_id(PORTABLE_EXPORT_SCHEMA_CONTRACT)
            .map_err(|_| PortableExportError::Invalid)?
            .clone();
        ContentRef::new(schema_id, raw_digest(&bytes)).map_err(|_| PortableExportError::Invalid)
    }

    /// Strictly decodes and validates one frame stream without ambient IO.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self, PortableExportError> {
        decode_frames(bytes)
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
        if trust.authorized_closure_reference != Some(&self.closure_reference) {
            return Err(PortableExportError::Invalid);
        }
        let Some(retained_release_trust) = trust.retained_release_trust else {
            return Err(PortableExportError::Invalid);
        };
        let Some(store_checkpoint_trust) = trust.store_checkpoint_trust else {
            return Err(PortableExportError::Invalid);
        };
        for fixation in self.run_fixations() {
            if !retained_release_trust.verify(&fixation.fixation, self.kind)
                || !store_checkpoint_trust.verify(
                    &fixation.fixation,
                    self.kind,
                    &self.closure_reference,
                )
            {
                return Err(PortableExportError::Invalid);
            }
        }
        let verified = verify_offline_recorded_history(
            RawRunHistory {
                run_id: self.run_id.clone(),
                batches: self.batches.clone(),
            },
            trust.program_verifier,
            trust.physical_binding_verifier,
        )
        .map_err(classify_fold_error)?;
        if portable_routes(&export_fact_routes(&verified).map_err(classify_fold_error)?)
            != self.fact_routes
        {
            return Err(PortableExportError::Invalid);
        }
        let mut verified_sources = BTreeMap::new();
        for prefix in &self.source_prefixes {
            let source = verify_offline_recorded_history(
                RawRunHistory {
                    run_id: prefix.run_id.clone(),
                    batches: prefix.batches.clone(),
                },
                trust.program_verifier,
                trust.physical_binding_verifier,
            )
            .map_err(classify_fold_error)?;
            if portable_routes(&export_fact_routes(&source).map_err(classify_fold_error)?)
                != prefix.fact_routes
            {
                return Err(PortableExportError::Invalid);
            }
            if source.run_id() != &prefix.run_id
                || source.admission().tenant_scope_id != self.tenant_scope_id
                || source.journal_head() != &prefix.fixation.journal_head
                || source.semantic_head() != &prefix.fixation.semantic_head
                || verified_sources
                    .insert(prefix.run_id.clone(), BTreeSet::new())
                    .is_some()
            {
                return Err(PortableExportError::Invalid);
            }
            let nested = source
                .direct_source_run_ids()
                .map_err(classify_fold_error)?;
            if nested
                .iter()
                .any(|run_id| run_id == &self.run_id || run_id == &prefix.run_id)
            {
                return Err(PortableExportError::Invalid);
            }
            verified_sources.insert(prefix.run_id.clone(), nested);
        }
        let root_sources = verified
            .direct_source_run_ids()
            .map_err(classify_fold_error)?;
        if root_sources
            .iter()
            .any(|run_id| !self.source_run_ids.contains(run_id))
        {
            return Err(PortableExportError::Invalid);
        }
        let mut source_graph = verified_sources.clone();
        source_graph.insert(self.run_id.clone(), root_sources.clone());
        let mut colors = BTreeMap::<RunId, u8>::new();
        for start in source_graph.keys().cloned().collect::<Vec<_>>() {
            if colors.get(&start).copied().unwrap_or_default() != 0 {
                continue;
            }
            let mut stack = vec![(start, false)];
            while let Some((run_id, exiting)) = stack.pop() {
                if exiting {
                    colors.insert(run_id, 2);
                    continue;
                }
                match colors.get(&run_id).copied().unwrap_or_default() {
                    1 => return Err(PortableExportError::Invalid),
                    2 => continue,
                    _ => {}
                }
                colors.insert(run_id.clone(), 1);
                stack.push((run_id.clone(), true));
                let children = source_graph
                    .get(&run_id)
                    .ok_or(PortableExportError::Invalid)?;
                for child in children.iter().rev() {
                    if !source_graph.contains_key(child) {
                        return Err(PortableExportError::Invalid);
                    }
                    stack.push((child.clone(), false));
                }
            }
        }
        let mut reachable = BTreeSet::new();
        let mut pending = root_sources;
        while let Some(run_id) = pending.pop_first() {
            if !reachable.insert(run_id.clone()) {
                continue;
            }
            let nested = verified_sources
                .get(&run_id)
                .ok_or(PortableExportError::Invalid)?;
            for child in nested {
                if child == &self.run_id || child == &run_id {
                    return Err(PortableExportError::Invalid);
                }
                pending.insert(child.clone());
            }
        }
        if verified.run_id() != &self.run_id
            || verified.admission().tenant_scope_id != self.tenant_scope_id
            || verified.journal_head() != &self.fixation.journal_head
            || verified.semantic_head() != &self.fixation.semantic_head
            || reachable != self.source_run_ids.iter().cloned().collect()
        {
            return Err(PortableExportError::Invalid);
        }
        let required_frontiers = self
            .batches
            .iter()
            .chain(
                self.source_prefixes
                    .iter()
                    .flat_map(|prefix| prefix.batches.iter()),
            )
            .filter_map(|batch| match &batch.tenant_fact_coordinate {
                TenantFactCoordinate::FactSelectionBarrier { frontier } => Some(frontier),
                TenantFactCoordinate::None | TenantFactCoordinate::FactPublication { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        for frontier in required_frontiers {
            let covered = self.source_prefixes.iter().any(|prefix| {
                prefix
                    .fact_frontiers
                    .iter()
                    .any(|published| published == frontier)
            });
            if !covered {
                return Err(PortableExportError::Invalid);
            }
        }
        Ok(recorded_evidence_from_verified(verified))
    }

    fn compute_closure_reference(&self) -> Result<ContentDigest, PortableExportError> {
        let run_fixations = self.run_fixations();
        let body = ClosureDigestBody {
            fixation: &self.fixation,
            kind: self.kind,
            root_run_id: &self.run_id,
            run_fixations: &run_fixations,
            source_run_ids: &self.source_run_ids,
            store_scope_id: &self.store_scope_id,
            tenant_scope_id: &self.tenant_scope_id,
            version: &self.version,
        };
        let canonical = canonical_json(&body).map_err(|_| PortableExportError::Invalid)?;
        Ok(raw_digest(canonical.as_bytes()))
    }

    fn validate_structure(&self) -> Result<(), PortableExportError> {
        let total_batches = self
            .batches
            .len()
            .checked_add(
                self.source_prefixes
                    .iter()
                    .try_fold(0usize, |count, prefix| {
                        count.checked_add(prefix.batches.len())
                    })
                    .ok_or(PortableExportError::TooLarge)?,
            )
            .ok_or(PortableExportError::TooLarge)?;
        if self.version != PORTABLE_EXPORT_VERSION
            || self.batches.is_empty()
            || total_batches > MAX_PORTABLE_BATCHES
            || total_batches.saturating_add(1) > MAX_PORTABLE_FRAMES
            || self.source_run_ids.len() > MAX_PORTABLE_SOURCE_RUNS
            || self.store_scope_id != self.fixation.store_scope_id
            || self.tenant_scope_id != self.fixation.tenant_scope_id
            || self.source_run_ids.binary_search(&self.run_id).is_ok()
            || self.source_prefixes.len() != self.source_run_ids.len()
            || self
                .source_prefixes
                .iter()
                .map(|prefix| &prefix.run_id)
                .collect::<Vec<_>>()
                != self.source_run_ids.iter().collect::<Vec<_>>()
            || self.compute_closure_reference()? != self.closure_reference
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
        if matches!(self.kind, ExportKind::Semantic) {
            let semantic_sequence = match &self.fixation.semantic_head {
                SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
                SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
            };
            if last.head.run_sequence != semantic_sequence {
                return Err(PortableExportError::Invalid);
            }
        }
        if self
            .source_run_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(PortableExportError::Invalid);
        }
        for prefix in &self.source_prefixes {
            validate_prefix(
                prefix,
                &self.store_scope_id,
                &self.tenant_scope_id,
                self.fixation.store_epoch,
                &self.fixation.physical_target,
                matches!(self.kind, ExportKind::Semantic),
            )?;
        }
        validate_fact_routes(self)?;
        if !dense_bundled_publication_frontiers(self) {
            return Err(PortableExportError::Invalid);
        }
        validate_required_source_heads(self)?;
        Ok(())
    }

    fn run_fixations(&self) -> Vec<PortableRunFixation> {
        let mut fixations = Vec::with_capacity(self.source_prefixes.len() + 1);
        fixations.push(PortableRunFixation {
            fact_frontiers: fact_frontiers(&self.batches),
            fact_routes: self.fact_routes.clone(),
            fixation: self.fixation.clone(),
            run_id: self.run_id.clone(),
            tenant_scope_id: self.tenant_scope_id.clone(),
        });
        fixations.extend(
            self.source_prefixes
                .iter()
                .map(|prefix| PortableRunFixation {
                    fact_frontiers: prefix.fact_frontiers.clone(),
                    fact_routes: prefix.fact_routes.clone(),
                    fixation: prefix.fixation.clone(),
                    run_id: prefix.run_id.clone(),
                    tenant_scope_id: prefix.tenant_scope_id.clone(),
                }),
        );
        fixations
    }
}

fn validate_fact_routes(export: &PortableRunExport) -> Result<(), PortableExportError> {
    let mut batches_by_run = BTreeMap::<RunId, &[CommittedBatch]>::new();
    batches_by_run.insert(export.run_id.clone(), &export.batches);
    for prefix in &export.source_prefixes {
        if batches_by_run
            .insert(prefix.run_id.clone(), &prefix.batches)
            .is_some()
        {
            return Err(PortableExportError::Invalid);
        }
    }
    let route_count = export
        .fact_routes
        .len()
        .checked_add(
            export
                .source_prefixes
                .iter()
                .try_fold(0usize, |count, prefix| {
                    count.checked_add(prefix.fact_routes.len())
                })
                .ok_or(PortableExportError::TooLarge)?,
        )
        .ok_or(PortableExportError::TooLarge)?;
    if route_count > MAX_PORTABLE_FACT_ROUTES {
        return Err(PortableExportError::TooLarge);
    }
    let check = |consumer_run_id: &RunId,
                 routes: &[PortableFactRoute]|
     -> Result<(), PortableExportError> {
        let consumer_batches = batches_by_run
            .get(consumer_run_id)
            .ok_or(PortableExportError::Invalid)?;
        for route in routes {
            if route.consumer_record.run_id != *consumer_run_id
                || route.publication_frontier.store_scope_id != export.store_scope_id
                || route.publication_frontier.store_epoch != export.fixation.store_epoch
                || route.publication_frontier.tenant_scope_id != export.tenant_scope_id
            {
                return Err(PortableExportError::Invalid);
            }
            let consumer_present = consumer_batches.iter().any(|batch| {
                batch
                    .records
                    .iter()
                    .any(|assigned| assigned.record_ref == route.consumer_record)
            });
            let producer_batches = batches_by_run
                .get(&route.producer_transition.run_id)
                .ok_or(PortableExportError::Invalid)?;
            let producer_present = producer_batches.iter().any(|batch| {
                batch.tenant_fact_coordinate
                    == TenantFactCoordinate::FactPublication {
                        frontier: route.publication_frontier.clone(),
                    }
                    && batch
                        .records
                        .iter()
                        .any(|assigned| assigned.record_ref == route.producer_transition)
            });
            if !consumer_present || !producer_present {
                return Err(PortableExportError::Invalid);
            }
        }
        Ok(())
    };
    check(&export.run_id, &export.fact_routes)?;
    for prefix in &export.source_prefixes {
        check(&prefix.run_id, &prefix.fact_routes)?;
    }
    Ok(())
}

fn validate_required_source_heads(export: &PortableRunExport) -> Result<(), PortableExportError> {
    let mut required = BTreeMap::<RunId, u64>::new();
    let mut record_routes = |routes: &[PortableFactRoute]| {
        for route in routes {
            if route.consumer_record.run_id == route.producer_transition.run_id {
                continue;
            }
            required
                .entry(route.producer_transition.run_id.clone())
                .and_modify(|sequence| {
                    *sequence = (*sequence).max(route.producer_transition.run_sequence)
                })
                .or_insert(route.producer_transition.run_sequence);
        }
    };
    record_routes(&export.fact_routes);
    for prefix in &export.source_prefixes {
        record_routes(&prefix.fact_routes);
    }
    let actual = export
        .source_prefixes
        .iter()
        .map(|prefix| {
            (
                prefix.run_id.clone(),
                prefix.fixation.journal_head.run_sequence,
            )
        })
        .collect::<BTreeMap<_, _>>();
    if required != actual {
        return Err(PortableExportError::Invalid);
    }
    Ok(())
}

/// Explicit trust material for offline portable verification.
pub trait RetainedPhysicalReleaseTrust:
    mfm_authority_seal::RetainedPhysicalReleaseTrustSeal + Send + Sync
{
    /// Verifies the retained public release history for one fixation.
    fn verify(&self, fixation: &PortableFixation, kind: ExportKind) -> bool;
}

/// Explicit external store/checkpoint trust for one portable closure.
pub trait StoreCheckpointTrust: mfm_authority_seal::StoreCheckpointTrustSeal + Send + Sync {
    /// Verifies target lineage, writer epoch, and exact authorized closure.
    fn verify(
        &self,
        fixation: &PortableFixation,
        kind: ExportKind,
        closure_reference: &ContentDigest,
    ) -> bool;
}

/// Callback-free trust inputs captured when an export closure is authorized.
pub struct ReplayTrustSnapshot<'a> {
    /// Deterministic program verifier.
    ///
    /// The verifier is retained by reference only for the duration of one fold.
    program_verifier: &'a dyn ProgramVerifier,
    /// Public physical-binding verifier.
    physical_binding_verifier: &'a dyn PublicPhysicalBindingVerifier,
    /// Optional retained-release lineage verifier.
    retained_release_trust: Option<&'a dyn RetainedPhysicalReleaseTrust>,
    /// Optional external store/checkpoint verifier.
    store_checkpoint_trust: Option<&'a dyn StoreCheckpointTrust>,
    /// Exact authorized closure reference, when one was captured.
    authorized_closure_reference: Option<&'a ContentDigest>,
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
            retained_release_trust: None,
            store_checkpoint_trust: None,
            authorized_closure_reference: None,
        }
    }

    /// Adds the exact closure reference and deployment trust required for
    /// isolated verification.
    pub fn with_authorized_closure(
        mut self,
        closure_reference: &'a ContentDigest,
        retained_release_trust: &'a dyn RetainedPhysicalReleaseTrust,
        store_checkpoint_trust: &'a dyn StoreCheckpointTrust,
    ) -> Self {
        self.authorized_closure_reference = Some(closure_reference);
        self.retained_release_trust = Some(retained_release_trust);
        self.store_checkpoint_trust = Some(store_checkpoint_trust);
        self
    }
}

/// Stable portable-export failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortableExportError {
    /// The stream is malformed, non-canonical, or fails offline fold verification.
    #[error("portable export is invalid")]
    Invalid,
    /// The stream exceeds a frozen byte, frame, count, or depth bound.
    #[error("portable export exceeds frozen bounds")]
    TooLarge,
}

impl From<PortableExportError> for StructuredReplayError {
    fn from(error: PortableExportError) -> Self {
        match error {
            PortableExportError::Invalid | PortableExportError::TooLarge => {
                StructuredReplayError::InvalidRecordedHistory
            }
        }
    }
}

fn select_batches(
    evidence: &ExportEncoderView<'_>,
    kind: ExportKind,
) -> Result<Vec<CommittedBatch>, PortableExportError> {
    let batches = evidence
        .batch_frames()
        .map(|frame| {
            serde_json::from_slice::<CommittedBatch>(frame)
                .map_err(|_| PortableExportError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
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
                .take_while(|batch| batch.head.run_sequence <= cutoff)
                .cloned()
                .collect::<Vec<_>>();
            if selected
                .last()
                .is_none_or(|batch| batch.head.run_sequence != cutoff)
            {
                return Err(PortableExportError::Invalid);
            }
            Ok(selected)
        }
    }
}

fn portable_prefix_from_source(
    source: mfm_store::structured::ExportEncoderSource<'_>,
    required_head: Option<u64>,
    semantic_cutoff_required: bool,
) -> Result<PortableRunPrefix, PortableExportError> {
    // A source is retained only through the largest transition actually routed
    // by the selected closure. This applies to audit exports as well: carrying
    // an unrelated later source append would violate the exact required-head
    // rule used to prove the recursive dependency closure.
    let semantic_cutoff = match source.semantic_head() {
        SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
        SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
    };
    let cutoff = required_head.unwrap_or(semantic_cutoff);
    let batches = source
        .batch_frames()
        .map(|frame| {
            serde_json::from_slice::<CommittedBatch>(frame)
                .map_err(|_| PortableExportError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .take_while(|batch| batch.head.run_sequence <= cutoff)
        .collect::<Vec<_>>();
    if batches.is_empty() {
        return Err(PortableExportError::Invalid);
    }
    let first = batches.first().ok_or(PortableExportError::Invalid)?;
    let semantic_head = semantic_head_at_cutoff(source.semantic_head(), &batches)?;
    let fixation = PortableFixation {
        semantic_head,
        journal_head: batches
            .last()
            .ok_or(PortableExportError::Invalid)?
            .head
            .clone(),
        store_scope_id: first.store_scope_id.clone(),
        store_epoch: first.store_epoch,
        tenant_scope_id: source.header().tenant_scope_id().clone(),
        physical_target: source.physical_target().clone(),
    };
    let cutoff = batches
        .last()
        .map(|batch| batch.head.run_sequence)
        .ok_or(PortableExportError::Invalid)?;
    let fact_routes = source
        .fact_routes()
        .iter()
        .filter(|route| route.consumer_record().run_sequence <= cutoff)
        .map(portable_fact_route)
        .collect::<Vec<_>>();
    let prefix_frontiers = fact_frontiers(&batches);
    let prefix = PortableRunPrefix {
        run_id: source.run_id().clone(),
        tenant_scope_id: source.header().tenant_scope_id().clone(),
        fixation,
        batches,
        fact_frontiers: prefix_frontiers,
        fact_routes,
    };
    validate_prefix(
        &prefix,
        &prefix.fixation.store_scope_id,
        &prefix.tenant_scope_id,
        prefix.fixation.store_epoch,
        &prefix.fixation.physical_target,
        semantic_cutoff_required,
    )?;
    Ok(prefix)
}

fn portable_fact_route(route: &mfm_store::structured::ExportFactRoute) -> PortableFactRoute {
    PortableFactRoute {
        consumer_record: route.consumer_record().clone(),
        producer_transition: route.producer_transition().clone(),
        publication_frontier: route.publication_frontier().clone(),
    }
}

fn portable_routes(routes: &[mfm_store::structured::ExportFactRoute]) -> Vec<PortableFactRoute> {
    routes.iter().map(portable_fact_route).collect()
}

fn semantic_head_at_cutoff(
    original: &SemanticHead,
    batches: &[CommittedBatch],
) -> Result<SemanticHead, PortableExportError> {
    let original_cutoff = match original {
        SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
        SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
    };
    let last_sequence = batches
        .last()
        .map(|batch| batch.head.run_sequence)
        .ok_or(PortableExportError::Invalid)?;
    if last_sequence >= original_cutoff {
        return Ok(original.clone());
    }
    let mut transition = None;
    for batch in batches {
        for assigned in &batch.records {
            if let mfm_journal::structured::RunRecord::StateTransitionCommitted(value) =
                &assigned.record
            {
                transition = Some(SemanticHead::Transition {
                    transition_ref: assigned.record_ref.clone(),
                    semantic_state_digest: value.after_semantic_state_digest.clone(),
                });
            }
        }
    }
    transition.ok_or(PortableExportError::Invalid)
}

fn fact_frontiers(batches: &[CommittedBatch]) -> Vec<TenantFactFrontier> {
    let mut frontiers = batches
        .iter()
        .filter_map(|batch| match &batch.tenant_fact_coordinate {
            TenantFactCoordinate::FactPublication { frontier }
            | TenantFactCoordinate::FactSelectionBarrier { frontier } => Some(frontier.clone()),
            TenantFactCoordinate::None => None,
        })
        .collect::<Vec<_>>();
    frontiers.sort();
    frontiers.dedup();
    frontiers
}

fn dense_bundled_publication_frontiers(export: &PortableRunExport) -> bool {
    let mut orders = BTreeMap::<(&StoreScopeId, StoreEpoch, &TenantScopeId), Vec<u64>>::new();
    let batch_sets = std::iter::once(export.batches.as_slice()).chain(
        export
            .source_prefixes
            .iter()
            .map(|prefix| prefix.batches.as_slice()),
    );
    for batch_set in batch_sets {
        for batch in batch_set {
            let TenantFactCoordinate::FactPublication { frontier } = &batch.tenant_fact_coordinate
            else {
                continue;
            };
            orders
                .entry((
                    &frontier.store_scope_id,
                    frontier.store_epoch,
                    &frontier.tenant_scope_id,
                ))
                .or_default()
                .push(frontier.fact_order);
        }
    }
    orders.values_mut().all(|orders| {
        orders.sort_unstable();
        orders.dedup();
        orders
            .iter()
            .enumerate()
            .all(|(index, order)| *order == u64::try_from(index + 1).unwrap_or_default())
    })
}

fn validate_prefix(
    prefix: &PortableRunPrefix,
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    store_epoch: StoreEpoch,
    physical_target: &PhysicalTargetIdentity,
    semantic_cutoff: bool,
) -> Result<(), PortableExportError> {
    if prefix.batches.is_empty()
        || &prefix.fixation.store_scope_id != store_scope_id
        || prefix.fixation.store_epoch != store_epoch
        || &prefix.fixation.physical_target != physical_target
        || &prefix.fixation.tenant_scope_id != tenant_scope_id
        || &prefix.tenant_scope_id != tenant_scope_id
        || prefix.batches.last().map(|batch| &batch.head) != Some(&prefix.fixation.journal_head)
        || prefix.fact_frontiers != fact_frontiers(&prefix.batches)
    {
        return Err(PortableExportError::Invalid);
    }
    if prefix
        .fact_frontiers
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    // A consumer may carry only a selection barrier at frontier N; the
    // producer publication route, not the consumer run's own batches, is
    // responsible for proving density through N.
    {
        return Err(PortableExportError::Invalid);
    }
    let mut previous = None;
    let mut objects = 0usize;
    for batch in &prefix.batches {
        if batch.store_scope_id != prefix.fixation.store_scope_id
            || batch.store_epoch != prefix.fixation.store_epoch
            || batch.predecessor != previous
            || batch.records.is_empty()
        {
            return Err(PortableExportError::Invalid);
        }
        objects = objects
            .checked_add(batch.objects.len())
            .ok_or(PortableExportError::TooLarge)?;
        if objects > MAX_PORTABLE_OBJECTS {
            return Err(PortableExportError::TooLarge);
        }
        previous = Some(batch.head.clone());
    }
    if semantic_cutoff {
        let cutoff = match &prefix.fixation.semantic_head {
            SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
            SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
        };
        if prefix
            .batches
            .last()
            .is_none_or(|batch| batch.head.run_sequence != cutoff)
        {
            return Err(PortableExportError::Invalid);
        }
    }
    Ok(())
}

fn encode_frames(export: &PortableRunExport) -> Result<Vec<u8>, PortableExportError> {
    let mut stream = Vec::new();
    let mut previous = None;
    let mut chain = genesis_chain_digest();
    let mut ordinal = 0_u64;
    // The root is emitted first, followed by source prefixes in run-id order.
    let prefixes = std::iter::once((&export.run_id, &export.batches)).chain(
        export
            .source_prefixes
            .iter()
            .map(|prefix| (&prefix.run_id, &prefix.batches)),
    );
    for (run_id, batches) in prefixes {
        for batch in batches {
            let payload = serde_json::to_value(PortableBatchPayload {
                batch: batch.clone(),
                run_id: run_id.clone(),
            })
            .map_err(|_| PortableExportError::Invalid)?;
            let frame = PortableFrame {
                kind: PortableFrameKind::Batch,
                ordinal,
                payload,
                previous_frame_digest: previous.clone(),
            };
            let encoded = encode_frame(&frame)?;
            let digest = raw_digest(&encoded[..encoded.len() - 1]);
            chain = chain_step(&chain, &digest);
            let next_len = stream
                .len()
                .checked_add(encoded.len())
                .ok_or(PortableExportError::TooLarge)?;
            if u64::try_from(next_len).map_err(|_| PortableExportError::TooLarge)?
                > MAX_PORTABLE_EXPORT_BYTES
            {
                return Err(PortableExportError::TooLarge);
            }
            stream.extend_from_slice(&encoded);
            previous = Some(digest);
            ordinal = ordinal.saturating_add(1);
        }
    }

    let frame_chain_digest = chain.clone();
    let final_frame_digest = previous.ok_or(PortableExportError::Invalid)?;
    let total_frames = ordinal.saturating_add(1);
    let mut total_bytes = 0_u64;
    let mut seal_bytes = Vec::new();
    for _ in 0..8 {
        let seal = PortableSeal {
            closure_reference: export.closure_reference.clone(),
            final_frame_digest: final_frame_digest.clone(),
            fixation: export.fixation.clone(),
            frame_chain_digest: frame_chain_digest.clone(),
            kind: export.kind,
            root_run_id: export.run_id.clone(),
            run_fixations: export.run_fixations(),
            source_run_ids: export.source_run_ids.clone(),
            store_scope_id: export.store_scope_id.clone(),
            tenant_scope_id: export.tenant_scope_id.clone(),
            total_bytes,
            total_frames,
            version: export.version.clone(),
        };
        let payload = serde_json::to_value(seal).map_err(|_| PortableExportError::Invalid)?;
        let frame = PortableFrame {
            kind: PortableFrameKind::Seal,
            ordinal: total_frames - 1,
            payload,
            previous_frame_digest: Some(final_frame_digest.clone()),
        };
        seal_bytes = encode_frame(&frame)?;
        let next_total = u64::try_from(stream.len().saturating_add(seal_bytes.len()))
            .map_err(|_| PortableExportError::TooLarge)?;
        if next_total > MAX_PORTABLE_EXPORT_BYTES {
            return Err(PortableExportError::TooLarge);
        }
        if next_total == total_bytes {
            break;
        }
        total_bytes = next_total;
    }
    if total_bytes == 0 {
        total_bytes = u64::try_from(stream.len().saturating_add(seal_bytes.len()))
            .map_err(|_| PortableExportError::TooLarge)?;
        let seal = PortableSeal {
            closure_reference: export.closure_reference.clone(),
            final_frame_digest: final_frame_digest.clone(),
            fixation: export.fixation.clone(),
            frame_chain_digest,
            kind: export.kind,
            root_run_id: export.run_id.clone(),
            run_fixations: export.run_fixations(),
            source_run_ids: export.source_run_ids.clone(),
            store_scope_id: export.store_scope_id.clone(),
            tenant_scope_id: export.tenant_scope_id.clone(),
            total_bytes,
            total_frames,
            version: export.version.clone(),
        };
        let payload = serde_json::to_value(seal).map_err(|_| PortableExportError::Invalid)?;
        seal_bytes = encode_frame(&PortableFrame {
            kind: PortableFrameKind::Seal,
            ordinal: total_frames - 1,
            payload,
            previous_frame_digest: Some(final_frame_digest),
        })?;
    }
    let final_len = stream
        .len()
        .checked_add(seal_bytes.len())
        .ok_or(PortableExportError::TooLarge)?;
    if u64::try_from(final_len).map_err(|_| PortableExportError::TooLarge)?
        > MAX_PORTABLE_EXPORT_BYTES
        || final_len as u64 != total_bytes
    {
        return Err(PortableExportError::TooLarge);
    }
    stream.extend_from_slice(&seal_bytes);
    Ok(stream)
}

fn decode_frames(bytes: &[u8]) -> Result<PortableRunExport, PortableExportError> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_PORTABLE_EXPORT_BYTES {
        return Err(PortableExportError::TooLarge);
    }
    if !bytes.ends_with(b"\n") {
        return Err(PortableExportError::Invalid);
    }
    let contract = RecoverabilityContract::embedded().map_err(|_| PortableExportError::Invalid)?;
    let mut batches_by_run: BTreeMap<RunId, Vec<CommittedBatch>> = BTreeMap::new();
    let mut previous = None;
    let mut chain = genesis_chain_digest();
    let mut seal = None;
    let mut frame_count = 0usize;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty())
    {
        if line.len().saturating_add(1) > MAX_PORTABLE_FRAME_BYTES {
            return Err(PortableExportError::TooLarge);
        }
        if line.contains(&b'\r') {
            return Err(PortableExportError::Invalid);
        }
        if frame_count >= MAX_PORTABLE_FRAMES {
            return Err(PortableExportError::TooLarge);
        }
        let validated = contract
            .strict_decode(PORTABLE_FRAME_SCHEMA_CONTRACT, line)
            .map_err(|_| PortableExportError::Invalid)?;
        let frame: PortableFrame = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| PortableExportError::Invalid)?;
        let expected_ordinal =
            u64::try_from(frame_count).map_err(|_| PortableExportError::TooLarge)?;
        if frame.ordinal != expected_ordinal || frame.previous_frame_digest != previous {
            return Err(PortableExportError::Invalid);
        }
        let digest = raw_digest(line);
        match frame.kind {
            PortableFrameKind::Batch => {
                if seal.is_some() {
                    return Err(PortableExportError::Invalid);
                }
                let payload: PortableBatchPayload = serde_json::from_value(frame.payload)
                    .map_err(|_| PortableExportError::Invalid)?;
                let batches = batches_by_run.entry(payload.run_id).or_default();
                batches.push(payload.batch);
            }
            PortableFrameKind::Seal => {
                if seal.is_some() || frame_count == 0 {
                    return Err(PortableExportError::Invalid);
                }
                seal = Some(
                    serde_json::from_value::<PortableSeal>(frame.payload)
                        .map_err(|_| PortableExportError::Invalid)?,
                );
            }
        }
        chain = chain_step(&chain, &digest);
        previous = Some(digest);
        frame_count = frame_count.saturating_add(1);
    }
    let seal = seal.ok_or(PortableExportError::Invalid)?;
    if frame_count != bytes.iter().filter(|byte| **byte == b'\n').count()
        || seal.total_frames != frame_count as u64
        || seal.total_bytes != bytes.len() as u64
        || seal.final_frame_digest
            != previous
                .as_ref()
                .and_then(|_| {
                    if frame_count >= 2 {
                        let mut lines = bytes.split(|byte| *byte == b'\n');
                        let last_batch = lines.nth(frame_count - 2)?;
                        Some(raw_digest(last_batch))
                    } else {
                        None
                    }
                })
                .ok_or(PortableExportError::Invalid)?
    {
        return Err(PortableExportError::Invalid);
    }
    let mut prior_chain = genesis_chain_digest();
    let mut lines = bytes
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty());
    for _ in 0..frame_count.saturating_sub(1) {
        let line = lines.next().ok_or(PortableExportError::Invalid)?;
        prior_chain = chain_step(&prior_chain, &raw_digest(line));
    }
    if seal.frame_chain_digest != prior_chain {
        return Err(PortableExportError::Invalid);
    }
    let root_batches = batches_by_run
        .remove(&seal.root_run_id)
        .ok_or(PortableExportError::Invalid)?;
    let seal_run_fixations = seal.run_fixations.clone();
    let mut source_prefixes = Vec::new();
    for fixation in seal.run_fixations.iter().skip(1) {
        let batches = batches_by_run
            .remove(&fixation.run_id)
            .ok_or(PortableExportError::Invalid)?;
        source_prefixes.push(PortableRunPrefix {
            run_id: fixation.run_id.clone(),
            tenant_scope_id: fixation.tenant_scope_id.clone(),
            fixation: fixation.fixation.clone(),
            batches,
            fact_frontiers: fixation.fact_frontiers.clone(),
            fact_routes: fixation.fact_routes.clone(),
        });
    }
    if !batches_by_run.is_empty() || seal.run_fixations.is_empty() {
        return Err(PortableExportError::Invalid);
    }
    let export = PortableRunExport {
        version: seal.version,
        kind: seal.kind,
        store_scope_id: seal.store_scope_id,
        tenant_scope_id: seal.tenant_scope_id,
        run_id: seal.root_run_id,
        fixation: seal.fixation,
        source_run_ids: seal.source_run_ids,
        source_prefixes,
        batches: root_batches,
        fact_routes: seal_run_fixations
            .first()
            .ok_or(PortableExportError::Invalid)?
            .fact_routes
            .clone(),
        closure_reference: seal.closure_reference,
    };
    if export.compute_closure_reference()? != export.closure_reference {
        return Err(PortableExportError::Invalid);
    }
    if seal_run_fixations != export.run_fixations() {
        return Err(PortableExportError::Invalid);
    }
    export.validate_structure()?;
    if encode_frames(&export)?.as_slice() != bytes {
        return Err(PortableExportError::Invalid);
    }
    Ok(export)
}

fn encode_frame(frame: &PortableFrame) -> Result<Vec<u8>, PortableExportError> {
    let canonical = canonical_json(frame).map_err(|_| PortableExportError::Invalid)?;
    if canonical.as_bytes().len().saturating_add(1) > MAX_PORTABLE_FRAME_BYTES {
        return Err(PortableExportError::TooLarge);
    }
    let mut bytes = canonical.as_bytes().to_vec();
    bytes.push(b'\n');
    Ok(bytes)
}

fn raw_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(bytes))
}

fn placeholder_digest() -> ContentDigest {
    raw_digest(b"mfm.portable-export.digest-placeholder")
}

fn genesis_chain_digest() -> ContentDigest {
    raw_digest(b"mfm.portable-export.frame-chain-genesis")
}

fn chain_step(previous: &ContentDigest, frame: &ContentDigest) -> ContentDigest {
    let mut bytes = Vec::with_capacity(previous.as_str().len() + frame.as_str().len());
    bytes.extend_from_slice(previous.as_str().as_bytes());
    bytes.extend_from_slice(frame.as_str().as_bytes());
    raw_digest(&bytes)
}

fn classify_fold_error(error: StructuredStoreError) -> PortableExportError {
    match error {
        StructuredStoreError::RunNotFound
        | StructuredStoreError::InvalidHistory
        | StructuredStoreError::CandidateRejected
        | StructuredStoreError::Certification
        | StructuredStoreError::StaleHead
        | StructuredStoreError::AppendConflict
        | StructuredStoreError::BackendUnavailable
        | StructuredStoreError::AcknowledgementUnknown => PortableExportError::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::{PortableExportError, PortableRunExport, MAX_PORTABLE_EXPORT_BYTES};

    #[test]
    fn oversized_and_monolithic_documents_fail_before_full_decode() {
        let oversize = vec![b'{'; (MAX_PORTABLE_EXPORT_BYTES as usize) + 1];
        assert_eq!(
            PortableRunExport::strict_decode(&oversize),
            Err(PortableExportError::TooLarge)
        );
        assert_eq!(json_depth(br#"{\"kind\":\"batch\"}"#), Some(1));
        assert!(PortableRunExport::strict_decode(b"").is_err());
        assert!(PortableRunExport::strict_decode(br#"{\"kind\":\"semantic\"}"#).is_err());
        assert!(PortableRunExport::strict_decode(
            br#"{\"version\":\"mfm.structured-portable-run-export.v1\"}"#
        )
        .is_err());
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
}
