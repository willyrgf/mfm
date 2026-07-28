//! Borrowed queries over the current persisted lifecycle algebra.
//!
//! This module is a temporary, non-Package-A bridge. It never yields raw journal slices,
//! projection maps, or owned lifecycle authority.

use super::journal::history_validation::{
    CertifiedHistoryFold, ManualResolutionBatchAuthorities, VerifiedHistoricalManualResolution,
};
use super::*;
use mfm_certify::CertifiedTypedSpec;
use std::ops::ControlFlow;

#[derive(Debug)]
pub(crate) struct CurrentLifecycleFold {
    projection: ProjectionSnapshot,
    certified_history: CertifiedHistoryFold,
}

impl CurrentLifecycleFold {
    pub(crate) fn build(
        certified: &CertifiedTypedSpec,
        journal: &CommittedRunJournal,
    ) -> Result<Self> {
        ProjectionSnapshot::validate_run_stream(journal.records())?;
        let mut projection = ProjectionSnapshot::default();
        let manual_authorities =
            extend_projection_by_batch(certified, journal, 0, &mut projection)?;
        let certified_history =
            CertifiedHistoryFold::build(certified, journal, &projection, &manual_authorities)?;
        Ok(Self {
            projection,
            certified_history,
        })
    }

    pub(crate) fn apply_suffix(
        mut self: Box<Self>,
        certified: &CertifiedTypedSpec,
        successor: &CommittedRunJournal,
        suffix_start: usize,
    ) -> Result<Box<Self>> {
        let manual_authorities =
            extend_projection_by_batch(certified, successor, suffix_start, &mut self.projection)?;
        self.certified_history.apply_suffix(
            certified,
            successor,
            suffix_start,
            &self.projection,
            &manual_authorities,
        )?;
        Ok(self)
    }

    pub(crate) fn projection(&self) -> &ProjectionSnapshot {
        &self.projection
    }

    pub(in crate::v1) fn verified_manual_resolution(
        &self,
    ) -> Option<&VerifiedHistoricalManualResolution> {
        self.certified_history.verified_manual_resolution()
    }
}

fn extend_projection_by_batch(
    certified: &CertifiedTypedSpec,
    journal: &CommittedRunJournal,
    suffix_start: usize,
    projection: &mut ProjectionSnapshot,
) -> Result<ManualResolutionBatchAuthorities> {
    let batches = journal.batches();
    let first = batches.partition_point(|batch| batch.record_range.end <= suffix_start);
    if batches
        .get(first)
        .is_some_and(|batch| batch.record_range.start != suffix_start)
    {
        return Err(StoreError::PersistedEventMismatch {
            field: "certified_history",
            message: "projection suffix splits an atomic journal batch".to_owned(),
        });
    }
    let spec = certified.validated_spec().spec();
    let terminal_policies = SideEffectTerminalPolicies::from_spec(spec)?;
    let mut manual_authorities = ManualResolutionBatchAuthorities::default();
    for batch in &batches[first..] {
        let records = &journal.records()[batch.record_range.clone()];
        if records.iter().any(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::ManualResolutionRecorded(_)
            )
        }) {
            let saga = projection.derive_saga_projection(
                journal.run_id(),
                &spec.saga,
                &terminal_policies,
            )?;
            let prefix = super::manual_resolution_authority::build_manual_prefix_authority(
                journal.run_id(),
                certified.spec_hash(),
                &spec.saga,
                batch.seq,
                &journal.records()[..batch.record_range.start],
                &saga,
            )?;
            manual_authorities.insert(batch.record_range.start, prefix)?;
        }
        projection.extend_from_committed_records(records, journal)?;
    }
    Ok(manual_authorities)
}

/// Creates a borrowed current-lifecycle reader over one verified run.
pub fn read(view: &VerifiedRunView) -> CurrentLifecycleReader<'_> {
    CurrentLifecycleReader { view }
}

/// Borrowed purpose-specific reader over one verified run.
#[derive(Debug)]
pub struct CurrentLifecycleReader<'view> {
    view: &'view VerifiedRunView,
}

