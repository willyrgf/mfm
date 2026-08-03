//! Sealed purpose-specific run history readers and evidence.
//!
//! Each purpose reader loads through the same callback-free fold, then wraps the
//! verified prefix in a purpose-sealed evidence newtype. Evidence types have no
//! public field, no `Deref`, and expose only the accessors that purpose needs.
//! Cross-purpose substitution is a type error: `PublicRunEvidence` cannot be
//! passed where `ExportRunEvidence` is required.

use std::collections::BTreeSet;

use mfm_facts::FactSelectionReadResponse;
use mfm_ids::{ContentRef, RunId};
use mfm_journal::structured::{
    AssignedRecord, HistoryObject, JournalHead, ObservationOutcome, PriorRunFactSelectionResponse,
    RunAdmitted, RunRecord, SemanticHead,
};

use super::backend::{StructuredHistoryBackend, StructuredRunHistoryReader};
use super::fold::{StructuredFrontier, VerifiedStructuredRun};
use super::Result;

/// Maximum distinct prior-run sources one portable export may traverse.
pub const MAX_EXPORT_SOURCE_RUNS: usize = 4_096;

macro_rules! purpose_reader_shell {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        pub struct $name<B: StructuredHistoryBackend> {
            reader: StructuredRunHistoryReader<B>,
        }

        impl<B: StructuredHistoryBackend> $name<B> {
            pub(super) fn new(reader: StructuredRunHistoryReader<B>) -> Self {
                Self { reader }
            }

            /// Returns the immutable qualified store identity.
            pub fn store_identity(&self) -> &super::StructuredStoreIdentity {
                self.reader.store_identity()
            }

            /// Probes backend readability without requiring an existing application run.
            pub async fn check_ready(&self) -> Result<()> {
                self.reader.check_ready().await
            }
        }
    };
}

purpose_reader_shell!(
    /// Target-bound public-read projection authority.
    PublicRunReader
);
purpose_reader_shell!(
    /// Target-bound transition-trace projection authority.
    TraceRunReader
);
purpose_reader_shell!(
    /// Target-bound access-audit projection authority.
    AuditRunReader
);
purpose_reader_shell!(
    /// Target-bound recorded-replay projection authority.
    ReplayRunReader
);
purpose_reader_shell!(
    /// Target-bound export projection authority.
    ExportRunReader
);

impl<B: StructuredHistoryBackend> PublicRunReader<B> {
    /// Loads one verified run as public-read evidence only.
    pub async fn load_public(&self, run_id: &RunId) -> Result<PublicRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(PublicRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> TraceRunReader<B> {
    /// Loads one verified run as transition-trace evidence only.
    pub async fn load_transition_trace(&self, run_id: &RunId) -> Result<TraceRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(TraceRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> AuditRunReader<B> {
    /// Loads one verified run as access-audit evidence only.
    pub async fn load_access_audit(&self, run_id: &RunId) -> Result<AuditRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(AuditRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> ReplayRunReader<B> {
    /// Loads one verified run as recorded-replay evidence only.
    pub async fn load_for_recorded_verify(&self, run_id: &RunId) -> Result<RecordedRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(RecordedRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> ExportRunReader<B> {
    /// Loads one verified run as portable-export evidence only.
    pub async fn load_for_export(&self, run_id: &RunId) -> Result<ExportRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(ExportRunEvidence::from_verified)
    }
}

/// Sealed public-read evidence. Cannot be used as export, trace, audit, or replay evidence.
#[derive(Debug)]
pub struct PublicRunEvidence(VerifiedStructuredRun);

impl PublicRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        self.0.semantic_head()
    }

    /// Returns the closed action frontier.
    pub const fn frontier(&self) -> &StructuredFrontier {
        self.0.frontier()
    }

    /// Returns the terminal nominal operation-outcome reference, when closed.
    pub const fn closed_outcome_ref(&self) -> Option<&ContentRef> {
        self.0.closed_outcome_ref()
    }

    /// Resolves one exact verified content-addressed history object.
    pub fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.0.object(content_ref)
    }
}

/// Sealed transition-trace evidence. Cannot be used as public, export, audit, or replay evidence.
#[derive(Debug)]
pub struct TraceRunEvidence(VerifiedStructuredRun);

impl TraceRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        self.0.journal_heads()
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        self.0.records()
    }
}

/// Sealed access-audit evidence. Cannot be used as public, export, trace, or replay evidence.
#[derive(Debug)]
pub struct AuditRunEvidence(VerifiedStructuredRun);

impl AuditRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        self.0.journal_heads()
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        self.0.records()
    }
}

/// Sealed recorded-replay evidence. Cannot be used as public, export, trace, or audit evidence.
#[derive(Debug)]
pub struct RecordedRunEvidence(VerifiedStructuredRun);

