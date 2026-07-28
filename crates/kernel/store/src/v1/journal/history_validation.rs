//! Certified-spec validation for a reconstructed committed journal.
//!
//! The physical journal fold validates the persisted algebra. This module performs the one
//! additional, callback-free pass that binds every recorded semantic identity and lifecycle
//! transition to the non-forgeable [`CertifiedTypedSpec`].

use super::super::*;
use mfm_certify::CertifiedTypedSpec;
use mfm_events::v1 as events;
use mfm_spec::v1 as spec;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

#[path = "history_validation/artifacts.rs"]
mod artifacts;
#[path = "history_validation/retention.rs"]
mod retention;
#[path = "history_validation/side_effects.rs"]
mod side_effects;
#[path = "history_validation/terminal.rs"]
mod terminal;

use self::side_effects::{
    derive_uncompleted_saga_mode, node_uses_side_effect_terminal_validation,
    validate_atomic_resource_lane_release_pairs, validate_atomic_side_effect_failure_pairs,
    validate_historical_side_effect_failure, validate_historical_side_effect_payload,
    validate_historical_side_effect_terminal, HistoricalSideEffectLedger,
};

/// Exact manually blocked prefix authority derived before one atomic journal batch.
#[derive(Debug, Default)]
pub(in crate::v1) struct ManualResolutionBatchAuthorities {
    by_batch_start: BTreeMap<usize, mfm_manual_auth::ManualResolutionPrefixAuthority>,
}

impl ManualResolutionBatchAuthorities {
    pub(in crate::v1) fn insert(
        &mut self,
        batch_start: usize,
        authority: mfm_manual_auth::ManualResolutionPrefixAuthority,
    ) -> Result<()> {
        if self.by_batch_start.insert(batch_start, authority).is_some() {
            return Err(invalid_history(
                "manual-resolution prefix authority was derived twice for one batch",
            ));
        }
        Ok(())
    }

    fn get(&self, batch_start: usize) -> Option<&mfm_manual_auth::ManualResolutionPrefixAuthority> {
        self.by_batch_start.get(&batch_start)
    }

    fn len(&self) -> usize {
        self.by_batch_start.len()
    }
}

/// Verified historical manual-resolution outcome retained by the certified fold.
///
/// The proof bytes and signer material are deliberately not retained after verification.
#[derive(Debug, Clone, Copy)]
pub(in crate::v1) struct VerifiedHistoricalManualResolution {
    outcome: events::ManualResolutionOutcome,
}

impl VerifiedHistoricalManualResolution {
    /// Returns the outcome derived from the verified historical authorization.
    pub(in crate::v1) const fn outcome(self) -> events::ManualResolutionOutcome {
        self.outcome
    }
}

/// Incremental certified-history frontier moved with a verified run view.
///
/// This is not a second lifecycle projection. It contains only the minimal certified-validation
/// frontier needed to reject an invalid strict successor without rescanning or refolding the
/// already verified prefix.
#[derive(Debug)]
pub(in crate::v1) struct CertifiedHistoryFold {
    available_cells: BTreeSet<CellId>,
    active_attempts: BTreeSet<(NodeId, AttemptId)>,
    skipped_attempts: BTreeSet<(NodeId, AttemptId)>,
    side_effect_ledgers: BTreeMap<SideEffectPairId, HistoricalSideEffectLedger>,
    seen_run_admitted: bool,
    produced_public_output: Option<events::PublicOutputCompletionEvidence>,
    retention_manifest_projected_seq: Option<StreamSeq>,
    saga_engaged: bool,
    manual_resolution: Option<VerifiedHistoricalManualResolution>,
    active_resource_lanes: BTreeSet<SideEffectPairId>,
    completed: bool,
    artifacts: artifacts::ArtifactHistoryFold,
    retention: retention::RetentionHistoryFold,
    terminal: terminal::TerminalHistoryFold,
    validated_record_count: usize,
}

impl CertifiedHistoryFold {
    /// Builds the certified-validation frontier for an initially loaded journal.
    pub(in crate::v1) fn build(
        certified: &CertifiedTypedSpec,
        journal: &CommittedRunJournal,
        projection: &ProjectionSnapshot,
        manual_authorities: &ManualResolutionBatchAuthorities,
    ) -> Result<Self> {
        let spec = HistorySpec::new(certified);
        let history = JournalHistory::new(journal);
        validate_run_admission_batch(&spec, journal.run_id(), &history)?;
        let mut fold = Self {
            available_cells: BTreeSet::new(),
            active_attempts: BTreeSet::new(),
            skipped_attempts: BTreeSet::new(),
            side_effect_ledgers: BTreeMap::new(),
            seen_run_admitted: false,
            produced_public_output: None,
            retention_manifest_projected_seq: None,
            saga_engaged: false,
            manual_resolution: None,
            active_resource_lanes: BTreeSet::new(),
            completed: false,
            artifacts: artifacts::ArtifactHistoryFold::new(),
            retention: retention::RetentionHistoryFold::new(),
            terminal: terminal::TerminalHistoryFold::new(),
            validated_record_count: 0,
        };
        fold.apply_records(&spec, journal, 0, projection, manual_authorities)?;
        Ok(fold)
    }

    /// Applies only a strict committed suffix to the moved validation frontier.
    pub(in crate::v1) fn apply_suffix(
        &mut self,
        certified: &CertifiedTypedSpec,
        successor: &CommittedRunJournal,
        suffix_start: usize,
        projection: &ProjectionSnapshot,
        manual_authorities: &ManualResolutionBatchAuthorities,
    ) -> Result<()> {
        if suffix_start != self.validated_record_count || suffix_start >= successor.records().len()
        {
            return Err(invalid_history(
                "successor suffix does not begin at the verified record frontier",
            ));
        }
        self.apply_records(
            &HistorySpec::new(certified),
            successor,
            suffix_start,
            projection,
            manual_authorities,
        )?;
        Ok(())
    }

    /// Returns the verified historical manual-resolution marker, when one was committed.
    pub(in crate::v1) const fn verified_manual_resolution(
        &self,
    ) -> Option<&VerifiedHistoricalManualResolution> {
        self.manual_resolution.as_ref()
    }