impl<'view> CurrentLifecycleReader<'view> {
    /// Returns the trusted certified authority bound to this verified run.
    pub fn certified_spec(&self) -> &'view CertifiedTypedSpec {
        self.view.certified_spec()
    }

    /// Returns the current run state.
    pub fn run_state(&self) -> RunState {
        self.projection().run_state(self.view.run_id())
    }

    /// Returns the next current-format sequence for an append.
    pub fn next_sequence(&self) -> Result<StreamSeq> {
        let current = self
            .view
            .journal()
            .records()
            .last()
            .ok_or_else(|| StoreError::RunNotFound {
                run_id: self.view.run_id().clone(),
            })?
            .seq();
        current.checked_next()
    }

    /// Returns the admitted root through a private-field wrapper.
    pub fn admission(&self) -> Result<CurrentAdmissionRef<'view>> {
        let admission = super::journal::run_admission_root(self.view.journal().records())?;
        Ok(CurrentAdmissionRef { admission })
    }

    /// Returns the terminal completion, when committed.
    pub fn completion(&self) -> Option<CurrentRunCompletionRef<'view>> {
        self.projection()
            .run_completion(self.view.run_id())
            .map(|completion| CurrentRunCompletionRef { completion })
    }

    /// Returns one admitted seed cell.
    pub fn seed(&self, cell_id: &CellId) -> Result<Option<CurrentSeedRef<'view>>> {
        Ok(self
            .admission()?
            .admission
            .seed_cells
            .iter()
            .find(|seed| &seed.cell_id == cell_id)
            .map(|seed| CurrentSeedRef { seed }))
    }

    /// Returns one admitted configuration artifact by exact artifact id.
    pub fn config(&self, artifact_id: &ArtifactId) -> Result<Option<CurrentConfigRef<'view>>> {
        Ok(self
            .admission()?
            .admission
            .config_artifacts
            .iter()
            .find(|artifact| &artifact.artifact_id == artifact_id)
            .map(|artifact| CurrentConfigRef { artifact }))
    }

    /// Returns one terminal cell result.
    pub fn cell(&self, cell_id: &CellId) -> Option<CurrentCellRef<'view>> {
        self.projection()
            .cell_terminal_for_run(self.view.run_id(), cell_id)
            .map(|cell| CurrentCellRef {
                cell_id: cell_id.clone(),
                cell,
            })
    }

    /// Returns one projected state attempt.
    pub fn attempt(
        &self,
        node_id: &NodeId,
        attempt_id: &AttemptId,
    ) -> Option<CurrentAttemptRef<'view>> {
        self.projection()
            .attempt(node_id, attempt_id)
            .filter(|attempt| attempt.run_id == *self.view.run_id())
            .map(|attempt| CurrentAttemptRef { attempt })
    }

    /// Visits this run's attempts in stable node/attempt identity order.
    pub fn visit_attempts<B>(
        &self,
        mut visitor: impl FnMut(CurrentAttemptRef<'view>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for (_, attempt) in self.projection().attempts() {
            if attempt.run_id == *self.view.run_id() {
                visitor(CurrentAttemptRef { attempt })?;
            }
        }
        ControlFlow::Continue(())
    }

    /// Returns one current side-effect ledger.
    pub fn side_effect(&self, pair_id: &SideEffectPairId) -> Option<CurrentSideEffectRef<'view>> {
        self.projection()
            .side_effect_for_pair(self.view.run_id(), pair_id)
            .map(|side_effect| CurrentSideEffectRef { side_effect })
    }

    /// Visits this run's side-effect ledgers in stable pair identity order.
    pub fn visit_side_effects<B>(
        &self,
        mut visitor: impl FnMut(CurrentSideEffectRef<'view>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for (ledger_ref, side_effect) in self.projection().side_effects() {
            if ledger_ref.run_id == *self.view.run_id() {
                visitor(CurrentSideEffectRef { side_effect })?;
            }
        }
        ControlFlow::Continue(())
    }

    /// Returns one active run-local resource lane.
    pub fn resource_lane(&self, key: &ResourceLaneKey) -> Option<CurrentResourceLaneRef<'view>> {
        self.projection()
            .resource_lane(key)
            .filter(|lane| lane.holder.run_id == *self.view.run_id())
            .map(|lane| CurrentResourceLaneRef {
                key: key.clone(),
                lane,
            })
    }

    /// Visits this run's active resource lanes in stable key order.
    pub fn visit_resource_lanes<B>(
        &self,
        mut visitor: impl FnMut(CurrentResourceLaneRef<'view>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for (key, lane) in self.projection().resource_lanes() {
            if lane.holder.run_id == *self.view.run_id() {
                visitor(CurrentResourceLaneRef {
                    key: key.clone(),
                    lane,
                })?;
            }
        }
        ControlFlow::Continue(())
    }

    /// Returns one run-local certified fact descriptor.
    pub fn fact_descriptor(
        &self,
        descriptor_hash: &ContentDigest,
    ) -> Option<CurrentFactDescriptorRef<'view>> {
        self.projection()
            .fact_descriptor(descriptor_hash)
            .map(|descriptor| CurrentFactDescriptorRef { descriptor })
    }

    /// Returns one run-local fact query entry.
    pub fn fact_query_entry(
        &self,
        claim_id: &mfm_facts::FactClaimId,
    ) -> Option<CurrentFactQueryRef<'view>> {
        self.projection()
            .fact_query_entry(claim_id)
            .map(|query| CurrentFactQueryRef { query })
    }

    /// Returns one exact explicit artifact-reference record.
    pub fn artifact_reference(
        &self,
        artifact_id: &ArtifactId,
        evidence_hash: &ContentDigest,
    ) -> Option<CurrentArtifactReferenceRef<'view>> {
        self.view
            .journal()
            .records()
            .iter()
            .enumerate()
            .find_map(|(position, record)| {
                let KernelEventPayload::ArtifactReferenced(reference) = record.payload() else {
                    return None;
                };
                (&reference.artifact_ref.artifact_id == artifact_id
                    && &reference.artifact_ref.evidence_hash == evidence_hash)
                    .then_some(CurrentArtifactReferenceRef {
                        position,
                        record,
                        reference,
                    })
            })
    }

    /// Derives and lends the current saga state without returning an owned lifecycle map.
    pub fn with_saga<T>(
        &self,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
        read: impl FnOnce(CurrentSagaRef<'_>) -> T,
    ) -> Result<T> {
        let saga = self.projection().derive_saga_projection(
            self.view.run_id(),
            policy,
            terminal_policies,
        )?;
        Ok(read(CurrentSagaRef { saga: &saga }))
    }

    /// Returns the terminal completion outcome supported by the current saga state.
    pub fn saga_terminal_completion_outcome(
        &self,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<events::RunCompletionOutcome> {
        self.projection().saga_terminal_completion_outcome(
            self.view.run_id(),
            policy,
            terminal_policies,
        )
    }

    /// Mints authority for a manual-resolution append from this exact verified prefix.
    pub fn manual_resolution_prefix_authority(
        &self,
    ) -> Result<mfm_manual_auth::ManualResolutionPrefixAuthority> {
        if !self.no_open_attempts() {
            return Err(StoreError::ProjectionConflict {
                key: format!("run:{}:manual_resolution_prefix", self.view.run_id()),
                message: "manual-resolution prefix contains an open semantic attempt".to_owned(),
            });
        }
        let spec = self.certified_spec().validated_spec().spec();
        let policy = &spec.saga;
        let terminal_policies = SideEffectTerminalPolicies::from_spec(spec)?;
        let saga = self.projection().derive_saga_projection(
            self.view.run_id(),
            policy,
            &terminal_policies,
        )?;
        super::manual_resolution_authority::build_manual_prefix_authority(
            self.view.run_id(),
            self.view.spec_hash(),
            policy,
            self.next_sequence()?,
            self.view.journal().records(),
            &saga,
        )
    }

    /// Mints an opaque terminal proof from the current saga state.
    pub fn saga_terminal_proof(&self, prefix_next_seq: StreamSeq) -> Result<SagaTerminalProof> {
        let spec = self.certified_spec().validated_spec().spec();
        let policy = &spec.saga;
        let terminal_policies = SideEffectTerminalPolicies::from_spec(spec)?;
        let saga = self.projection().derive_saga_projection(
            self.view.run_id(),
            policy,
            &terminal_policies,
        )?;
        SagaTerminalProof::new(
            policy,
            &saga,
            prefix_next_seq,
            self.view.spec_hash(),
            self.view
                .current_lifecycle_fold()
                .verified_manual_resolution(),
        )
    }

    /// Returns whether this run has no open semantic attempt.
    pub fn no_open_attempts(&self) -> bool {
        self.projection()
            .open_attempt_for_run(self.view.run_id())
            .is_none()
    }

    /// Returns current retention authority, when any.
    pub fn retention(&self) -> Option<CurrentRetentionRef<'view>> {
        self.projection()
            .retention(self.view.run_id())
            .map(|retention| CurrentRetentionRef { retention })
    }

    /// Returns the current public-output outcome for one public schema.
    pub fn public_output(&self, schema_id: &SchemaId) -> Option<CurrentPublicOutputRef<'view>> {
        self.projection()
            .public_output(self.view.run_id(), schema_id)
            .map(|output| CurrentPublicOutputRef { output })
    }

    /// Visits records in committed journal order and permits an early break.
    pub fn visit_records<B>(
        &self,
        mut visitor: impl FnMut(CurrentRecordRef<'view>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for batch in self.view.journal().batches() {
            for position in batch.record_range.clone() {
                let record = &self.view.journal().records()[position];
                visitor(CurrentRecordRef { position, record })?;
            }
        }
        ControlFlow::Continue(())
    }

    /// Visits records before one previously observed zero-based position.
    pub fn visit_records_before<B>(
        &self,
        end_position: usize,
        mut visitor: impl FnMut(CurrentRecordRef<'view>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        if end_position > self.view.journal().records().len() {
            return Err(StoreError::PersistedEventMismatch {
                field: "record_position",
                message: "current lifecycle prefix position exceeds the journal".to_owned(),
            });
        }
        for (position, record) in self.view.journal().records()[..end_position]
            .iter()
            .enumerate()
        {
            if let ControlFlow::Break(value) = visitor(CurrentRecordRef { position, record }) {
                return Ok(ControlFlow::Break(value));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    fn object(
        &self,
        artifact_id: &ArtifactId,
        evidence_hash: &ContentDigest,
    ) -> Option<CurrentObjectRef<'view>> {
        self.view
            .journal()
            .object(artifact_id, evidence_hash)
            .map(|object| CurrentObjectRef { object })
    }

    /// Looks up one exact retained object after checking its event-derived requirement.
    pub fn object_for_requirement(
        &self,
        requirement: &EventArtifactRequirement,
    ) -> Option<CurrentObjectRef<'view>> {
        if !self.view.journal().requires_object(requirement) {
            return None;
        }
        let object = self.object(&requirement.artifact_id, &requirement.evidence_hash)?;
        validate_artifact_requirement_against_evidence(requirement, object.evidence())
            .ok()
            .map(|()| object)
    }

    fn projection(&self) -> &'view ProjectionSnapshot {
        self.view.current_lifecycle_fold().projection()
    }
}

/// Borrowed admitted-root fields.
#[derive(Debug)]
pub struct CurrentAdmissionRef<'view> {
    admission: &'view events::RunAdmitted,
}

