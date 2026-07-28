use super::*;
use std::ops::ControlFlow;

#[path = "broker/artifacts.rs"]
mod artifacts;
#[path = "broker/facts.rs"]
mod facts;
#[path = "broker/queries.rs"]
mod queries;
#[path = "broker/side_effects.rs"]
mod side_effects;

pub(super) struct ReplayProducedCellRecordRef<'a> {
    pub(super) sequence: store::StreamSeq,
    pub(super) commit_key: &'a store::CommitKey,
    pub(super) payload: &'a events::CellProduced,
}

pub(super) struct ReplayFactRecordRef<'a> {
    pub(super) event_id: &'a mfm_ids::EventId,
    pub(super) sequence: store::StreamSeq,
    pub(super) ordinal: store::CommitOrdinal,
    pub(super) commit_key: &'a store::CommitKey,
    pub(super) payload: &'a events::FactRecorded,
}

pub(super) enum ReplayFactEventRef<'a> {
    Primary(ReplayFactRecordRef<'a>),
    Source(&'a RetainedSourceFactReplayEvent),
}

impl ReplayFactEventRef<'_> {
    pub(super) fn event_id(&self) -> &mfm_ids::EventId {
        match self {
            Self::Primary(record) => record.event_id,
            Self::Source(source) => source.event_id(),
        }
    }

    pub(super) fn payload(&self) -> &events::FactRecorded {
        match self {
            Self::Primary(record) => record.payload,
            Self::Source(source) => source.payload(),
        }
    }
}

#[derive(Default)]
pub(super) struct ReplaySideEffectRecords<'a> {
    pub(super) prepared: Option<&'a side_effect::InvocationPrepared>,
    pub(super) submission: Option<&'a side_effect::SubmissionObserved>,
    pub(super) submission_unknown: Option<&'a side_effect::SubmissionUnknown>,
    pub(super) not_submitted: Option<&'a side_effect::NotSubmittedProven>,
    pub(super) receipt: Option<&'a side_effect::ReceiptObserved>,
    pub(super) confirmation: Option<&'a side_effect::ConfirmationObserved>,
    pub(super) ambiguity: Option<&'a side_effect::Ambiguous>,
}

impl<'view> ReplayBroker<'view> {
    /// Builds a replay broker from borrowing replay authority.
    pub fn from_read_authority(authority: ReplayReadAuthority<'view>) -> Result<Self> {
        validate_additional_artifacts(&authority.additional_artifacts)?;
        validate_source_fact_events(&authority.source_fact_events)?;
        let broker = Self {
            view: authority.view,
            additional_artifacts: authority.additional_artifacts,
            source_fact_events: authority.source_fact_events,
        };
        broker.validate_recorded_replay_evidence()?;
        Ok(broker)
    }