    fn apply_records(
        &mut self,
        spec: &HistorySpec<'_>,
        journal: &CommittedRunJournal,
        start: usize,
        projection: &ProjectionSnapshot,
        manual_authorities: &ManualResolutionBatchAuthorities,
    ) -> Result<()> {
        let run_id = journal.run_id();
        let records = journal.records();
        let suffix = records
            .get(start..)
            .ok_or_else(|| invalid_history("successor suffix start exceeds the journal"))?;
        if suffix.is_empty() {
            return Err(invalid_history("verified successor suffix cannot be empty"));
        }

        let known_side_effect_pairs = self
            .side_effect_ledgers
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let history = JournalHistory::new(journal);
        let mut used_manual_authorities = 0_usize;
        for batch in history.suffix_batches(start)? {
            let manual_count = batch
                .records()
                .iter()
                .filter(|event| {
                    matches!(
                        event.payload(),
                        events::KernelEventPayload::ManualResolutionRecorded(_)
                    )
                })
                .count();
            let manual_authority = manual_authorities.get(batch.start());
            match (manual_count, manual_authority) {
                (0, None) => {}
                (0, Some(_)) => {
                    return Err(invalid_history(
                        "manual-resolution prefix authority has no event in its batch",
                    ));
                }
                (_, None) => {
                    return Err(invalid_history(
                        "manual-resolution event is missing exact prefix authority",
                    ));
                }
                (_, Some(_)) => {
                    used_manual_authorities += 1;
                }
            }
            for event in batch.records() {
                if self.completed {
                    return Err(invalid_history(
                        "run journal contains records after RunCompleted",
                    ));
                }
                self.apply_record(spec, run_id, event, projection, manual_authority, journal)?;
            }
        }
        if used_manual_authorities != manual_authorities.len() {
            return Err(invalid_history(
                "manual-resolution prefix authority lies outside the applied suffix",
            ));
        }

        validate_atomic_resource_lane_release_pairs(spec, suffix)?;
        validate_atomic_terminal_pairs(spec, suffix)?;
        validate_atomic_side_effect_failure_pairs(spec, &known_side_effect_pairs, suffix)?;
        self.artifacts.apply_suffix(&history, start)?;
        self.retention.apply_suffix(spec, &history, start)?;
        let expected_saga_outcome = derive_uncompleted_saga_mode(
            spec,
            &self.side_effect_ledgers,
            self.saga_engaged,
            self.manual_resolution.map(|manual| manual.outcome()),
        )?
        .0
        .saga_terminal_outcome();
        self.terminal.apply_suffix(
            spec,
            run_id,
            &history,
            start,
            projection,
            expected_saga_outcome.as_ref(),
        )?;
        validate_recovery_frontier(spec, run_id, projection)?;
        self.validated_record_count = records.len();
        Ok(())
    }