impl<'view> CurrentAdmissionRef<'view> {
    /// Returns the admitted run id.
    pub fn run_id(&self) -> &'view RunId {
        &self.admission.run_id
    }

    /// Returns the admitted run identity material.
    pub fn identity_material(&self) -> &'view events::RunIdentityMaterialV1 {
        &self.admission.identity_material
    }

    /// Returns public entry-point launch evidence.
    pub fn entry_point(&self) -> &'view events::EntryPointLaunchEvidence {
        &self.admission.entry_point
    }

    /// Returns the admitted certified spec hash.
    pub fn spec_hash(&self) -> &'view SpecHash {
        &self.admission.spec_hash
    }

    /// Returns the admitted certified-spec artifact evidence.
    pub fn spec_artifact(&self) -> &'view events::RunArtifactEvidenceRef {
        &self.admission.spec_artifact
    }

    /// Returns the admitted certificate artifact evidence.
    pub fn certificate_artifact(&self) -> &'view events::RunArtifactEvidenceRef {
        &self.admission.certificate_artifact
    }

    /// Iterates admitted configuration artifacts.
    pub fn config_artifacts(
        &self,
    ) -> impl ExactSizeIterator<Item = &'view events::RunArtifactEvidenceRef> {
        self.admission.config_artifacts.iter()
    }

    /// Iterates admitted fact-descriptor artifacts.
    pub fn fact_descriptor_artifacts(
        &self,
    ) -> impl ExactSizeIterator<Item = &'view events::RunArtifactEvidenceRef> {
        self.admission.fact_descriptor_artifacts.iter()
    }

    /// Returns the public-output schema.
    pub fn public_output_schema_id(&self) -> &'view SchemaId {
        &self.admission.public_output_schema_id
    }

    /// Returns the certified saga-policy digest.
    pub fn saga_policy_digest(&self) -> &'view ContentDigest {
        &self.admission.saga_policy_digest
    }

    /// Returns the digest covering admitted executable bindings.
    pub fn admitted_binding_digest(&self) -> &'view ContentDigest {
        &self.admission.admitted_binding_digest
    }

    /// Returns the admitted canonicalizer identity.
    pub fn canonicalizer_identity(&self) -> &'view CanonicalizerIdentity {
        &self.admission.canonicalizer_identity
    }

    /// Iterates admitted runner executable identities.
    pub fn runner_executables(
        &self,
    ) -> impl ExactSizeIterator<Item = &'view events::ExecutableIdentity> {
        self.admission.runner_executables.iter()
    }

    /// Iterates admitted adapter executable identities.
    pub fn adapter_executables(
        &self,
    ) -> impl ExactSizeIterator<Item = &'view events::ExecutableIdentity> {
        self.admission.adapter_executables.iter()
    }

    /// Iterates admitted capability implementation identities.
    pub fn capability_implementations(
        &self,
    ) -> impl ExactSizeIterator<Item = &'view events::CapabilityImplementationIdentity> {
        self.admission.capability_implementations.iter()
    }

    /// Iterates admitted seed cells.
    pub fn seed_cells(&self) -> impl ExactSizeIterator<Item = &'view events::SeedCellRef> {
        self.admission.seed_cells.iter()
    }
}

/// Borrowed terminal run completion.
#[derive(Debug)]
pub struct CurrentRunCompletionRef<'view> {
    completion: &'view RunCompletionProjection,
}

impl<'view> CurrentRunCompletionRef<'view> {
    /// Returns the event that recorded completion.
    pub fn event_id(&self) -> &'view EventId {
        &self.completion.event_id
    }

    /// Returns the terminal run outcome.
    pub fn outcome(&self) -> &'view events::RunCompletionOutcome {
        &self.completion.outcome
    }
}

/// Borrowed admitted seed-cell evidence.
#[derive(Debug)]
pub struct CurrentSeedRef<'view> {
    seed: &'view events::SeedCellRef,
}

impl<'view> CurrentSeedRef<'view> {
    /// Returns the certified seed evidence.
    pub fn evidence(&self) -> &'view events::SeedCellRef {
        self.seed
    }
}

/// Borrowed admitted configuration evidence.
#[derive(Debug)]
pub struct CurrentConfigRef<'view> {
    artifact: &'view events::RunArtifactEvidenceRef,
}

impl<'view> CurrentConfigRef<'view> {
    /// Returns the exact configuration artifact evidence.
    pub fn evidence(&self) -> &'view events::RunArtifactEvidenceRef {
        self.artifact
    }
}

/// Borrowed terminal cell state.
#[derive(Debug)]
pub struct CurrentCellRef<'view> {
    cell_id: CellId,
    cell: &'view CellTerminalProjection,
}

impl<'view> CurrentCellRef<'view> {
    /// Returns the certified cell id.
    pub fn cell_id(&self) -> &CellId {
        &self.cell_id
    }

    /// Returns the event that established terminal cell state.
    pub fn event_id(&self) -> &'view EventId {
        match self.cell {
            CellTerminalProjection::Produced { event_id, .. }
            | CellTerminalProjection::Skipped { event_id, .. } => event_id,
        }
    }

    /// Returns produced-cell evidence, when produced.
    pub fn produced(&self) -> Option<CurrentProducedCellRef<'view>> {
        match self.cell {
            CellTerminalProjection::Produced {
                node_id,
                attempt_id,
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                evidence_hash,
                ..
            } => Some(CurrentProducedCellRef {
                node_id,
                attempt_id,
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                evidence_hash,
            }),
            CellTerminalProjection::Skipped { .. } => None,
        }
    }

    /// Returns skipped-cell evidence, when skipped.
    pub fn skipped(&self) -> Option<CurrentSkippedCellRef<'view>> {
        match self.cell {
            CellTerminalProjection::Skipped {
                node_id,
                attempt_id,
                schema_id,
                semantic_type_id,
                skip_reason,
                ..
            } => Some(CurrentSkippedCellRef {
                node_id,
                attempt_id,
                schema_id,
                semantic_type_id,
                skip_reason,
            }),
            CellTerminalProjection::Produced { .. } => None,
        }
    }
}

/// Borrowed produced-cell evidence.
#[derive(Debug)]
pub struct CurrentProducedCellRef<'view> {
    node_id: &'view NodeId,
    attempt_id: &'view AttemptId,
    schema_id: &'view SchemaId,
    semantic_type_id: &'view SemanticTypeId,
    artifact_id: &'view ArtifactId,
    content_digest: &'view ContentDigest,
    evidence_hash: &'view ContentDigest,
}

impl<'view> CurrentProducedCellRef<'view> {
    /// Returns the producer node.
    pub fn node_id(&self) -> &'view NodeId {
        self.node_id
    }

    /// Returns the producer attempt.
    pub fn attempt_id(&self) -> &'view AttemptId {
        self.attempt_id
    }

    /// Returns the value schema.
    pub fn schema_id(&self) -> &'view SchemaId {
        self.schema_id
    }

    /// Returns the value semantic type.
    pub fn semantic_type_id(&self) -> &'view SemanticTypeId {
        self.semantic_type_id
    }

    /// Returns the retained value artifact.
    pub fn artifact_id(&self) -> &'view ArtifactId {
        self.artifact_id
    }

    /// Returns the canonical value digest.
    pub fn content_digest(&self) -> &'view ContentDigest {
        self.content_digest
    }

    /// Returns the exact artifact evidence hash.
    pub fn evidence_hash(&self) -> &'view ContentDigest {
        self.evidence_hash
    }
}

/// Borrowed skipped-cell evidence.
#[derive(Debug)]
pub struct CurrentSkippedCellRef<'view> {
    node_id: &'view NodeId,
    attempt_id: &'view AttemptId,
    schema_id: &'view SchemaId,
    semantic_type_id: &'view SemanticTypeId,
    skip_reason: &'view events::SkipReason,
}