    pub(super) fn node(&self, node_id: &NodeId) -> Result<&spec::NodeSpec> {
        self.certified_spec()
            .spec
            .nodes
            .iter()
            .chain(self.certified_spec().spec.remediations.values())
            .find(|node| &node.node_id == node_id)
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    format!("node {node_id} is absent from the certified spec"),
                )
            })
    }

    pub(super) fn cell(&self, cell_id: &mfm_ids::CellId) -> Result<&spec::CellSpec> {
        self.certified_spec()
            .spec
            .cells
            .iter()
            .find(|cell| &cell.cell_id == cell_id)
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    format!("cell {cell_id} is absent from the certified spec"),
                )
            })
    }

    pub(super) fn side_effect_records_for_request(
        &self,
        request: &SideEffectEvidenceReplayRequest,
    ) -> Result<ReplaySideEffectRecords<'_>> {
        self.side_effect_records_for_pair(&request.pair_id, request.invocation_epoch)
    }

    fn validate_recorded_replay_evidence(&self) -> Result<()> {
        for reference in self.artifact_references()? {
            if reference.artifact_ref.role == ArtifactRole::FactQueryEvidence {
                self.verify_fact_query_evidence_reference(reference)?;
            }
        }
        Ok(())
    }

    pub(super) fn artifact_references(&self) -> Result<Vec<&events::ArtifactReferenced>> {
        let mut references = Vec::new();
        let _ = store::current_lifecycle::read(self.view).visit_records(|record| {
            if let store::current_lifecycle::CurrentRecordKindRef::ArtifactReferenced(reference) =
                record.kind()
            {
                references.push(reference);
            }
            ControlFlow::<()>::Continue(())
        });
        Ok(references)
    }

    pub(super) fn retention_references(&self) -> Result<Vec<&events::RetentionRef>> {
        let mut references = Vec::new();
        let _ = store::current_lifecycle::read(self.view).visit_records(|record| {
            if let store::current_lifecycle::CurrentRecordKindRef::RetentionRefsAppended(
                retention,
            ) = record.kind()
            {
                references.extend(retention.refs.iter());
            }
            ControlFlow::<()>::Continue(())
        });
        Ok(references)
    }

    pub(super) fn produced_cell_records(&self) -> Result<Vec<ReplayProducedCellRecordRef<'_>>> {
        let mut records = Vec::new();
        let _ = store::current_lifecycle::read(self.view).visit_records(|record| {
            if let store::current_lifecycle::CurrentRecordKindRef::CellProduced(payload) =
                record.kind()
            {
                records.push(ReplayProducedCellRecordRef {
                    sequence: record.sequence(),
                    commit_key: record.commit_key(),
                    payload,
                });
            }
            ControlFlow::<()>::Continue(())
        });
        Ok(records)
    }

    pub(super) fn fact_records(&self) -> Result<Vec<ReplayFactRecordRef<'_>>> {
        let mut records = Vec::new();
        let _ = store::current_lifecycle::read(self.view).visit_records(|record| {
            if let store::current_lifecycle::CurrentRecordKindRef::FactRecorded(payload) =
                record.kind()
            {
                records.push(ReplayFactRecordRef {
                    event_id: record.event_id(),
                    sequence: record.sequence(),
                    ordinal: record.ordinal(),
                    commit_key: record.commit_key(),
                    payload,
                });
            }
            ControlFlow::<()>::Continue(())
        });
        Ok(records)
    }

    pub(super) fn side_effect_intents(&self) -> Result<Vec<&side_effect::IntentPersisted>> {
        let mut intents = Vec::new();
        let _ = store::current_lifecycle::read(self.view).visit_records(|record| {
            if let store::current_lifecycle::CurrentRecordKindRef::SideEffectIntentPersisted(
                intent,
            ) = record.kind()
            {
                intents.push(intent);
            }
            ControlFlow::<()>::Continue(())
        });
        Ok(intents)
    }

    pub(super) fn side_effect_records_for_pair(
        &self,
        pair_id: &SideEffectPairId,
        invocation_epoch: u32,
    ) -> Result<ReplaySideEffectRecords<'_>> {
        let mut matched = ReplaySideEffectRecords::default();
        let flow =
            store::current_lifecycle::read(self.view).visit_records(|record| match record.kind() {
                store::current_lifecycle::CurrentRecordKindRef::SideEffectInvocationPrepared(
                    payload,
                ) if payload.pair_id == *pair_id
                    && payload.invocation_epoch == invocation_epoch =>
                {
                    set_side_effect_record(
                        &mut matched.prepared,
                        payload,
                        "prepared invocation",
                        pair_id,
                    )
                }
                store::current_lifecycle::CurrentRecordKindRef::SideEffectSubmissionObserved(
                    payload,
                ) if payload.pair_id == *pair_id
                    && payload.invocation_epoch == invocation_epoch =>
                {
                    set_side_effect_record(&mut matched.submission, payload, "submission", pair_id)
                }
                store::current_lifecycle::CurrentRecordKindRef::SideEffectSubmissionUnknown(
                    payload,
                ) if payload.pair_id == *pair_id
                    && payload.invocation_epoch == invocation_epoch =>
                {
                    set_side_effect_record(
                        &mut matched.submission_unknown,
                        payload,
                        "submission-unknown evidence",
                        pair_id,
                    )
                }
                store::current_lifecycle::CurrentRecordKindRef::SideEffectNotSubmittedProven(
                    payload,
                ) if payload.pair_id == *pair_id
                    && payload.invocation_epoch == invocation_epoch =>
                {
                    set_side_effect_record(
                        &mut matched.not_submitted,
                        payload,
                        "not-submitted proof",
                        pair_id,
                    )
                }
                store::current_lifecycle::CurrentRecordKindRef::SideEffectReceiptObserved(
                    payload,
                ) if payload.pair_id == *pair_id
                    && payload.invocation_epoch == invocation_epoch =>
                {
                    set_side_effect_record(&mut matched.receipt, payload, "receipt", pair_id)
                }
                store::current_lifecycle::CurrentRecordKindRef::SideEffectConfirmationObserved(
                    payload,
                ) if payload.pair_id == *pair_id
                    && payload.invocation_epoch == invocation_epoch =>
                {
                    set_side_effect_record(
                        &mut matched.confirmation,
                        payload,
                        "confirmation",
                        pair_id,
                    )
                }
                store::current_lifecycle::CurrentRecordKindRef::SideEffectAmbiguous(payload)
                    if payload.pair_id == *pair_id
                        && payload.invocation_epoch == invocation_epoch =>
                {
                    set_side_effect_record(&mut matched.ambiguity, payload, "ambiguity", pair_id)
                }
                _ => ControlFlow::Continue(()),
            });
        match flow {
            ControlFlow::Continue(()) => Ok(matched),
            ControlFlow::Break(error) => Err(error),
        }
    }

    pub(super) fn side_effect_intent(
        &self,
        pair_id: &SideEffectPairId,
    ) -> Result<&side_effect::IntentPersisted> {
        let lifecycle = store::current_lifecycle::read(self.view);
        let projected = lifecycle.side_effect(pair_id).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::SideEffectMissing,
                format!("missing side-effect intent {pair_id}"),
            )
        })?;
        let projected_epoch = projected.intent().invocation_epoch();
        let mut found = None;
        let flow = lifecycle.visit_records(|record| {
            let store::current_lifecycle::CurrentRecordKindRef::SideEffectIntentPersisted(intent) =
                record.kind()
            else {
                return ControlFlow::Continue(());
            };
            if intent.pair_id != *pair_id || intent.invocation_epoch != projected_epoch {
                return ControlFlow::Continue(());
            }
            if found.is_some() {
                return ControlFlow::Break(ReplayError::new(
                    ReplayErrorKind::InvalidRunJournal,
                    format!("duplicate side-effect intent {pair_id}"),
                ));
            }
            found = Some(intent);
            ControlFlow::Continue(())
        });
        if let ControlFlow::Break(error) = flow {
            return Err(error);
        }
        found.ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::InvalidRunJournal,
                format!("projected side-effect {pair_id} lacks its committed intent record"),
            )
        })
    }

    pub(super) fn fact_record_for_claim(
        &self,
        fact_claim_id: &mfm_facts::FactClaimId,
    ) -> Result<Option<ReplayFactEventRef<'_>>> {
        for record in self.fact_records()? {
            let candidate = mfm_facts::derive_fact_claim_id(
                self.view.run_id().clone(),
                record.sequence.as_u64(),
                record.ordinal.as_u32(),
            )
            .map_err(|error| {
                ReplayError::new(ReplayErrorKind::InvalidRunJournal, error.to_string())
            })?;
            if &candidate == fact_claim_id {
                return Ok(Some(ReplayFactEventRef::Primary(record)));
            }
        }
        for source in &self.source_fact_events {
            if source.fact_claim_id() == fact_claim_id {
                return Ok(Some(ReplayFactEventRef::Source(source)));
            }
        }
        Ok(None)
    }
}