    fn apply_record(
        &mut self,
        spec: &HistorySpec<'_>,
        run_id: &RunId,
        event: &KernelEventEnvelope,
        projection: &ProjectionSnapshot,
        manual_authority: Option<&mfm_manual_auth::ManualResolutionPrefixAuthority>,
        journal: &CommittedRunJournal,
    ) -> Result<()> {
        if self.completed {
            return Err(invalid_history(
                "run journal contains records after RunCompleted",
            ));
        }
        if event.run_id() != run_id {
            return Err(invalid_history(format!(
                "record for run {} appeared in journal {}",
                event.run_id(),
                run_id
            )));
        }
        if event.payload().spec_hash() != spec.spec_hash() {
            return Err(invalid_history(format!(
                "record payload spec hash {} does not match certified {}",
                event.payload().spec_hash(),
                spec.spec_hash()
            )));
        }
        if !self.seen_run_admitted
            && !matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_))
        {
            return Err(invalid_history(
                "run journal records appeared before RunAdmitted",
            ));
        }

        match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => {
                if self.seen_run_admitted {
                    return Err(invalid_history(
                        "run journal contains multiple RunAdmitted records",
                    ));
                }
                self.seen_run_admitted = true;
                for cell_id in validate_seed_cells(spec, &payload.seed_cells)?.into_keys() {
                    self.available_cells.insert(cell_id);
                }
            }
            events::KernelEventPayload::RunCompleted(payload) => {
                if !self.active_attempts.is_empty() {
                    return Err(invalid_history(
                        "RunCompleted cannot finalize while state attempts are active",
                    ));
                }
                validate_run_completed(
                    spec,
                    run_id,
                    event,
                    payload,
                    self.produced_public_output.as_ref(),
                    self.retention_manifest_projected_seq,
                )?;
                self.completed = true;
            }
            events::KernelEventPayload::ManualResolutionRecorded(payload) => {
                let manual_authority = manual_authority.ok_or_else(|| {
                    invalid_history("manual-resolution event is missing prefix authority")
                })?;
                validate_manual_resolution(
                    spec,
                    run_id,
                    event,
                    payload,
                    self,
                    manual_authority,
                    journal,
                )?;
                self.manual_resolution = Some(VerifiedHistoricalManualResolution {
                    outcome: payload.outcome,
                });
            }
            events::KernelEventPayload::RetentionRefsAppended(_) => {}
            events::KernelEventPayload::RetentionManifestProjected(payload) => {
                if &payload.run_id == run_id && payload.spec_hash == *spec.spec_hash() {
                    self.retention_manifest_projected_seq = Some(event.seq());
                }
            }
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                let node = spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "attempt started for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.state_kind != node.state_kind
                    || payload.state_version != node.state_version
                {
                    return Err(invalid_history(format!(
                        "attempt {} for node {} carries state identity outside the certified spec",
                        payload.attempt_id, payload.node_id
                    )));
                }
                validate_attempt_start_boundary(spec, node, payload, &self.available_cells)?;
                if !self
                    .active_attempts
                    .insert((payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    return Err(invalid_history(format!(
                        "attempt {} for node {} was started more than once",
                        payload.attempt_id, payload.node_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.output_cell_id != node.output_cell {
                    return Err(invalid_history(format!(
                        "attempt {} for node {} completed uncertified output cell {}",
                        payload.attempt_id, payload.node_id, payload.output_cell_id
                    )));
                }
                if node_uses_side_effect_terminal_validation(node) {
                    validate_historical_side_effect_terminal(
                        spec,
                        &self.side_effect_ledgers,
                        node,
                        &payload.attempt_id,
                        self.skipped_attempts
                            .contains(&(node.node_id.clone(), payload.attempt_id.clone())),
                    )?;
                }
                if !self
                    .active_attempts
                    .remove(&(payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    return Err(invalid_history(format!(
                        "attempt completion for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptInterrupted(payload) => {
                spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "attempt interrupted for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if !self
                    .active_attempts
                    .remove(&(payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    return Err(invalid_history(format!(
                        "attempt interruption for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                let node = spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if node_uses_side_effect_terminal_validation(node) {
                    validate_historical_side_effect_failure(
                        spec,
                        &self.side_effect_ledgers,
                        node,
                        &payload.attempt_id,
                    )?;
                }
                if !self
                    .active_attempts
                    .remove(&(payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    return Err(invalid_history(format!(
                        "attempt failure for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
                if !payload.retryable {
                    self.saga_engaged = true;
                }
            }
            events::KernelEventPayload::CellProduced(payload) => {
                validate_produced_cell(spec, projection, event, payload)?;
                let node = spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "produced cell references uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if node_uses_side_effect_terminal_validation(node) {
                    validate_historical_side_effect_terminal(
                        spec,
                        &self.side_effect_ledgers,
                        node,
                        &payload.attempt_id,
                        false,
                    )?;
                }
                self.available_cells.insert(payload.cell_id.clone());
                record_attempt_skip(
                    &mut self.skipped_attempts,
                    &payload.node_id,
                    &payload.attempt_id,
                    false,
                );
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                validate_skipped_cell(spec, projection, event, payload)?;
                let node = spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "skipped cell references uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if node_uses_side_effect_terminal_validation(node) {
                    validate_historical_side_effect_terminal(
                        spec,
                        &self.side_effect_ledgers,
                        node,
                        &payload.attempt_id,
                        true,
                    )?;
                }
                self.available_cells.insert(payload.cell_id.clone());
                record_attempt_skip(
                    &mut self.skipped_attempts,
                    &payload.node_id,
                    &payload.attempt_id,
                    true,
                );
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                validate_fact_recorded(spec, projection, &self.active_attempts, event, payload)?;
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                if let Some(node_id) = &payload.node_id {
                    spec.node(node_id).ok_or_else(|| {
                        invalid_history(format!(
                            "artifact referenced for uncertified node {node_id}"
                        ))
                    })?;
                }
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output(spec, node, payload)?;
                validate_public_output_produced(spec, projection, event, node, payload)?;
                self.produced_public_output = Some(events::PublicOutputCompletionEvidence {
                    public_output_schema_id: payload.public_schema_id.clone(),
                    public_output_event_id: event.event_id().clone(),
                });
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                let node = spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "public output failure by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output_render_node(
                    spec,
                    node,
                    &payload.public_schema_id,
                    &payload.renderer_descriptor_id,
                )?;
                validate_public_output_failed(projection, event.run_id(), payload)?;
            }
            events::KernelEventPayload::ResourceLaneClaimIntent(_)
            | events::KernelEventPayload::ResourceLaneReleaseIntent(_) => {
                return Err(invalid_history(
                    "run journal contains unmaterialized resource-lane intent",
                ));
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::ResourceLaneClaimed(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_)
            | events::KernelEventPayload::ResourceLaneReleased(_) => {
                validate_historical_side_effect_payload(
                    spec,
                    event.run_id(),
                    &self.active_attempts,
                    &mut self.side_effect_ledgers,
                    event.payload(),
                )?;
                match event.payload() {
                    events::KernelEventPayload::ResourceLaneClaimed(payload) => {
                        self.active_resource_lanes.insert(payload.pair_id.clone());
                    }
                    events::KernelEventPayload::ResourceLaneReleased(payload) => {
                        self.active_resource_lanes.remove(&payload.pair_id);
                    }
                    events::KernelEventPayload::SideEffectAmbiguous(payload)
                        if matches!(
                            payload.ledger_purpose,
                            events::SideEffectLedgerPurpose::Forward
                        ) =>
                    {
                        self.saga_engaged = true;
                    }
                    events::KernelEventPayload::SideEffectFailed(payload) if !payload.retryable => {
                        self.saga_engaged = true;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

pub(super) struct HistorySpec<'a> {
    certified: &'a CertifiedTypedSpec,
}

impl<'a> HistorySpec<'a> {
    fn new(certified: &'a CertifiedTypedSpec) -> Self {
        Self { certified }
    }

    pub(super) fn certified(&self) -> &'a CertifiedTypedSpec {
        self.certified
    }

    pub(super) fn spec(&self) -> &'a spec::TypedExecutionSpec {
        self.certified.validated_spec().spec()
    }

    pub(super) fn spec_hash(&self) -> &'a SpecHash {
        self.certified.spec_hash()
    }

    pub(super) fn node(&self, node_id: &NodeId) -> Option<&'a spec::NodeSpec> {
        let graph = self.certified.validated_spec().graph();
        graph.forward_node(node_id).or_else(|| {
            graph
                .remediations()
                .find_map(|(_, node)| (node.node_id == *node_id).then_some(node))
        })
    }

    pub(super) fn cell(&self, cell_id: &CellId) -> Option<&'a spec::CellSpec> {
        self.certified.validated_spec().graph().cell(cell_id)
    }

    pub(super) fn state_descriptor_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<&'a spec::StateDescriptorIdentity> {
        self.certified
            .descriptor_set()
            .state(&node.descriptor_id)
            .ok_or_else(|| {
                invalid_history(format!(
                    "certified node {} references missing state descriptor {}",
                    node.node_id, node.descriptor_id
                ))
            })
    }

    pub(super) fn executable_nodes(&self) -> impl Iterator<Item = &'a spec::NodeSpec> + use<'a> {
        let graph = self.certified.validated_spec().graph();
        graph
            .forward_nodes()
            .chain(graph.remediations().map(|(_, node)| node))
    }

    pub(super) fn fact_descriptor_hashes(&self) -> BTreeSet<ContentDigest> {
        self.executable_nodes()
            .flat_map(|node| {
                node.fact_descriptor_allowlist
                    .iter()
                    .map(|reference| reference.descriptor_hash.clone())
            })
            .collect()
    }

    pub(super) fn side_effect_pair_for_submit_node(
        &self,
        node_id: &NodeId,
    ) -> Option<&'a SideEffectPairId> {
        self.spec()
            .side_effect_verify_pair_for_submit_node(node_id)
            .ok()
            .map(|pair| pair.pair_id)
    }

    pub(super) fn validate_input_binding(
        &self,
        input: &spec::InputBindingNodeSpec,
    ) -> Result<Vec<CellId>> {
        let mut cells = Vec::new();
        self.validate_input_node(input, &mut cells)?;
        Ok(cells)
    }

    fn validate_input_node(
        &self,
        input: &spec::InputBindingNodeSpec,
        cells: &mut Vec<CellId>,
    ) -> Result<()> {
        match input {
            spec::InputBindingNodeSpec::Unit => {}
            spec::InputBindingNodeSpec::Cell(cell) => {
                let certified = self.cell(&cell.cell_id).ok_or_else(|| {
                    invalid_history(format!(
                        "input binding references missing cell {}",
                        cell.cell_id
                    ))
                })?;
                if certified.semantic_type_id != cell.semantic_type_id
                    || certified.schema_id != cell.schema_id
                    || certified.value_lineage != cell.value_lineage
                {
                    return Err(invalid_history(format!(
                        "input binding for cell {} does not match certified cell metadata",
                        cell.cell_id
                    )));
                }
                cells.push(cell.cell_id.clone());
            }
            spec::InputBindingNodeSpec::Tuple(elements) => {
                for element in elements {
                    self.validate_input_node(element, cells)?;
                }
            }
            spec::InputBindingNodeSpec::Struct(fields) => {
                for field in fields {
                    self.validate_input_node(&field.node, cells)?;
                }
            }
            spec::InputBindingNodeSpec::Vec { elements, .. }
            | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
                for element in elements {
                    self.validate_input_node(element, cells)?;
                }
            }
        }
        Ok(())
    }
}

pub(super) struct JournalHistory<'a> {
    journal: &'a CommittedRunJournal,
}

impl<'a> JournalHistory<'a> {
    fn new(journal: &'a CommittedRunJournal) -> Self {
        Self { journal }
    }

    pub(super) fn root_record(&self) -> Option<&'a KernelEventEnvelope> {
        self.journal.records().first()
    }

    pub(super) fn admission_root(&self) -> Result<&'a events::RunAdmitted> {
        super::run_admission_root(self.journal.records())
    }

    pub(super) fn batches(&self) -> impl Iterator<Item = JournalBatch<'a>> + '_ {
        self.journal.batches().iter().map(|batch| JournalBatch {
            records: self.journal.records(),
            range: batch.record_range.clone(),
        })
    }

    pub(super) fn suffix_batches(
        &self,
        suffix_start: usize,
    ) -> Result<impl Iterator<Item = JournalBatch<'a>> + '_> {
        let batches = self.journal.batches();
        let first = batches.partition_point(|batch| batch.record_range.end <= suffix_start);
        if batches
            .get(first)
            .is_some_and(|batch| batch.record_range.start != suffix_start)
        {
            return Err(invalid_history(
                "certified-history suffix splits an atomic journal batch",
            ));
        }
        Ok(batches[first..].iter().map(|batch| JournalBatch {
            records: self.journal.records(),
            range: batch.record_range.clone(),
        }))
    }

    pub(super) fn object(
        &self,
        artifact_id: &ArtifactId,
        evidence_hash: &ContentDigest,
    ) -> Option<&'a super::VerifiedJournalObject> {
        self.journal.object(artifact_id, evidence_hash)
    }
}

pub(super) struct JournalBatch<'a> {
    records: &'a [KernelEventEnvelope],
    range: Range<usize>,
}

impl<'a> JournalBatch<'a> {
    pub(super) fn start(&self) -> usize {
        self.range.start
    }

    pub(super) fn records(&self) -> &'a [KernelEventEnvelope] {
        &self.records[self.range.clone()]
    }
}

fn validate_run_admission_batch(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    history: &JournalHistory<'_>,
) -> Result<()> {
    let Some(first) = history.root_record() else {
        return Err(invalid_history("run journal is missing RunAdmitted root"));
    };
    if first.seq() != StreamSeq::FIRST || first.ordinal() != CommitOrdinal::new(0) {
        return Err(invalid_history(
            "RunAdmitted must be the first record in the run journal",
        ));
    }
    let first_batch = history
        .batches()
        .next()
        .ok_or_else(|| invalid_history("run journal is missing RunAdmitted root"))?;
    if first_batch.records().len() != 1 {
        return Err(invalid_history(
            "RunAdmitted batch must contain exactly one root record",
        ));
    }
    let events::KernelEventPayload::RunAdmitted(admitted) = first.payload() else {
        return Err(invalid_history("run journal must start with RunAdmitted"));
    };
    validate_run_admitted(spec, run_id, admitted)
}

fn validate_run_admitted(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    admitted: &events::RunAdmitted,
) -> Result<()> {
    validate_run_identity_material(spec, run_id, admitted)?;
    if admitted.run_id != *run_id
        || admitted.spec_hash != *spec.spec_hash()
        || admitted.spec_version != spec.spec().spec_version
        || admitted.lowering_version != spec.spec().lowering_version
        || admitted.public_output_schema_id != spec.spec().public_outputs.public_schema_id
        || admitted.saga_policy_digest
            != spec
                .spec()
                .saga
                .saga_policy_digest()
                .map_err(|error| invalid_history(error.to_string()))?
        || admitted.canonicalizer_identity
            != spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
        || admitted.descriptor_identities != spec.spec().descriptor_identities
    {
        return Err(invalid_history(
            "RunAdmitted payload does not match the certified typed spec",
        ));
    }
    validate_spec_artifact(spec, &admitted.spec_artifact)?;
    validate_certificate_artifact(spec, &admitted.certificate_artifact)?;
    validate_config_artifacts(spec, &admitted.config_artifacts)?;
    validate_fact_descriptor_artifacts(spec, &admitted.fact_descriptor_artifacts)?;
    validate_seed_cells(spec, &admitted.seed_cells)?;
    Ok(())
}

fn validate_run_identity_material(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    admitted: &events::RunAdmitted,
) -> Result<()> {
    if &admitted.run_id != run_id {
        return Err(invalid_history(format!(
            "RunAdmitted targets {} while journal authority targets {}",
            admitted.run_id, run_id
        )));
    }
    if &admitted.spec_hash != spec.spec_hash() {
        return Err(invalid_history(format!(
            "RunAdmitted spec hash {} does not match certified {}",
            admitted.spec_hash,
            spec.spec_hash()
        )));
    }
    if admitted.identity_material.certified_spec_hash != admitted.spec_hash {
        return Err(invalid_history(
            "RunAdmitted identity-material spec hash does not match its payload",
        ));
    }
    let derived = admitted
        .identity_material
        .derive_run_id()
        .map_err(|error| invalid_history(error.to_string()))?;
    if derived != admitted.run_id {
        return Err(invalid_history(
            "RunAdmitted run id does not match its identity material",
        ));
    }
    Ok(())
}

fn validate_spec_artifact(
    spec: &HistorySpec<'_>,
    evidence: &events::RunArtifactEvidenceRef,
) -> Result<()> {
    let canonical = spec
        .spec()
        .canonical_json()
        .map_err(|error| invalid_history(error.to_string()))?;
    validate_certified_artifact(
        evidence,
        &canonical,
        &spec.spec().media_type,
        &spec::typed_execution_spec_schema_id()
            .map_err(|error| invalid_history(error.to_string()))?,
        events::ArtifactRole::TypedExecutionSpec,
        "typed execution spec artifact evidence does not match the certified spec",
    )
}

fn validate_certificate_artifact(
    spec: &HistorySpec<'_>,
    evidence: &events::RunArtifactEvidenceRef,
) -> Result<()> {
    let canonical = spec
        .certified()
        .certificate()
        .canonical_json()
        .map_err(|error| invalid_history(error.to_string()))?;
    let media_type = spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
        .map_err(|error| invalid_history(error.to_string()))?;
    validate_certified_artifact(
        evidence,
        &canonical,
        &media_type,
        &mfm_certify::typed_spec_certificate_schema_id()
            .map_err(|error| invalid_history(error.to_string()))?,
        events::ArtifactRole::TypedSpecCertificate,
        "typed spec certificate artifact evidence does not match the certified spec",
    )
}

fn validate_certified_artifact(
    evidence: &events::RunArtifactEvidenceRef,
    canonical: &mfm_canonical::PlainCanonicalJsonBytes,
    media_type: &spec::MediaType,
    schema_id: &SchemaId,
    role: events::ArtifactRole,
    mismatch: &'static str,
) -> Result<()> {
    let digest = canonical.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.artifact_id != artifact_id
        || evidence.content_digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != *media_type
        || evidence.schema_id.as_ref() != Some(schema_id)
        || evidence.semantic_type_id.is_some()
        || evidence.role != role
    {
        return Err(invalid_history(mismatch));
    }
    Ok(())
}

fn validate_config_artifacts(
    spec: &HistorySpec<'_>,
    evidence: &[events::RunArtifactEvidenceRef],
) -> Result<()> {
    let mut by_key = BTreeMap::new();
    for artifact in evidence {
        let Some(schema_id) = artifact.schema_id.clone() else {
            return Err(invalid_history(
                "typed config artifact evidence must carry schema_id",
            ));
        };
        if artifact.role != events::ArtifactRole::TypedConfig || artifact.semantic_type_id.is_some()
        {
            return Err(invalid_history(
                "typed config artifact evidence has invalid role or semantic metadata",
            ));
        }
        let key = format!("{}:{}", schema_id, artifact.content_digest);
        if by_key.insert(key, artifact).is_some() {
            return Err(invalid_history("duplicate typed config artifact evidence"));
        }
    }
    for config in &spec.spec().config_refs {
        let key = format!("{}:{}", config.schema_id, config.digest);
        let artifact = by_key.remove(&key).ok_or_else(|| {
            invalid_history(format!(
                "missing typed config evidence for schema {} digest {}",
                config.schema_id, config.digest
            ))
        })?;
        if artifact.artifact_id != config.artifact_id
            || artifact.content_digest != config.digest
            || artifact.byte_len != config.byte_len
            || artifact.media_type != config.media_type
            || artifact.schema_id.as_ref() != Some(&config.schema_id)
        {
            return Err(invalid_history(format!(
                "typed config evidence for schema {} digest {} does not match certified config",
                config.schema_id, config.digest
            )));
        }
    }
    if !by_key.is_empty() {
        return Err(invalid_history(
            "typed config evidence contains entries absent from the certified spec",
        ));
    }
    Ok(())
}

fn validate_fact_descriptor_artifacts(
    spec: &HistorySpec<'_>,
    artifacts: &[events::RunArtifactEvidenceRef],
) -> Result<()> {
    let required = spec.fact_descriptor_hashes();
    let schema_id = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| invalid_history(error.to_string()))?;
    let media_type = spec::MediaType::new("application/json")
        .map_err(|error| invalid_history(error.to_string()))?;
    let mut admitted = BTreeSet::new();
    for artifact in artifacts {
        if artifact.role != events::ArtifactRole::FactDescriptor
            || artifact.content_digest.algorithm() != DigestAlgorithm::Sha256JcsV1
            || artifact.media_type != media_type
            || artifact.schema_id.as_ref() != Some(&schema_id)
            || artifact.semantic_type_id.is_some()
        {
            return Err(invalid_history(format!(
                "fact descriptor artifact {} has invalid metadata",
                artifact.artifact_id
            )));
        }
        let expected = ArtifactId::from_digest(
            artifact.content_digest.algorithm(),
            *artifact.content_digest.digest(),
        );
        if artifact.artifact_id != expected || !admitted.insert(artifact.content_digest.clone()) {
            return Err(invalid_history(format!(
                "fact descriptor artifact {} has invalid or duplicate identity",
                artifact.artifact_id
            )));
        }
    }
    if admitted != required {
        return Err(invalid_history(
            "RunAdmitted fact descriptors do not match certified allow-lists",
        ));
    }
    Ok(())
}

fn validate_seed_cells(
    spec: &HistorySpec<'_>,
    seed_cells: &[events::SeedCellRef],
) -> Result<BTreeMap<CellId, ()>> {
    let mut by_seed = BTreeMap::new();
    let mut by_cell = BTreeMap::new();
    for seed in seed_cells {
        if by_seed.insert(seed.seed_id.clone(), seed).is_some() {
            return Err(invalid_history(format!(
                "duplicate seed evidence for {}",
                seed.seed_id
            )));
        }
        if by_cell.insert(seed.cell_id.clone(), ()).is_some() {
            return Err(invalid_history(format!(
                "duplicate seed-cell evidence for {}",
                seed.cell_id
            )));
        }
    }
    for declared in &spec.spec().seeds {
        let seed = by_seed.get(&declared.seed_id).ok_or_else(|| {
            invalid_history(format!(
                "missing RunAdmitted seed evidence for {}",
                declared.seed_id
            ))
        })?;
        if seed.cell_id != declared.cell_id
            || seed.scope_id != declared.scope_id
            || seed.schema_id != declared.schema_id
            || seed.semantic_type_id != declared.semantic_type_id
            || declared
                .required_digest
                .as_ref()
                .is_some_and(|required| required != &seed.digest)
            || seed.seed_artifact.role != events::ArtifactRole::SeedInput
            || seed.seed_artifact.schema_id != declared.schema_id
            || seed.seed_artifact.semantic_type_id.as_ref() != Some(&declared.semantic_type_id)
            || seed.seed_artifact.content_digest != seed.digest
        {
            return Err(invalid_history(format!(
                "RunAdmitted seed {} does not match certified seed metadata",
                declared.seed_id
            )));
        }
    }
    if by_seed.len() != spec.spec().seeds.len() {
        return Err(invalid_history(
            "RunAdmitted contains seed evidence absent from the certified spec",
        ));
    }
    Ok(by_cell)
}

fn validate_attempt_start_boundary(
    spec: &HistorySpec<'_>,
    node: &spec::NodeSpec,
    payload: &events::StateAttemptStarted,
    available_cells: &BTreeSet<CellId>,
) -> Result<()> {
    for cell_id in spec.validate_input_binding(&node.input_bindings.root)? {
        if !available_cells.contains(&cell_id) {
            return Err(invalid_history(format!(
                "attempt {} for node {} started before input cell {} was terminal",
                payload.attempt_id, node.node_id, cell_id
            )));
        }
    }
    Ok(())
}

fn validate_manual_resolution(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    event: &KernelEventEnvelope,
    payload: &events::ManualResolutionRecorded,
    fold: &CertifiedHistoryFold,
    prefix: &mfm_manual_auth::ManualResolutionPrefixAuthority,
    journal: &CommittedRunJournal,
) -> Result<()> {
    if event.run_id() != run_id
        || payload.run_id != *run_id
        || payload.spec_hash != *spec.spec_hash()
    {
        return Err(invalid_history(
            "manual resolution identity does not match certified run",
        ));
    }
    if fold.manual_resolution.is_some() {
        return Err(invalid_history(
            "run journal contains multiple manual resolutions",
        ));
    }
    if !fold.active_attempts.is_empty() || !fold.active_resource_lanes.is_empty() {
        return Err(invalid_history(
            "manual resolution was recorded with active attempt or resource-lane authority",
        ));
    }
    let manual = match &spec.spec().saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual.as_ref(),
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => manual.as_ref(),
        _ => {
            return Err(invalid_history(
                "manual resolution was recorded without certified manual policy",
            ));
        }
    };
    if payload.evidence_schema_id != manual.evidence_schema {
        return Err(invalid_history(
            "manual resolution evidence schema does not match certified policy",
        ));
    }
    let authorization_schema = mfm_manual_auth::manual_authorization_proof_schema_id()
        .map_err(|error| invalid_history(error.to_string()))?;
    if payload.authorization_schema_id != authorization_schema {
        return Err(invalid_history(
            "manual authorization schema does not match certified policy",
        ));
    }
    let (mode, reason) =
        derive_uncompleted_saga_mode(spec, &fold.side_effect_ledgers, fold.saga_engaged, None)?;
    if mode != RunMode::ManualBlocked || reason.is_none() {
        return Err(invalid_history(
            "manual resolution prefix is not certified as manually blocked",
        ));
    }
    verify_manual_resolution_authorization(event, payload, prefix, journal)
}

fn verify_manual_resolution_authorization(
    event: &KernelEventEnvelope,
    payload: &events::ManualResolutionRecorded,
    prefix: &mfm_manual_auth::ManualResolutionPrefixAuthority,
    journal: &CommittedRunJournal,
) -> Result<()> {
    let requirements = event_artifact_requirements(event.payload());
    let evidence = exact_manual_requirement(
        &requirements,
        EventArtifactReferenceSource::ManualResolutionEvidence,
    )?;
    let authorization = exact_manual_requirement(
        &requirements,
        EventArtifactReferenceSource::ManualResolutionAuthorization,
    )?;
    exact_required_object(journal, evidence)?;
    let proof_object = exact_required_object(journal, authorization)?;
    let verified = mfm_manual_auth::ManualResolutionProofAuthority::new(
        prefix.clone(),
        payload.outcome,
        manual_evidence_ref(evidence)?,
        manual_evidence_ref(authorization)?,
        proof_object.bytes.clone(),
    )
    .and_then(mfm_manual_auth::ManualResolutionProofAuthority::verify)
    .map_err(|error| {
        invalid_history(format!(
            "manual-resolution authorization verification failed: {error}"
        ))
    })?;
    if verified.outcome() != payload.outcome {
        return Err(invalid_history(
            "manual-resolution authorization outcome does not match its event",
        ));
    }
    Ok(())
}

fn exact_manual_requirement(
    requirements: &[EventArtifactRequirement],
    source: EventArtifactReferenceSource,
) -> Result<&EventArtifactRequirement> {
    let mut matches = requirements
        .iter()
        .filter(|requirement| requirement.source == source);
    let requirement = matches
        .next()
        .ok_or_else(|| invalid_history("manual-resolution event is missing artifact authority"))?;
    if matches.next().is_some() {
        return Err(invalid_history(
            "manual-resolution event repeats artifact authority",
        ));
    }
    Ok(requirement)
}

fn exact_required_object<'a>(
    journal: &'a CommittedRunJournal,
    requirement: &EventArtifactRequirement,
) -> Result<&'a super::VerifiedJournalObject> {
    if !journal.requires_object(requirement) {
        return Err(invalid_history(
            "manual-resolution artifact is not required by its exact event",
        ));
    }
    let object = journal
        .object(&requirement.artifact_id, &requirement.evidence_hash)
        .ok_or_else(|| {
            invalid_history("manual-resolution exact retained artifact object is missing")
        })?;
    validate_artifact_requirement_against_evidence(requirement, &object.evidence)
        .map_err(|error| invalid_history(error.to_string()))?;
    verify_retained_artifact_bytes(&object.bytes, &object.evidence)
        .map_err(|error| invalid_history(error.to_string()))?;
    Ok(object)
}