impl<'view> CurrentSkippedCellRef<'view> {
    /// Returns the producer node.
    pub fn node_id(&self) -> &'view NodeId {
        self.node_id
    }

    /// Returns the producer attempt.
    pub fn attempt_id(&self) -> &'view AttemptId {
        self.attempt_id
    }

    /// Returns the cell schema.
    pub fn schema_id(&self) -> &'view SchemaId {
        self.schema_id
    }

    /// Returns the cell semantic type.
    pub fn semantic_type_id(&self) -> &'view SemanticTypeId {
        self.semantic_type_id
    }

    /// Returns the typed skip reason.
    pub fn skip_reason(&self) -> &'view events::SkipReason {
        self.skip_reason
    }
}

/// Borrowed state-attempt lifecycle.
#[derive(Debug)]
pub struct CurrentAttemptRef<'view> {
    attempt: &'view AttemptProjection,
}

impl<'view> CurrentAttemptRef<'view> {
    /// Returns the owning run.
    pub fn run_id(&self) -> &'view RunId {
        &self.attempt.run_id
    }

    /// Returns the certified node.
    pub fn node_id(&self) -> &'view NodeId {
        &self.attempt.node_id
    }

    /// Returns the store-owned attempt id.
    pub fn attempt_id(&self) -> &'view AttemptId {
        &self.attempt.attempt_id
    }

    /// Returns the last event affecting this attempt.
    pub fn event_id(&self) -> &'view EventId {
        &self.attempt.event_id
    }

    /// Returns the semantic attempt status.
    pub fn status(&self) -> CurrentAttemptStatusRef<'view> {
        match &self.attempt.status {
            AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => CurrentAttemptStatusRef::Started {
                attempt_no: *attempt_no,
                state_kind,
                state_version,
            },
            AttemptStatus::Completed { output_cell_id } => {
                CurrentAttemptStatusRef::Completed { output_cell_id }
            }
            AttemptStatus::Failed { retryable, error } => CurrentAttemptStatusRef::Failed {
                retryable: *retryable,
                error,
            },
            AttemptStatus::Interrupted => CurrentAttemptStatusRef::Interrupted,
        }
    }
}

/// Borrowed semantic attempt status.
#[derive(Debug)]
pub enum CurrentAttemptStatusRef<'view> {
    /// Attempt remains active.
    Started {
        /// One-based attempt number.
        attempt_no: u32,
        /// Certified state kind.
        state_kind: &'view StateKind,
        /// Certified state version.
        state_version: &'view StateVersion,
    },
    /// Attempt completed with one output cell.
    Completed {
        /// Produced output cell.
        output_cell_id: &'view CellId,
    },
    /// Attempt failed.
    Failed {
        /// Whether retry remains allowed.
        retryable: bool,
        /// Redaction-safe failure.
        error: &'view events::MfmErrorInfo,
    },
    /// Attempt was interrupted.
    Interrupted,
}

/// Borrowed current side-effect ledger.
#[derive(Debug)]
pub struct CurrentSideEffectRef<'view> {
    side_effect: &'view SideEffectProjection,
}

impl<'view> CurrentSideEffectRef<'view> {
    /// Returns the owning run.
    pub fn run_id(&self) -> &'view RunId {
        &self.side_effect.run_id
    }

    /// Returns the diagnostic ledger key.
    pub fn ledger_key(&self) -> &'view events::SideEffectLedgerKey {
        &self.side_effect.ledger_key
    }

    /// Returns the ledger purpose.
    pub fn ledger_purpose(&self) -> &'view events::SideEffectLedgerPurpose {
        &self.side_effect.ledger_purpose
    }

    /// Returns the certified pair id.
    pub fn pair_id(&self) -> &'view SideEffectPairId {
        &self.side_effect.pair_id
    }

    /// Returns the last event affecting the ledger.
    pub fn event_id(&self) -> &'view EventId {
        &self.side_effect.event_id
    }

    /// Returns the intent that opened this ledger.
    pub fn intent(&self) -> CurrentSideEffectIntentRef<'view> {
        CurrentSideEffectIntentRef {
            intent: &self.side_effect.intent,
        }
    }

    /// Returns the current semantic phase.
    pub fn phase(&self) -> &'view SideEffectPhase {
        &self.side_effect.phase
    }

    /// Returns the validated current side-effect typestate.
    pub fn ledger_state(&self) -> Result<CurrentSideEffectLedgerState<'view>> {
        self.side_effect
            .ledger_state()
            .map(|state| CurrentSideEffectLedgerState { state })
    }

    /// Returns the active claim, when any.
    pub fn claim(&self) -> Option<CurrentSideEffectClaimRef<'view>> {
        self.side_effect
            .claim
            .as_ref()
            .map(|claim| CurrentSideEffectClaimRef { claim })
    }

    /// Returns prepared invocation evidence, when any.
    pub fn prepared_invocation(&self) -> Option<CurrentArtifactProjectionRef<'view>> {
        self.side_effect
            .prepared_invocation
            .as_ref()
            .map(|artifact| CurrentArtifactProjectionRef { artifact })
    }

    /// Returns exclusive resource-key evidence, when any.
    pub fn resource_key(&self) -> Option<&'view events::ResourceKeyEvidence> {
        self.side_effect.resource_key.as_ref()
    }

    /// Returns submission evidence, when any.
    pub fn submission(&self) -> Option<CurrentArtifactProjectionRef<'view>> {
        self.side_effect
            .submission
            .as_ref()
            .map(|artifact| CurrentArtifactProjectionRef { artifact })
    }

    /// Returns receipt evidence, when any.
    pub fn receipt(&self) -> Option<CurrentArtifactProjectionRef<'view>> {
        self.side_effect
            .receipt
            .as_ref()
            .map(|artifact| CurrentArtifactProjectionRef { artifact })
    }

    /// Returns confirmation evidence, when any.
    pub fn confirmation(&self) -> Option<CurrentArtifactProjectionRef<'view>> {
        self.side_effect
            .confirmation
            .as_ref()
            .map(|artifact| CurrentArtifactProjectionRef { artifact })
    }

    /// Returns exact resource touched-set evidence, when any.
    pub fn resource_touched_set(&self) -> Option<&'view events::ResourceTouchedSetEvidence> {
        self.side_effect.resource_touched_set.as_ref()
    }
}

/// Borrowed side-effect intent fields.
#[derive(Debug)]
pub struct CurrentSideEffectIntentRef<'view> {
    intent: &'view SideEffectIntentProjection,
}

/// Borrowed validated side-effect typestate without projection access.
#[derive(Debug)]
pub struct CurrentSideEffectLedgerState<'view> {
    state: SideEffectLedgerState<'view>,
}

impl<'view> CurrentSideEffectLedgerState<'view> {
    /// Returns the validated semantic phase.
    pub fn phase(&self) -> SideEffectLedgerPhase<'view> {
        self.state.phase()
    }

    /// Returns the ledger purpose.
    pub fn ledger_purpose(&self) -> &'view events::SideEffectLedgerPurpose {
        self.state.ledger_purpose()
    }

    /// Returns whether forward completion remains possible.
    pub fn is_forward_completion_candidate(&self) -> bool {
        self.state.is_forward_completion_candidate()
    }

    /// Returns whether the ledger is ambiguous.
    pub fn is_ambiguous(&self) -> bool {
        self.state.is_ambiguous()
    }

    /// Returns whether the ledger failed.
    pub fn is_failed(&self) -> bool {
        self.state.is_failed()
    }

    /// Returns whether confirmation was observed.
    pub fn is_confirmed(&self) -> bool {
        self.state.is_confirmed()
    }
}