fn set_side_effect_record<'a, T>(
    slot: &mut Option<&'a T>,
    value: &'a T,
    label: &'static str,
    pair_id: &SideEffectPairId,
) -> ControlFlow<ReplayError> {
    if slot.replace(value).is_some() {
        return ControlFlow::Break(ReplayError::new(
            ReplayErrorKind::InvalidRunJournal,
            format!("duplicate side-effect {label} {pair_id}"),
        ));
    }
    ControlFlow::Continue(())
}

fn validate_additional_artifacts(artifacts: &[store::VerifiedRetainedArtifactBytes]) -> Result<()> {
    for (index, artifact) in artifacts.iter().enumerate() {
        let key = replay_artifact_key(artifact.evidence())?;
        for other in &artifacts[index + 1..] {
            if replay_artifact_key(other.evidence())? != key {
                continue;
            }
            if artifact != other {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!(
                        "conflicting additional retained artifact evidence for {}",
                        artifact.evidence().artifact_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_source_fact_events(source_facts: &[RetainedSourceFactReplayEvent]) -> Result<()> {
    for (index, source) in source_facts.iter().enumerate() {
        for other in &source_facts[index + 1..] {
            if source.fact_claim_id() != other.fact_claim_id() {
                continue;
            }
            if source.event_id() != other.event_id() || source.payload() != other.payload() {
                return Err(ReplayError::new(
                    ReplayErrorKind::FactMismatch,
                    "conflicting retained source fact events share one fact claim id",
                ));
            }
        }
    }
    Ok(())
}