fn manual_evidence_ref(
    requirement: &EventArtifactRequirement,
) -> Result<mfm_manual_auth::ManualResolutionEvidenceRef> {
    Ok(mfm_manual_auth::ManualResolutionEvidenceRef {
        schema_id: requirement.schema_id.clone().ok_or_else(|| {
            invalid_history("manual-resolution artifact requirement has no schema")
        })?,
        content_hash: requirement.digest.clone().ok_or_else(|| {
            invalid_history("manual-resolution artifact requirement has no content digest")
        })?,
        artifact_id: requirement.artifact_id.clone(),
    })
}

fn validate_produced_cell(
    spec: &HistorySpec<'_>,
    projection: &ProjectionSnapshot,
    event: &KernelEventEnvelope,
    payload: &events::CellProduced,
) -> Result<()> {
    let cell = spec
        .cell(&payload.cell_id)
        .ok_or_else(|| invalid_history(format!("produced uncertified cell {}", payload.cell_id)))?;
    let node = spec.node(&payload.node_id).ok_or_else(|| {
        invalid_history(format!(
            "cell {} was produced by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
        || cell.context != payload.context
    {
        return Err(invalid_history(format!(
            "produced cell {} does not match certified metadata",
            payload.cell_id
        )));
    }
    match projection.cell_terminal_for_run(event.run_id(), &payload.cell_id) {
        Some(CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
            evidence_hash,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && artifact_id == &payload.artifact_id
            && content_digest == &payload.content_digest
            && evidence_hash == &payload.evidence_hash =>
        {
            Ok(())
        }
        _ => Err(invalid_history(format!(
            "produced cell {} projection does not match authoritative record",
            payload.cell_id
        ))),
    }
}

fn validate_skipped_cell(
    spec: &HistorySpec<'_>,
    projection: &ProjectionSnapshot,
    event: &KernelEventEnvelope,
    payload: &events::CellSkipped,
) -> Result<()> {
    let cell = spec
        .cell(&payload.cell_id)
        .ok_or_else(|| invalid_history(format!("skipped uncertified cell {}", payload.cell_id)))?;
    let node = spec.node(&payload.node_id).ok_or_else(|| {
        invalid_history(format!(
            "cell {} was skipped by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
        || cell.context != payload.context
        || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
    {
        return Err(invalid_history(format!(
            "skipped cell {} does not match certified metadata",
            payload.cell_id
        )));
    }
    match projection.cell_terminal_for_run(event.run_id(), &payload.cell_id) {
        Some(CellTerminalProjection::Skipped {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            skip_reason,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && skip_reason == &payload.skip_reason =>
        {
            Ok(())
        }
        _ => Err(invalid_history(format!(
            "skipped cell {} projection does not match authoritative record",
            payload.cell_id
        ))),
    }
}

fn validate_fact_recorded(
    spec: &HistorySpec<'_>,
    projection: &ProjectionSnapshot,
    active_attempts: &BTreeSet<(NodeId, AttemptId)>,
    event: &KernelEventEnvelope,
    payload: &events::FactRecorded,
) -> Result<()> {
    let node = spec.node(&payload.node_id).ok_or_else(|| {
        invalid_history(format!(
            "fact recorded for uncertified node {}",
            payload.node_id
        ))
    })?;
    if !node
        .fact_descriptor_allowlist
        .iter()
        .any(|reference| &reference.descriptor_hash == payload.claim.fact_descriptor_hash())
    {
        return Err(invalid_history(format!(
            "fact descriptor {} is not certified for node {}",
            payload.claim.fact_descriptor_hash(),
            payload.node_id
        )));
    }
    let descriptor = spec.state_descriptor_for_node(node)?;
    if descriptor.effect_class != "read_external"
        || descriptor.emitted_fact_descriptors.len() != 1
        || node.fact_descriptor_allowlist.len() != 1
    {
        return Err(invalid_history(format!(
            "fact for node {} was not produced by one certified fact-emitting external read",
            node.node_id
        )));
    }
    require_projected_attempt(projection, &payload.node_id, &payload.attempt_id, "fact")?;
    if !active_attempts.contains(&(payload.node_id.clone(), payload.attempt_id.clone())) {
        return Err(invalid_history(format!(
            "fact {} for node {} attempt {} was recorded outside an active attempt",
            payload.claim.subject().fact_key(),
            payload.node_id,
            payload.attempt_id
        )));
    }
    let claim_id = mfm_facts::derive_fact_claim_id(
        event.run_id().clone(),
        event.seq().as_u64(),
        event.ordinal().as_u32(),
    )
    .map_err(|error| invalid_history(error.to_string()))?;
    let projected = projection.fact_query_entry(&claim_id).ok_or_else(|| {
        invalid_history(format!(
            "fact {} for node {} attempt {} is not projected",
            payload.claim.subject().fact_key(),
            payload.node_id,
            payload.attempt_id
        ))
    })?;
    let projected_ref = projected
        .internal_ref()
        .map_err(|error| invalid_history(error.to_string()))?;
    let record_ref = mfm_facts::InternalFactRef::from_claim(
        claim_id,
        event.event_id().clone(),
        projected.recorded_at().to_owned(),
        payload.node_id.clone(),
        &payload.claim,
    )
    .map_err(|error| invalid_history(error.to_string()))?;
    if projected.source_event_id() != event.event_id()
        || projected.producer_node_id() != &payload.node_id
        || projected.attempt_id() != &payload.attempt_id
        || projected_ref != record_ref
    {
        return Err(invalid_history(format!(
            "fact {} projection does not match authoritative record",
            payload.claim.subject().fact_key()
        )));
    }
    Ok(())
}

fn require_projected_attempt(
    projection: &ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    kind: &'static str,
) -> Result<AttemptStatus> {
    projection
        .attempt(node_id, attempt_id)
        .map(|attempt| attempt.status.clone())
        .ok_or_else(|| {
            invalid_history(format!(
                "{kind} references missing attempt {attempt_id} for node {node_id}"
            ))
        })
}

fn validate_public_output(
    spec: &HistorySpec<'_>,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    validate_public_output_render_node(
        spec,
        node,
        &payload.public_schema_id,
        &payload.renderer_descriptor_id,
    )?;
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(invalid_history("public output node lacks render contract"));
    };
    if payload.output_spec_digest != render.output_spec_digest
        || payload.receipt_cell_id != node.output_cell
        || payload.cells.len() != render.required_cells.len()
    {
        return Err(invalid_history(format!(
            "public output for node {} does not match certified render contract",
            node.node_id
        )));
    }
    for (actual, expected) in payload.cells.iter().zip(&render.required_cells) {
        if actual.public_field_path != expected.public_field_path
            || actual.cell_id != expected.cell_id
            || actual.producer != expected.producer
            || actual.scope_id != expected.scope_id
            || actual.semantic_type_id != expected.semantic_type_id
            || actual.schema_id != expected.schema_id
            || actual.value_lineage != expected.value_lineage
        {
            return Err(invalid_history(format!(
                "public-output cell evidence for node {} does not match certified output",
                node.node_id
            )));
        }
    }
    Ok(())
}

fn validate_public_output_render_node(
    spec: &HistorySpec<'_>,
    node: &spec::NodeSpec,
    public_schema_id: &SchemaId,
    renderer_descriptor_id: &DescriptorId,
) -> Result<()> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(invalid_history(format!(
            "node {} is not a certified public-output render node",
            node.node_id
        )));
    };
    if &render.public_schema_id != public_schema_id
        || &spec.spec().public_outputs.public_schema_id != public_schema_id
        || render.renderer_descriptor.descriptor_id != *renderer_descriptor_id
    {
        return Err(invalid_history(format!(
            "public-output render metadata for node {} does not match certified spec",
            node.node_id
        )));
    }
    Ok(())
}

fn validate_public_output_produced(
    spec: &HistorySpec<'_>,
    projection: &ProjectionSnapshot,
    event: &KernelEventEnvelope,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    match require_projected_attempt(
        projection,
        &payload.node_id,
        &payload.attempt_id,
        "public output",
    )? {
        AttemptStatus::Completed { output_cell_id } if output_cell_id == node.output_cell => {}
        _ => {
            return Err(invalid_history(format!(
                "public output for node {} attempt {} lacks completed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    for cell in &payload.cells {
        let certified = spec.cell(&cell.cell_id).ok_or_else(|| {
            invalid_history(format!(
                "public output references uncertified cell {}",
                cell.cell_id
            ))
        })?;
        match projection.cell_terminal_for_run(event.run_id(), &cell.cell_id) {
            Some(CellTerminalProjection::Produced {
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                evidence_hash,
                ..
            }) if schema_id == &cell.schema_id
                && semantic_type_id == &cell.semantic_type_id
                && artifact_id == &cell.artifact_id
                && content_digest == &cell.content_digest
                && evidence_hash == &cell.evidence_hash
                && certified.producer == cell.producer
                && certified.scope_id == cell.scope_id
                && certified.value_lineage == cell.value_lineage => {}
            _ => {
                return Err(invalid_history(format!(
                    "public-output source cell {} lacks matching terminal evidence",
                    cell.cell_id
                )));
            }
        }
    }
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(invalid_history("public output node lacks render contract"));
    };
    let rendered_digest = public_output_rendered_digest(render, &payload.cells)?;
    if payload.rendered_digest != rendered_digest {
        return Err(invalid_history(format!(
            "public output for node {} carries incorrect rendered digest",
            node.node_id
        )));
    }
    let receipt_digest = public_output_receipt_digest(
        render,
        &payload.cells,
        &rendered_digest,
        payload.rendered_artifact_id.as_ref(),
    )?;
    let receipt_artifact =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let output_cell = spec.cell(&node.output_cell).ok_or_else(|| {
        invalid_history(format!(
            "render node {} output cell {} is absent",
            node.node_id, node.output_cell
        ))
    })?;
    let receipt_schema = spec::public_output_receipt_schema_id()
        .map_err(|error| invalid_history(error.to_string()))?;
    match projection.cell_terminal_for_run(event.run_id(), &node.output_cell) {
        Some(CellTerminalProjection::Produced {
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
            ..
        }) if schema_id == &receipt_schema
            && semantic_type_id == &output_cell.semantic_type_id
            && artifact_id == &receipt_artifact
            && content_digest == &receipt_digest => {}
        _ => {
            return Err(invalid_history(format!(
                "public output for node {} lacks matching receipt cell",
                node.node_id
            )));
        }
    }
    match projection.public_output(event.run_id(), &payload.public_schema_id) {
        Some(PublicOutputProjection::Produced {
            event_id,
            rendered_digest,
            rendered_artifact_id,
        }) if event_id == event.event_id()
            && rendered_digest == &payload.rendered_digest
            && rendered_artifact_id == &payload.rendered_artifact_id =>
        {
            Ok(())
        }
        _ => Err(invalid_history(format!(
            "public-output projection for schema {} does not match authoritative record",
            payload.public_schema_id
        ))),
    }
}

fn validate_public_output_failed(
    projection: &ProjectionSnapshot,
    run_id: &RunId,
    payload: &events::PublicOutputRenderFailed,
) -> Result<()> {
    match require_projected_attempt(
        projection,
        &payload.node_id,
        &payload.attempt_id,
        "public-output failure",
    )? {
        AttemptStatus::Failed { .. } => {}
        _ => {
            return Err(invalid_history(format!(
                "public-output failure for node {} attempt {} lacks failed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    match projection.public_output(run_id, &payload.public_schema_id) {
        Some(PublicOutputProjection::Produced { .. })
        | Some(PublicOutputProjection::RenderFailed { .. }) => Ok(()),
        _ => Err(invalid_history(format!(
            "public-output failure for schema {} is not projected",
            payload.public_schema_id
        ))),
    }
}

fn validate_run_completed(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    event: &KernelEventEnvelope,
    payload: &events::RunCompleted,
    produced_output: Option<&events::PublicOutputCompletionEvidence>,
    retention_seq: Option<StreamSeq>,
) -> Result<()> {
    if &payload.run_id != run_id || payload.spec_hash != *spec.spec_hash() {
        return Err(invalid_history(
            "RunCompleted identity does not match certified run",
        ));
    }
    let events::RunCompletionOutcome::Completed(completion) = &payload.outcome else {
        return Ok(());
    };
    if completion.public_output_schema_id != spec.spec().public_outputs.public_schema_id {
        return Err(invalid_history(format!(
            "RunCompleted references uncertified public schema {}",
            completion.public_output_schema_id
        )));
    }
    match produced_output {
        Some(produced) if produced == completion.as_ref() => {}
        Some(_) => {
            return Err(invalid_history(
                "RunCompleted public-output evidence differs from preceding output",
            ));
        }
        None => {
            return Err(invalid_history(
                "RunCompleted appeared before PublicOutputProduced",
            ));
        }
    }
    match retention_seq {
        Some(seq) if seq < event.seq() => Ok(()),
        Some(_) => Err(invalid_history(
            "RunCompleted must follow a prior retention projection batch",
        )),
        None => Err(invalid_history(
            "RunCompleted appeared before retention projection",
        )),
    }
}

fn validate_recovery_frontier(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    projection: &ProjectionSnapshot,
) -> Result<()> {
    for node in spec.executable_nodes() {
        if let Some(terminal) = projection.cell_terminal_for_run(run_id, &node.output_cell) {
            validate_terminal_cell_has_completed_attempt(spec, projection, node, terminal)?;
        }
        let mut started = None;
        for ((attempt_node_id, attempt_id), attempt) in projection.attempts() {
            if attempt_node_id != &node.node_id {
                continue;
            }
            match &attempt.status {
                AttemptStatus::Started { .. } => {
                    if projection
                        .cell_terminal_for_run(run_id, &node.output_cell)
                        .is_some()
                    {
                        return Err(invalid_history(format!(
                            "node {} has a started attempt after its output became terminal",
                            node.node_id
                        )));
                    }
                    if started.replace(attempt_id).is_some() {
                        return Err(invalid_history(format!(
                            "node {} has multiple started attempts",
                            node.node_id
                        )));
                    }
                }
                AttemptStatus::Completed { output_cell_id }
                    if projection
                        .cell_terminal_for_run(run_id, output_cell_id)
                        .is_none() =>
                {
                    return Err(invalid_history(format!(
                        "node {} attempt {} completed without terminal cell projection",
                        node.node_id, attempt_id
                    )));
                }
                AttemptStatus::Completed { .. }
                | AttemptStatus::Failed { .. }
                | AttemptStatus::Interrupted => {}
            }
        }
    }
    Ok(())
}

fn validate_terminal_cell_has_completed_attempt(
    spec: &HistorySpec<'_>,
    projection: &ProjectionSnapshot,
    node: &spec::NodeSpec,
    terminal: &CellTerminalProjection,
) -> Result<AttemptId> {
    let cell = spec.cell(&node.output_cell).ok_or_else(|| {
        invalid_history(format!(
            "node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let (terminal_node, terminal_attempt, schema_id, semantic_type_id) = match terminal {
        CellTerminalProjection::Produced {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        }
        | CellTerminalProjection::Skipped {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        } => (node_id, attempt_id, schema_id, semantic_type_id),
    };
    if terminal_node != &node.node_id
        || cell.producer != spec::CellProducer::Node(node.node_id.clone())
        || schema_id != &cell.schema_id
        || semantic_type_id != &cell.semantic_type_id
    {
        return Err(invalid_history(format!(
            "terminal cell {} is not certified evidence for node {}",
            node.output_cell, node.node_id
        )));
    }
    match projection.attempt(&node.node_id, terminal_attempt) {
        Some(AttemptProjection {
            status: AttemptStatus::Completed { output_cell_id },
            ..
        }) if output_cell_id == &node.output_cell => Ok(terminal_attempt.clone()),
        _ => Err(invalid_history(format!(
            "terminal cell {} lacks matching completed attempt {}",
            node.output_cell, terminal_attempt
        ))),
    }
}

fn public_output_rendered_digest(
    render: &spec::PublicOutputRenderNodeSpec,
    cells: &[events::NamedTypedCellRef],
) -> Result<ContentDigest> {
    Ok(canonical_json(serde_json::json!({
        "cells": cells.iter().map(public_output_cell_json).collect::<Vec<_>>(),
        "output_spec_digest": render.output_spec_digest.as_str(),
        "public_schema_id": render.public_schema_id.as_str(),
        "renderer_descriptor_id": render.renderer_descriptor.descriptor_id.as_str(),
    }))?
    .content_digest())
}

fn public_output_receipt_digest(
    render: &spec::PublicOutputRenderNodeSpec,
    cells: &[events::NamedTypedCellRef],
    rendered_digest: &ContentDigest,
    rendered_artifact_id: Option<&ArtifactId>,
) -> Result<ContentDigest> {
    Ok(canonical_json(serde_json::json!({
        "cells": cells.iter().map(public_output_cell_json).collect::<Vec<_>>(),
        "output_spec_digest": render.output_spec_digest.as_str(),
        "public_schema_id": render.public_schema_id.as_str(),
        "rendered_artifact_id": rendered_artifact_id.map(ArtifactId::as_str),
        "rendered_digest": rendered_digest.as_str(),
        "renderer_descriptor_id": render.renderer_descriptor.descriptor_id.as_str(),
    }))?
    .content_digest())
}

fn public_output_cell_json(cell: &events::NamedTypedCellRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": cell.artifact_id.as_str(),
        "cell_id": cell.cell_id.as_str(),
        "content_digest": cell.content_digest.as_str(),
        "producer": cell_producer_json(&cell.producer),
        "public_field_path": cell.public_field_path.as_str(),
        "schema_id": cell.schema_id.as_str(),
        "scope_id": cell.scope_id.as_str(),
        "semantic_type_id": cell.semantic_type_id.as_str(),
        "value_lineage": cell.value_lineage.lineage_digest.as_str(),
    })
}

fn cell_producer_json(producer: &spec::CellProducer) -> serde_json::Value {
    match producer {
        spec::CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
        spec::CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
    }
}

fn validate_atomic_terminal_pairs(
    spec: &HistorySpec<'_>,
    records: &[KernelEventEnvelope],
) -> Result<()> {
    let mut completions = BTreeSet::new();
    let mut terminal_cells = BTreeSet::new();
    let mut public_outputs = BTreeSet::new();
    for event in records {
        match event.payload() {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                completions.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.output_cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellProduced(payload) => {
                spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "cell produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "cell skipped by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = spec.node(&payload.node_id).ok_or_else(|| {
                    invalid_history(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if !matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                ) {
                    return Err(invalid_history(format!(
                        "public output produced by non-render node {}",
                        payload.node_id
                    )));
                }
                public_outputs.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.receipt_cell_id.clone(),
                ));
            }
            _ => {}
        }
    }
    for terminal in &completions {
        if !terminal_cells.contains(terminal) {
            return Err(invalid_history(format!(
                "attempt {} for node {} completed without terminal cell {} in the same batch",
                terminal.2, terminal.1, terminal.3
            )));
        }
    }
    for terminal in &terminal_cells {
        if !completions.contains(terminal) {
            return Err(invalid_history(format!(
                "terminal cell {} for node {} lacks StateAttemptCompleted in the same batch",
                terminal.3, terminal.1
            )));
        }
    }
    for terminal in public_outputs {
        if !terminal_cells.contains(&terminal) || !completions.contains(&terminal) {
            return Err(invalid_history(format!(
                "public output for node {} attempt {} was split from its receipt terminal cell",
                terminal.1, terminal.2
            )));
        }
    }
    Ok(())
}

pub(super) fn invalid_history(message: impl Into<String>) -> StoreError {
    StoreError::PersistedEventMismatch {
        field: "certified_history",
        message: message.into(),
    }
}

fn record_attempt_skip(
    skipped_attempts: &mut BTreeSet<(NodeId, AttemptId)>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    skipped: bool,
) {
    if skipped {
        skipped_attempts.insert((node_id.clone(), attempt_id.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produced_and_skipped_terminals_have_distinct_skip_frontiers() {
        let node_id = test_support::fixed_node_id_for_test(0x91);
        let produced_attempt = test_support::fixed_attempt_id_for_test(0x92);
        let skipped_attempt = test_support::fixed_attempt_id_for_test(0x93);
        let mut skipped = BTreeSet::new();

        record_attempt_skip(&mut skipped, &node_id, &produced_attempt, false);
        record_attempt_skip(&mut skipped, &node_id, &skipped_attempt, true);

        assert!(!skipped.contains(&(node_id.clone(), produced_attempt)));
        assert!(skipped.contains(&(node_id, skipped_attempt)));
    }
}