impl<'view> CurrentSideEffectIntentRef<'view> {
    /// Returns the node that authored the intent.
    pub fn node_id(&self) -> &'view NodeId {
        &self.intent.node_id
    }

    /// Returns the state attempt that authored the intent.
    pub fn attempt_id(&self) -> &'view AttemptId {
        &self.intent.attempt_id
    }

    /// Returns the owning scope.
    pub fn scope_id(&self) -> &'view ScopeId {
        &self.intent.scope_id
    }

    /// Returns the invocation epoch.
    pub fn invocation_epoch(&self) -> u32 {
        self.intent.invocation_epoch
    }

    /// Returns the intent schema.
    pub fn intent_schema_id(&self) -> &'view SchemaId {
        &self.intent.intent_schema_id
    }

    /// Returns the canonical intent hash.
    pub fn intent_hash(&self) -> &'view ContentDigest {
        &self.intent.intent_hash
    }

    /// Returns the retained intent artifact.
    pub fn intent_artifact_id(&self) -> &'view ArtifactId {
        &self.intent.intent_artifact_id
    }

    /// Returns the idempotency-input schema.
    pub fn idempotency_input_schema_id(&self) -> &'view SchemaId {
        &self.intent.idempotency_input_schema_id
    }

    /// Returns the canonical idempotency-input hash.
    pub fn idempotency_input_hash(&self) -> &'view ContentDigest {
        &self.intent.idempotency_input_hash
    }

    /// Returns the certified idempotency key.
    pub fn idempotency_key(&self) -> &'view events::IdempotencyKeyRef {
        &self.intent.idempotency_key
    }

    /// Returns the selected capability kind.
    pub fn capability_kind(&self) -> &'view CapabilityKind {
        &self.intent.capability_kind
    }

    /// Returns the selected capability version.
    pub fn capability_version(&self) -> &'view CapabilityVersion {
        &self.intent.capability_version
    }

    /// Returns the selected adapter kind.
    pub fn adapter_kind(&self) -> &'view AdapterKind {
        &self.intent.adapter_kind
    }

    /// Returns the selected adapter version.
    pub fn adapter_version(&self) -> &'view AdapterVersion {
        &self.intent.adapter_version
    }
}

/// Borrowed side-effect claim fields.
#[derive(Debug)]
pub struct CurrentSideEffectClaimRef<'view> {
    claim: &'view SideEffectClaimProjection,
}

impl<'view> CurrentSideEffectClaimRef<'view> {
    /// Returns the claim owner.
    pub fn claim_owner(&self) -> &'view events::RunnerInvocationId {
        &self.claim.claim_owner
    }

    /// Returns the invocation epoch.
    pub fn invocation_epoch(&self) -> u32 {
        self.claim.invocation_epoch
    }

    /// Returns the claim generation.
    pub fn claim_generation(&self) -> u32 {
        self.claim.claim_generation
    }

    /// Returns the claim fencing token.
    pub fn claim_fencing_token(&self) -> &'view events::side_effect::ClaimFencingToken {
        &self.claim.claim_fencing_token
    }
}

/// Borrowed retained side-effect artifact fields.
#[derive(Debug)]
pub struct CurrentArtifactProjectionRef<'view> {
    artifact: &'view SideEffectArtifactProjection,
}

impl<'view> CurrentArtifactProjectionRef<'view> {
    /// Returns the retained artifact id.
    pub fn artifact_id(&self) -> &'view ArtifactId {
        &self.artifact.artifact_id
    }

    /// Returns its canonical content digest.
    pub fn content_digest(&self) -> &'view ContentDigest {
        &self.artifact.content_digest
    }

    /// Returns its exact evidence hash.
    pub fn evidence_hash(&self) -> &'view ContentDigest {
        &self.artifact.evidence_hash
    }

    /// Returns its schema, when schema-bearing.
    pub fn schema_id(&self) -> Option<&'view SchemaId> {
        self.artifact.schema_id.as_ref()
    }

    /// Returns the certified producer node.
    pub fn producer_node_id(&self) -> &'view NodeId {
        &self.artifact.producer_node_id
    }
}

/// Borrowed active resource-lane state.
#[derive(Debug)]
pub struct CurrentResourceLaneRef<'view> {
    key: ResourceLaneKey,
    lane: &'view ResourceLaneProjection,
}

impl<'view> CurrentResourceLaneRef<'view> {
    /// Returns the resource-lane key.
    pub fn key(&self) -> &ResourceLaneKey {
        &self.key
    }

    /// Returns the event that established the active holder.
    pub fn event_id(&self) -> &'view EventId {
        &self.lane.event_id
    }

    /// Returns the run-scoped holder.
    pub fn holder(&self) -> CurrentSideEffectPairRef<'view> {
        CurrentSideEffectPairRef {
            ledger_ref: &self.lane.holder,
        }
    }

    /// Returns the diagnostic ledger key.
    pub fn ledger_key(&self) -> &'view events::SideEffectLedgerKey {
        &self.lane.ledger_key
    }

    /// Returns the ledger purpose.
    pub fn ledger_purpose(&self) -> &'view events::SideEffectLedgerPurpose {
        &self.lane.ledger_purpose
    }

    /// Returns the node holding the lane.
    pub fn node_id(&self) -> &'view NodeId {
        &self.lane.node_id
    }

    /// Returns the attempt holding the lane.
    pub fn attempt_id(&self) -> &'view AttemptId {
        &self.lane.attempt_id
    }

    /// Returns the invocation epoch.
    pub fn invocation_epoch(&self) -> u32 {
        self.lane.invocation_epoch
    }

    /// Returns the lane claim id.
    pub fn claim_id(&self) -> &'view events::ResourceLaneClaimId {
        &self.lane.claim_id
    }

    /// Returns the lane-local fencing token.
    pub fn claim_fencing_token(&self) -> u64 {
        self.lane.claim_fencing_token
    }

    /// Returns the lane-local transition sequence.
    pub fn lane_transition_sequence(&self) -> u64 {
        self.lane.lane_transition_seq
    }
}

/// Borrowed run-scoped side-effect pair identity.
#[derive(Debug)]
pub struct CurrentSideEffectPairRef<'view> {
    ledger_ref: &'view SideEffectPairLedgerRef,
}

impl<'view> CurrentSideEffectPairRef<'view> {
    /// Returns the owning run.
    pub fn run_id(&self) -> &'view RunId {
        &self.ledger_ref.run_id
    }

    /// Returns the certified pair id.
    pub fn pair_id(&self) -> &'view SideEffectPairId {
        &self.ledger_ref.pair_id
    }
}

/// Borrowed run-local fact descriptor.
#[derive(Debug)]
pub struct CurrentFactDescriptorRef<'view> {
    descriptor: &'view FactDescriptorProjection,
}

impl<'view> CurrentFactDescriptorRef<'view> {
    /// Returns the canonical descriptor hash.
    pub fn descriptor_hash(&self) -> &'view ContentDigest {
        &self.descriptor.descriptor_hash
    }

    /// Returns the retained descriptor artifact.
    pub fn artifact_id(&self) -> &'view ArtifactId {
        &self.descriptor.descriptor_artifact_id
    }

    /// Returns exact retained descriptor evidence.
    pub fn artifact_evidence(&self) -> &'view ArtifactEvidenceRef {
        &self.descriptor.descriptor_artifact_evidence
    }

    /// Returns the declared fact kind.
    pub fn fact_kind(&self) -> &'view mfm_facts::FactKind {
        &self.descriptor.fact_kind
    }

    /// Returns the descriptor schema.
    pub fn descriptor_schema_id(&self) -> &'view SchemaId {
        &self.descriptor.descriptor_schema_id
    }

    /// Returns the subject schema.
    pub fn subject_schema_id(&self) -> &'view SchemaId {
        &self.descriptor.subject_schema_id
    }

    /// Returns the response schema.
    pub fn response_schema_id(&self) -> &'view SchemaId {
        &self.descriptor.response_schema_id
    }

    /// Returns the descriptor-derived subject namespace.
    pub fn subject_namespace_hash(&self) -> &'view ContentDigest {
        &self.descriptor.fact_subject_namespace_hash
    }
}