impl RecordedRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Wraps one offline-folded verified run as recorded-replay evidence.
    pub(crate) fn from_offline_verified(verified: VerifiedStructuredRun) -> Self {
        Self::from_verified(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        self.0.semantic_head()
    }

    /// Returns the closed action frontier.
    pub const fn frontier(&self) -> &StructuredFrontier {
        self.0.frontier()
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        self.0.records()
    }

    /// Returns every exact committed-batch envelope in append order.
    pub fn batches(&self) -> &[mfm_journal::structured::CommittedBatch] {
        self.0.batches()
    }
}

/// Sealed portable-export evidence. Cannot be used as public, trace, audit, or replay evidence.
#[derive(Debug)]
pub struct ExportRunEvidence(VerifiedStructuredRun);

impl ExportRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        self.0.semantic_head()
    }

    /// Returns the closed action frontier.
    pub const fn frontier(&self) -> &StructuredFrontier {
        self.0.frontier()
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        self.0.journal_heads()
    }

    /// Returns every exact committed-batch envelope in append order.
    pub fn batches(&self) -> &[mfm_journal::structured::CommittedBatch] {
        self.0.batches()
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        self.0.records()
    }

    /// Returns the verified record prefix through the exact semantic head record.
    pub fn semantic_records(&self) -> impl Iterator<Item = &AssignedRecord> {
        self.0.semantic_records()
    }

    /// Returns verified objects first admitted no later than one physical append sequence.
    pub fn objects_through(&self, run_sequence: u64) -> impl Iterator<Item = &HistoryObject> {
        self.0.objects_through(run_sequence)
    }

    /// Resolves one exact verified content-addressed history object.
    pub fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.0.object(content_ref)
    }

    /// Returns the terminal nominal operation-outcome reference, when closed.
    pub const fn closed_outcome_ref(&self) -> Option<&ContentRef> {
        self.0.closed_outcome_ref()
    }

    /// Returns the sole callback-free cursor.
    pub fn cursor(&self) -> &super::fold::ProgramCursor {
        self.0.cursor()
    }

    /// Returns every distinct prior-run producer referenced by retained fact selections.
    ///
    /// Discovery uses only identifier-bearing fact-selection metadata. It does not
    /// load those source runs and does not expose their object content for
    /// serialization. Callers must authorize each returned identity before any
    /// export byte is emitted.
    pub fn direct_source_run_ids(&self) -> Result<BTreeSet<RunId>> {
        let mut sources = BTreeSet::new();
        let consumer = self.run_id();
        for assigned in self.records() {
            let RunRecord::ExternalAccessObserved(observation) = &assigned.record else {
                continue;
            };
            let ObservationOutcome::Returned { value } = &observation.outcome else {
                continue;
            };
            let Some(object) = self.object(&value.value_ref) else {
                continue;
            };
            let Ok(returned) = object.decode::<FactSelectionReadResponse>() else {
                continue;
            };
            let Ok(response) = serde_json::from_str::<PriorRunFactSelectionResponse>(
                returned.canonical_response_json(),
            ) else {
                continue;
            };
            for query in &response.query_results {
                for selected in &query.selected {
                    let producer = &selected.producer_transition_ref.run_id;
                    if producer != consumer {
                        sources.insert(producer.clone());
                    }
                }
            }
        }
        if sources.len() > MAX_EXPORT_SOURCE_RUNS {
            return Err(super::fold::StructuredStoreError::InvalidHistory);
        }
        Ok(sources)
    }
}

/// Expands the bounded recursive export source graph from one already-loaded root.
///
/// `load_sources` must return only identifier-level direct dependencies for one
/// already-authorized run. Shared DAGs are accepted once; cycles and over-budget
/// closures fail closed. The returned set excludes the root and is ordered by
/// `RunId` so encounter order cannot change authorization or serialization
/// requirements.
pub fn expand_export_source_closure<E, F>(
    root_run_id: &RunId,
    root_sources: BTreeSet<RunId>,
    mut load_sources: F,
) -> std::result::Result<BTreeSet<RunId>, E>
where
    F: FnMut(&RunId) -> std::result::Result<BTreeSet<RunId>, E>,
    E: From<ExportSourceClosureError>,
{
    let mut authorized = BTreeSet::new();
    let mut pending = root_sources;
    let mut edges: std::collections::BTreeMap<RunId, BTreeSet<RunId>> =
        std::collections::BTreeMap::from([(root_run_id.clone(), pending.clone())]);

    while let Some(run_id) = pending.pop_first() {
        if run_id == *root_run_id || !authorized.insert(run_id.clone()) {
            continue;
        }
        if authorized.len() > MAX_EXPORT_SOURCE_RUNS {
            return Err(ExportSourceClosureError::OverBudget.into());
        }
        let sources = load_sources(&run_id)?;
        if sources.len() > MAX_EXPORT_SOURCE_RUNS {
            return Err(ExportSourceClosureError::OverBudget.into());
        }
        edges.insert(run_id.clone(), sources.clone());
        for source in sources {
            if source != *root_run_id && !authorized.contains(&source) {
                pending.insert(source);
            }
        }
    }

    reject_export_source_cycles(&edges)?;
    Ok(authorized)
}