/// Borrowed run-local fact query evidence.
#[derive(Debug)]
pub struct CurrentFactQueryRef<'view> {
    query: &'view FactQueryProjection,
}

impl<'view> CurrentFactQueryRef<'view> {
    /// Returns the derived fact claim id.
    pub fn claim_id(&self) -> &'view mfm_facts::FactClaimId {
        &self.query.fact_claim_id
    }

    /// Returns the source run.
    pub fn source_run_id(&self) -> &'view RunId {
        &self.query.source_run_id
    }

    /// Returns the source run sequence.
    pub fn source_sequence(&self) -> u64 {
        self.query.source_seq
    }

    /// Returns the source ordinal.
    pub fn source_ordinal(&self) -> u32 {
        self.query.source_ordinal
    }

    /// Returns the source event.
    pub fn source_event_id(&self) -> &'view EventId {
        &self.query.source_event_id
    }

    /// Returns the producer node.
    pub fn producer_node_id(&self) -> &'view NodeId {
        &self.query.producer_node_id
    }

    /// Returns the producing attempt.
    pub fn attempt_id(&self) -> &'view AttemptId {
        &self.query.attempt_id
    }

    /// Returns the atomic commit key.
    pub fn commit_key(&self) -> &'view CommitKey {
        &self.query.commit_id
    }

    /// Returns the store-wide commit order.
    pub fn store_commit_order(&self) -> u64 {
        self.query.store_commit_order
    }

    /// Reconstructs the public internal fact reference.
    pub fn internal_ref(&self) -> Result<mfm_facts::InternalFactRef> {
        self.query.internal_ref()
    }

    /// Returns exact response artifact evidence, when retained.
    pub fn response_artifact_evidence(&self) -> Option<&'view ArtifactEvidenceRef> {
        self.query.response_artifact_evidence.as_ref()
    }

    /// Visits descriptor-derived query terms in field-id order.
    pub fn visit_terms<B>(
        &self,
        mut visitor: impl FnMut(&'view mfm_facts::FactQueryTerm) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for term in self.query.terms.values() {
            visitor(term)?;
        }
        ControlFlow::Continue(())
    }
}

/// Borrowed explicit artifact-reference record.
pub struct CurrentArtifactReferenceRef<'view> {
    position: usize,
    record: &'view KernelEventEnvelope,
    reference: &'view events::ArtifactReferenced,
}

impl fmt::Debug for CurrentArtifactReferenceRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentArtifactReferenceRef")
            .field("position", &self.position)
            .field("event_id", self.record.event_id())
            .field("artifact_id", &self.reference.artifact_ref.artifact_id)
            .finish_non_exhaustive()
    }
}

impl<'view> CurrentArtifactReferenceRef<'view> {
    /// Returns controlled record metadata.
    pub fn record(&self) -> CurrentRecordRef<'view> {
        CurrentRecordRef {
            position: self.position,
            record: self.record,
        }
    }

    /// Returns the optional producer node.
    pub fn node_id(&self) -> Option<&'view NodeId> {
        self.reference.node_id.as_ref()
    }

    /// Returns the optional producer attempt.
    pub fn attempt_id(&self) -> Option<&'view AttemptId> {
        self.reference.attempt_id.as_ref()
    }

    /// Returns exact artifact evidence.
    pub fn evidence(&self) -> &'view events::ArtifactEvidenceRef {
        &self.reference.artifact_ref
    }
}

/// Scoped borrowed derived saga result with private fields.
#[derive(Debug)]
pub struct CurrentSagaRef<'view> {
    saga: &'view SagaProjection,
}

impl CurrentSagaRef<'_> {
    /// Returns the run described by this result.
    pub fn run_id(&self) -> &RunId {
        &self.saga.run_id
    }

    /// Returns the derived run mode.
    pub fn run_mode(&self) -> RunMode {
        self.saga.run_mode
    }

    /// Returns whether all forward ledgers are quiescent.
    pub fn forward_quiescent(&self) -> bool {
        self.saga.forward_quiescent
    }

    /// Returns the derived manual block reason.
    pub fn manual_block_reason(&self) -> Option<ManualBlockReason> {
        self.saga.manual_block_reason
    }

    /// Returns first saga engagement, when any.
    pub fn engagement(&self) -> Option<CurrentSagaEngagementRef<'_>> {
        self.saga
            .engagement
            .as_ref()
            .map(|engagement| CurrentSagaEngagementRef { engagement })
    }

    /// Returns one derived forward obligation.
    pub fn obligation(&self, pair_id: &SideEffectPairId) -> Option<CurrentSagaObligationRef<'_>> {
        self.saga
            .obligations
            .get(pair_id)
            .map(|obligation| CurrentSagaObligationRef { obligation })
    }

    /// Visits derived obligations in certified pair order.
    pub fn visit_obligations<B>(
        &self,
        mut visitor: impl FnMut(CurrentSagaObligationRef<'_>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for obligation in self.saga.obligations.values() {
            visitor(CurrentSagaObligationRef { obligation })?;
        }
        ControlFlow::Continue(())
    }

    /// Returns committed manual resolution, when any.
    pub fn manual_resolution(&self) -> Option<CurrentManualResolutionRef<'_>> {
        self.saga
            .manual_resolution
            .as_ref()
            .map(|resolution| CurrentManualResolutionRef { resolution })
    }

    /// Returns committed run completion, when any.
    pub fn completion(&self) -> Option<CurrentRunCompletionRef<'_>> {
        self.saga
            .run_completion
            .as_ref()
            .map(|completion| CurrentRunCompletionRef { completion })
    }
}

/// Borrowed first saga engagement.
#[derive(Debug)]
pub struct CurrentSagaEngagementRef<'view> {
    engagement: &'view SagaEngagementProjection,
}

impl<'view> CurrentSagaEngagementRef<'view> {
    /// Returns the engaging event.
    pub fn event_id(&self) -> &'view EventId {
        &self.engagement.event_id
    }

    /// Returns the engagement reason.
    pub fn reason(&self) -> &'view SagaEngagementReason {
        &self.engagement.reason
    }
}

/// Borrowed derived saga obligation.
#[derive(Debug)]
pub struct CurrentSagaObligationRef<'view> {
    obligation: &'view SagaObligationProjection,
}

impl<'view> CurrentSagaObligationRef<'view> {
    /// Returns the forward ledger key.
    pub fn forward_ledger_key(&self) -> &'view events::SideEffectLedgerKey {
        &self.obligation.forward_ledger_key
    }

    /// Returns the forward pair id.
    pub fn forward_pair_id(&self) -> &'view SideEffectPairId {
        &self.obligation.forward_pair_id
    }

    /// Returns the current forward phase.
    pub fn forward_phase(&self) -> &'view SideEffectPhase {
        &self.obligation.forward_phase
    }

    /// Returns the derived classification.
    pub fn classification(&self) -> ForwardLedgerClassification {
        self.obligation.classification
    }

    /// Returns linked remediation state, when any.
    pub fn remediation(&self) -> Option<CurrentRemediationRef<'view>> {
        self.obligation
            .remediation
            .as_ref()
            .map(|remediation| CurrentRemediationRef { remediation })
    }
}

/// Borrowed linked remediation state.
#[derive(Debug)]
pub struct CurrentRemediationRef<'view> {
    remediation: &'view RemediationLedgerProjection,
}

impl<'view> CurrentRemediationRef<'view> {
    /// Returns the remediation ledger key.
    pub fn ledger_key(&self) -> &'view events::SideEffectLedgerKey {
        &self.remediation.ledger_key
    }

    /// Returns the remediation pair id.
    pub fn pair_id(&self) -> &'view SideEffectPairId {
        &self.remediation.pair_id
    }

    /// Returns the current remediation phase.
    pub fn phase(&self) -> &'view SideEffectPhase {
        &self.remediation.phase
    }

    /// Returns whether the obligation is closed.
    pub fn closed(&self) -> bool {
        self.remediation.closed
    }

    /// Returns a terminal unresolved reason.
    pub fn unresolved(&self) -> Option<ManualBlockReason> {
        self.remediation.unresolved
    }
}

/// Borrowed committed manual-resolution evidence.
#[derive(Debug)]
pub struct CurrentManualResolutionRef<'view> {
    resolution: &'view ManualResolutionProjection,
}

impl<'view> CurrentManualResolutionRef<'view> {
    /// Returns the recording event.
    pub fn event_id(&self) -> &'view EventId {
        &self.resolution.event_id
    }

    /// Returns the operator-selected outcome.
    pub fn outcome(&self) -> &'view events::ManualResolutionOutcome {
        &self.resolution.outcome
    }

    /// Returns the evidence artifact id.
    pub fn evidence_artifact_id(&self) -> &'view ArtifactId {
        &self.resolution.evidence_artifact_id
    }

    /// Returns the authorization artifact id.
    pub fn authorization_artifact_id(&self) -> &'view ArtifactId {
        &self.resolution.authorization_artifact_id
    }

    /// Returns the optional redaction-safe note.
    pub fn note(&self) -> Option<&'view events::ManualResolutionNote> {
        self.resolution.note.as_ref()
    }
}

/// Borrowed retention authority.
#[derive(Debug)]
pub struct CurrentRetentionRef<'view> {
    retention: &'view RetentionProjection,
}

impl<'view> CurrentRetentionRef<'view> {
    /// Returns the number of exact retained references.
    pub fn reference_count(&self) -> usize {
        self.retention.refs.len()
    }

    /// Visits exact retained references in authority-key order.
    pub fn visit_references<B>(
        &self,
        mut visitor: impl FnMut(&'view events::RetentionRef) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for reference in self.retention.refs.values() {
            visitor(reference)?;
        }
        ControlFlow::Continue(())
    }

    /// Returns the current projected manifest.
    pub fn current_manifest(&self) -> Option<CurrentRetentionManifestRef<'view>> {
        self.retention
            .manifest
            .as_ref()
            .map(|manifest| CurrentRetentionManifestRef { manifest })
    }

    /// Visits projected manifests in manifest-sequence order.
    pub fn visit_manifests<B>(
        &self,
        mut visitor: impl FnMut(CurrentRetentionManifestRef<'view>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for manifest in self.retention.manifests.values() {
            visitor(CurrentRetentionManifestRef { manifest })?;
        }
        ControlFlow::Continue(())
    }
}

/// Borrowed projected retention manifest.
#[derive(Debug)]
pub struct CurrentRetentionManifestRef<'view> {
    manifest: &'view RetentionManifestProjection,
}

impl<'view> CurrentRetentionManifestRef<'view> {
    /// Returns the manifest sequence.
    pub fn sequence(&self) -> u64 {
        self.manifest.manifest_seq
    }

    /// Returns the manifest digest.
    pub fn digest(&self) -> &'view ContentDigest {
        &self.manifest.manifest_digest
    }

    /// Returns the predecessor manifest digest.
    pub fn previous_digest(&self) -> Option<&'view ContentDigest> {
        self.manifest.previous_manifest_digest.as_ref()
    }

    /// Returns the retained manifest artifact.
    pub fn artifact_id(&self) -> &'view ArtifactId {
        &self.manifest.manifest_artifact_id
    }
}

/// Borrowed public-output outcome.
#[derive(Debug)]
pub struct CurrentPublicOutputRef<'view> {
    output: &'view PublicOutputProjection,
}

impl<'view> CurrentPublicOutputRef<'view> {
    /// Returns the event that established this outcome.
    pub fn event_id(&self) -> &'view EventId {
        match self.output {
            PublicOutputProjection::Produced { event_id, .. }
            | PublicOutputProjection::RenderFailed { event_id, .. } => event_id,
        }
    }

    /// Returns produced output evidence, when rendering succeeded.
    pub fn produced(&self) -> Option<CurrentProducedPublicOutputRef<'view>> {
        match self.output {
            PublicOutputProjection::Produced {
                rendered_digest,
                rendered_artifact_id,
                ..
            } => Some(CurrentProducedPublicOutputRef {
                rendered_digest,
                rendered_artifact_id: rendered_artifact_id.as_ref(),
            }),
            PublicOutputProjection::RenderFailed { .. } => None,
        }
    }

    /// Returns redaction-safe rendering failure, when rendering failed.
    pub fn render_failure(&self) -> Option<&'view events::MfmErrorInfo> {
        match self.output {
            PublicOutputProjection::RenderFailed { error, .. } => Some(error),
            PublicOutputProjection::Produced { .. } => None,
        }
    }
}

/// Borrowed successfully rendered public output.
#[derive(Debug)]
pub struct CurrentProducedPublicOutputRef<'view> {
    rendered_digest: &'view ContentDigest,
    rendered_artifact_id: Option<&'view ArtifactId>,
}

impl<'view> CurrentProducedPublicOutputRef<'view> {
    /// Returns the canonical rendered digest.
    pub fn rendered_digest(&self) -> &'view ContentDigest {
        self.rendered_digest
    }

    /// Returns the optional retained rendered artifact.
    pub fn rendered_artifact_id(&self) -> Option<&'view ArtifactId> {
        self.rendered_artifact_id
    }
}

/// One borrowed retained object.
pub struct CurrentObjectRef<'view> {
    object: &'view super::journal::VerifiedJournalObject,
}

impl fmt::Debug for CurrentObjectRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentObjectRef")
            .field("artifact_id", &self.object.evidence.artifact_id)
            .field("byte_len", &self.object.bytes.len())
            .finish_non_exhaustive()
    }
}

impl<'view> CurrentObjectRef<'view> {
    /// Returns the exact verified object bytes.
    pub fn bytes(&self) -> &'view [u8] {
        &self.object.bytes
    }

    /// Returns the exact verified object evidence.
    pub fn evidence(&self) -> &'view ArtifactEvidenceRef {
        &self.object.evidence
    }
}

/// Borrowed committed-record metadata plus a closed typed kind.
pub struct CurrentRecordRef<'view> {
    position: usize,
    record: &'view KernelEventEnvelope,
}

impl fmt::Debug for CurrentRecordRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentRecordRef")
            .field("position", &self.position)
            .field("event_id", self.record.event_id())
            .field("run_id", self.record.run_id())
            .field("sequence", &self.record.seq())
            .field("ordinal", &self.record.ordinal())
            .finish_non_exhaustive()
    }
}

impl<'view> CurrentRecordRef<'view> {
    /// Returns the zero-based stable position in this journal load.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Returns the store-derived event id.
    pub fn event_id(&self) -> &'view EventId {
        self.record.event_id()
    }

    /// Returns the event schema id.
    pub fn event_schema_id(&self) -> &'view SchemaId {
        self.record.event_schema_id()
    }

    /// Returns the admitted run id.
    pub fn run_id(&self) -> &'view RunId {
        self.record.run_id()
    }

    /// Returns the current persisted run sequence.
    pub fn sequence(&self) -> StreamSeq {
        self.record.seq()
    }

    /// Returns the store-wide commit order.
    pub fn store_commit_order(&self) -> StoreCommitOrder {
        self.record.store_commit_order()
    }

    /// Returns the ordinal inside the atomic commit.
    pub fn ordinal(&self) -> CommitOrdinal {
        self.record.ordinal()
    }

    /// Returns the certified spec hash.
    pub fn spec_hash(&self) -> &'view SpecHash {
        self.record.spec_hash()
    }

    /// Returns the commit key.
    pub fn commit_key(&self) -> &'view CommitKey {
        self.record.commit_key()
    }

    /// Returns the store-derived logical key.
    pub fn logical_key(&self) -> &'view LogicalEventKey {
        self.record.logical_key()
    }

    /// Returns the canonical payload hash.
    pub fn payload_hash(&self) -> &'view ContentDigest {
        self.record.payload_hash()
    }

    /// Returns the closed borrowed current-lifecycle kind.
    pub fn kind(&self) -> CurrentRecordKindRef<'view> {
        CurrentRecordKindRef::from_payload(self.record.payload())
    }

    /// Visits exact retained-object requirements derived from this tagged record.
    ///
    /// A selected requirement can be copied and passed unchanged to
    /// [`CurrentLifecycleReader::object_for_requirement`]. This does not expose generic key-based
    /// object browsing or weaken full requirement equality.
    pub fn visit_artifact_requirements<B>(
        &self,
        mut visitor: impl FnMut(&EventArtifactRequirement) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        for requirement in event_artifact_requirements(self.record.payload()) {
            visitor(&requirement)?;
        }
        ControlFlow::Continue(())
    }
}