/// Stable failure while expanding an export source closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSourceClosureError {
    /// The graph exceeds the fixed source-run budget.
    OverBudget,
    /// The dependency graph contains a cycle.
    Cycle,
}

fn reject_export_source_cycles(
    graph: &std::collections::BTreeMap<RunId, BTreeSet<RunId>>,
) -> std::result::Result<(), ExportSourceClosureError> {
    let nodes = graph
        .keys()
        .cloned()
        .chain(graph.values().flat_map(|deps| deps.iter().cloned()))
        .collect::<BTreeSet<_>>();
    let mut incoming = nodes
        .iter()
        .cloned()
        .map(|run_id| (run_id, 0usize))
        .collect::<std::collections::BTreeMap<_, _>>();
    for dependencies in graph.values() {
        for dependency in dependencies {
            if let Some(count) = incoming.get_mut(dependency) {
                *count = count.saturating_add(1);
            }
        }
    }
    let mut ready = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(run_id, _)| run_id.clone())
        .collect::<BTreeSet<_>>();
    let mut seen = 0usize;
    while let Some(run_id) = ready.pop_first() {
        seen = seen.saturating_add(1);
        if let Some(dependencies) = graph.get(&run_id) {
            for dependency in dependencies {
                if let Some(count) = incoming.get_mut(dependency) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        ready.insert(dependency.clone());
                    }
                }
            }
        }
    }
    if seen != nodes.len() {
        return Err(ExportSourceClosureError::Cycle);
    }
    Ok(())
}

#[cfg(test)]
mod export_source_closure_tests {
    use super::{expand_export_source_closure, ExportSourceClosureError, MAX_EXPORT_SOURCE_RUNS};
    use mfm_ids::{DigestAlgorithm, RunId};
    use std::collections::{BTreeMap, BTreeSet};

    fn run(digit: u8) -> RunId {
        let hex = format!("{digit:x}").repeat(64);
        RunId::parse(format!("run:sha256-jcs-v1:{hex}")).expect("run id")
    }

    #[test]
    fn multi_hop_shared_and_deterministic() {
        let root = run(1);
        let mid = run(2);
        let shared = run(3);
        let other = run(4);
        // root -> mid, other; mid -> shared; other -> shared
        let graph = BTreeMap::from([
            (root.clone(), BTreeSet::from([mid.clone(), other.clone()])),
            (mid.clone(), BTreeSet::from([shared.clone()])),
            (other.clone(), BTreeSet::from([shared.clone()])),
            (shared.clone(), BTreeSet::new()),
        ]);
        let first = expand_export_source_closure(&root, graph[&root].clone(), |id| {
            Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
        })
        .expect("shared dag");
        let second = expand_export_source_closure(&root, graph[&root].clone(), |id| {
            Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
        })
        .expect("shared dag again");
        assert_eq!(first, second);
        assert_eq!(first, BTreeSet::from([mid, other, shared]));
    }

    #[test]
    fn cyclic_source_graph_is_rejected() {
        let root = run(1);
        let a = run(2);
        let b = run(3);
        let graph = BTreeMap::from([
            (root.clone(), BTreeSet::from([a.clone()])),
            (a.clone(), BTreeSet::from([b.clone()])),
            (b.clone(), BTreeSet::from([a.clone()])),
        ]);
        let error = expand_export_source_closure(&root, graph[&root].clone(), |id| {
            Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
        })
        .expect_err("cycle");
        assert_eq!(error, ExportSourceClosureError::Cycle);
    }

    #[test]
    fn over_budget_source_graph_is_rejected() {
        let root = run(0);
        let mut pending = BTreeSet::new();
        let mut graph = BTreeMap::new();
        // root fans out past the fixed budget.
        for index in 1..=(MAX_EXPORT_SOURCE_RUNS + 1) {
            let digit = (index % 15) as u8;
            // Distinct run ids via algorithm domain not available; use digest hex.
            let hex = format!("{index:064x}");
            let source = RunId::parse(format!("run:sha256-jcs-v1:{hex}")).expect("distinct run id");
            pending.insert(source.clone());
            graph.insert(source, BTreeSet::new());
            let _ = digit;
            let _ = DigestAlgorithm::Sha256JcsV1;
        }
        graph.insert(root.clone(), pending.clone());
        let error = expand_export_source_closure(&root, pending, |id| {
            Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
        })
        .expect_err("over budget");
        assert_eq!(error, ExportSourceClosureError::OverBudget);
    }
}