/// Closed borrowed kinds from the current persisted lifecycle.
#[derive(Debug)]
pub enum CurrentRecordKindRef<'view> {
    /// Run admission.
    RunAdmitted(&'view events::RunAdmitted),
    /// State attempt start.
    StateAttemptStarted(&'view events::StateAttemptStarted),
    /// Recorded fact.
    FactRecorded(&'view events::FactRecorded),
    /// Artifact reference.
    ArtifactReferenced(&'view events::ArtifactReferenced),
    /// Produced cell.
    CellProduced(&'view events::CellProduced),
    /// Skipped cell.
    CellSkipped(&'view events::CellSkipped),
    /// Side-effect intent.
    SideEffectIntentPersisted(&'view events::side_effect::IntentPersisted),
    /// Side-effect claim.
    SideEffectClaimed(&'view events::side_effect::Claimed),
    /// Side-effect claim takeover.
    SideEffectClaimTakenOver(&'view events::side_effect::ClaimTakenOver),
    /// Resource-lane claim.
    ResourceLaneClaimed(&'view events::ResourceLaneClaimed),
    /// Legacy prepared resource-lane claim.
    ResourceLaneClaimIntent(&'view events::ResourceLaneClaimIntent),
    /// Prepared side-effect invocation.
    SideEffectInvocationPrepared(&'view events::side_effect::InvocationPrepared),
    /// Started side-effect invocation.
    SideEffectInvocationStarted(&'view events::side_effect::InvocationStarted),
    /// Proven non-submission.
    SideEffectNotSubmittedProven(&'view events::side_effect::NotSubmittedProven),
    /// Observed submission.
    SideEffectSubmissionObserved(&'view events::side_effect::SubmissionObserved),
    /// Unknown submission.
    SideEffectSubmissionUnknown(&'view events::side_effect::SubmissionUnknown),
    /// Observed receipt.
    SideEffectReceiptObserved(&'view events::side_effect::ReceiptObserved),
    /// Observed confirmation.
    SideEffectConfirmationObserved(&'view events::side_effect::ConfirmationObserved),
    /// Ambiguous side effect.
    SideEffectAmbiguous(&'view events::side_effect::Ambiguous),
    /// Failed side effect.
    SideEffectFailed(&'view events::side_effect::Failed),
    /// Resource-lane release.
    ResourceLaneReleased(&'view events::ResourceLaneReleased),
    /// Legacy prepared resource-lane release.
    ResourceLaneReleaseIntent(&'view events::ResourceLaneReleaseIntent),
    /// Produced public output.
    PublicOutputProduced(&'view events::PublicOutputProduced),
    /// Failed public-output rendering.
    PublicOutputRenderFailed(&'view events::PublicOutputRenderFailed),
    /// Completed state attempt.
    StateAttemptCompleted(&'view events::StateAttemptCompleted),
    /// Interrupted state attempt.
    StateAttemptInterrupted(&'view events::StateAttemptInterrupted),
    /// Failed state attempt.
    StateAttemptFailed(&'view events::StateAttemptFailed),
    /// Manual resolution.
    ManualResolutionRecorded(&'view events::ManualResolutionRecorded),
    /// Run completion.
    RunCompleted(&'view events::RunCompleted),
    /// Retention references.
    RetentionRefsAppended(&'view events::RetentionRefsAppended),
    /// Retention manifest.
    RetentionManifestProjected(&'view events::RetentionManifestProjected),
}

impl<'view> CurrentRecordKindRef<'view> {
    fn from_payload(payload: &'view KernelEventPayload) -> Self {
        match payload {
            KernelEventPayload::RunAdmitted(value) => Self::RunAdmitted(value),
            KernelEventPayload::StateAttemptStarted(value) => Self::StateAttemptStarted(value),
            KernelEventPayload::FactRecorded(value) => Self::FactRecorded(value),
            KernelEventPayload::ArtifactReferenced(value) => Self::ArtifactReferenced(value),
            KernelEventPayload::CellProduced(value) => Self::CellProduced(value),
            KernelEventPayload::CellSkipped(value) => Self::CellSkipped(value),
            KernelEventPayload::SideEffectIntentPersisted(value) => {
                Self::SideEffectIntentPersisted(value)
            }
            KernelEventPayload::SideEffectClaimed(value) => Self::SideEffectClaimed(value),
            KernelEventPayload::SideEffectClaimTakenOver(value) => {
                Self::SideEffectClaimTakenOver(value)
            }
            KernelEventPayload::ResourceLaneClaimed(value) => Self::ResourceLaneClaimed(value),
            KernelEventPayload::ResourceLaneClaimIntent(value) => {
                Self::ResourceLaneClaimIntent(value)
            }
            KernelEventPayload::SideEffectInvocationPrepared(value) => {
                Self::SideEffectInvocationPrepared(value)
            }
            KernelEventPayload::SideEffectInvocationStarted(value) => {
                Self::SideEffectInvocationStarted(value)
            }
            KernelEventPayload::SideEffectNotSubmittedProven(value) => {
                Self::SideEffectNotSubmittedProven(value)
            }
            KernelEventPayload::SideEffectSubmissionObserved(value) => {
                Self::SideEffectSubmissionObserved(value)
            }
            KernelEventPayload::SideEffectSubmissionUnknown(value) => {
                Self::SideEffectSubmissionUnknown(value)
            }
            KernelEventPayload::SideEffectReceiptObserved(value) => {
                Self::SideEffectReceiptObserved(value)
            }
            KernelEventPayload::SideEffectConfirmationObserved(value) => {
                Self::SideEffectConfirmationObserved(value)
            }
            KernelEventPayload::SideEffectAmbiguous(value) => Self::SideEffectAmbiguous(value),
            KernelEventPayload::SideEffectFailed(value) => Self::SideEffectFailed(value),
            KernelEventPayload::ResourceLaneReleased(value) => Self::ResourceLaneReleased(value),
            KernelEventPayload::ResourceLaneReleaseIntent(value) => {
                Self::ResourceLaneReleaseIntent(value)
            }
            KernelEventPayload::PublicOutputProduced(value) => Self::PublicOutputProduced(value),
            KernelEventPayload::PublicOutputRenderFailed(value) => {
                Self::PublicOutputRenderFailed(value)
            }
            KernelEventPayload::StateAttemptCompleted(value) => Self::StateAttemptCompleted(value),
            KernelEventPayload::StateAttemptInterrupted(value) => {
                Self::StateAttemptInterrupted(value)
            }
            KernelEventPayload::StateAttemptFailed(value) => Self::StateAttemptFailed(value),
            KernelEventPayload::ManualResolutionRecorded(value) => {
                Self::ManualResolutionRecorded(value)
            }
            KernelEventPayload::RunCompleted(value) => Self::RunCompleted(value),
            KernelEventPayload::RetentionRefsAppended(value) => Self::RetentionRefsAppended(value),
            KernelEventPayload::RetentionManifestProjected(value) => {
                Self::RetentionManifestProjected(value)
            }
        }
    }
}
