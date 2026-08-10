//! Sole ordering owner for one qualified semantic commit.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_ids::RunId;
use mfm_journal::structured::{
    CommittedBatch, RunRecord, TenantFactCoordinate, TenantFactFrontier,
};
use mfm_runtime::history::{
    CommittedAccessAuthorization, HistoryAppendOutcome, QualifiedRuntimeIntent,
    StructuredAppendAttempt,
};

use super::backend::{
    prior_run_fact_source, AppendAttemptLookup, BackendAppendOutcome, RawRunHistory,
    StructuredHistoryBackend, StructuredRunHistoryWriter,
};
use super::compiler::{compile_preview, ComparedReduction, FactScanPermitSpec};
use super::obligations::{discharge, FinalizedReduction, ObligationDischargeScope};
use super::qualification::{
    invalid, qualify_admission_intent, qualify_existing_intent, qualify_recorded_history,
    qualify_recorded_successor, QualifiedHistory, QualifiedRunContext, StructuredStoreError,
};
use super::reducer::{
    reduce_event, PendingSemanticStep, QualifiedEvent, QualifiedIntentEvent, ReducedRunState,
    TenantFactRequirement,
};
use super::validated_append::{RunCurrentProjection, ValidatedRunAppend};
use super::VerifiedStructuredRun;

struct PreparedIntent {
    run_id: RunId,
    context: Arc<QualifiedRunContext>,
    previous: Box<ReducedRunState>,
    previous_projection: Option<RunCurrentProjection>,
    retained_objects: BTreeSet<mfm_ids::ContentRef>,
    prior_history: Option<Arc<QualifiedHistory>>,
    event: QualifiedIntentEvent,
}

pub(super) async fn commit_event<B: StructuredHistoryBackend>(
    writer: &StructuredRunHistoryWriter<B>,
    previous: Option<VerifiedStructuredRun>,
    intent: QualifiedRuntimeIntent,
) -> super::Result<(RunId, StructuredAppendAttempt)> {
    let run_id = intent_run_id(writer, previous.as_ref(), &intent).map_err(candidate_rejected)?;
    let append_request_id = intent.append_request_id().clone();
    if let Some(found) = writer
        .backend
        .lookup_append_attempt(&run_id, &append_request_id)
        .await?
    {
        return classify_retained(writer, run_id, intent, found).await;
    }
    let prepared = prepare_new(writer, previous, intent).map_err(candidate_rejected)?;
    let intended = reduce_event(
        &prepared.context,
        &prepared.previous,
        &QualifiedEvent::Intent(&prepared.event),
    )
    .map_err(candidate_rejected)?;
    let tenant_frontier = tenant_frontier(writer, &prepared.context, &intended).await?;
    let preview = compile_preview(
        writer.backend.identity(),
        &prepared.context,
        prepared.previous_projection.clone(),
        tenant_frontier,
        &prepared.retained_objects,
        &append_request_id,
        &intended,
    )?;
    let candidate = preview.committed().clone();
    let history = match prepared.prior_history {
        Some(prior) => {
            qualify_recorded_successor(&prior, candidate.clone()).map_err(candidate_rejected)?
        }
        None => qualify_recorded_history(
            RawRunHistory {
                run_id: prepared.run_id.clone(),
                batches: vec![candidate.clone()],
            },
            &writer.programs,
        )
        .map_err(candidate_rejected)?,
    };
    let recorded_batch = history.batches.last().ok_or_else(invalid)?;
    let recorded = reduce_event(
        &history.context,
        &prepared.previous,
        &QualifiedEvent::Recorded(&recorded_batch.event),
    )
    .map_err(candidate_rejected)?;
    let compared = ComparedReduction::compare(
        Some(intended),
        recorded,
        &recorded_batch.assertions,
        &recorded_batch.committed,
        preview,
    )
    .map_err(candidate_rejected)?;
    let finalized = discharge(
        compared,
        &history.context,
        writer.physical.as_ref(),
        ObligationDischargeScope::RetainedAndCurrent,
    )
    .map_err(candidate_rejected)?;
    let command = ValidatedRunAppend::from_finalized(&finalized);
    let outcome = writer.backend.append(command).await?;
    classify_backend(writer, prepared.run_id, history, finalized, outcome).await
}

fn intent_run_id<B: StructuredHistoryBackend>(
    writer: &StructuredRunHistoryWriter<B>,
    previous: Option<&VerifiedStructuredRun>,
    intent: &QualifiedRuntimeIntent,
) -> super::Result<RunId> {
    match (previous, intent) {
        (None, QualifiedRuntimeIntent::Admission(command)) => {
            mfm_journal::structured::derive_run_id(
                &writer.backend.identity().store_scope_id,
                command.tenant_scope_id(),
                command.entry_point_operation_id(),
                command.invocation_identity(),
            )
            .map_err(|_| invalid())
        }
        (Some(_previous), QualifiedRuntimeIntent::Admission(_)) => Err(invalid()),
        (Some(previous), _) => Ok(previous.run_id().clone()),
        (None, _) => Err(invalid()),
    }
}

fn prepare_new<B: StructuredHistoryBackend>(
    writer: &StructuredRunHistoryWriter<B>,
    previous: Option<VerifiedStructuredRun>,
    intent: QualifiedRuntimeIntent,
) -> super::Result<PreparedIntent> {
    match (previous, intent) {
        (None, QualifiedRuntimeIntent::Admission(command)) => {
            let admission =
                qualify_admission_intent(writer.backend.identity(), *command, &writer.programs)?;
            let run_id = admission.admission.run_id.clone();
            Ok(PreparedIntent {
                run_id,
                previous: Box::new(ReducedRunState::empty(&admission)),
                previous_projection: None,
                retained_objects: BTreeSet::new(),
                prior_history: None,
                event: QualifiedIntentEvent::Admission,
                context: admission,
            })
        }
        (Some(previous), intent @ QualifiedRuntimeIntent::Transition(_))
        | (Some(previous), intent @ QualifiedRuntimeIntent::Authorization(_))
        | (Some(previous), intent @ QualifiedRuntimeIntent::Observation(_)) => {
            if previous.admission().store_scope_id != writer.backend.identity().store_scope_id
                || previous.admission().store_epoch != writer.backend.identity().store_epoch
            {
                return Err(invalid());
            }
            let (context, event) = qualify_existing_intent(&previous.history.context, intent)?;
            let run_id = previous.run_id().clone();
            let previous_projection = previous.current_projection();
            let retained_objects = previous.history.objects.objects().keys().cloned().collect();
            let prior_history = Arc::clone(&previous.history);
            Ok(PreparedIntent {
                run_id,
                context,
                previous: previous.reduced,
                previous_projection: Some(previous_projection),
                retained_objects,
                prior_history: Some(prior_history),
                event,
            })
        }
        _ => Err(invalid()),
    }
}

async fn tenant_frontier<B: StructuredHistoryBackend>(
    writer: &StructuredRunHistoryWriter<B>,
    context: &QualifiedRunContext,
    pending: &PendingSemanticStep,
) -> super::Result<Option<TenantFactFrontier>> {
    match pending.tenant_fact_requirement {
        TenantFactRequirement::None => Ok(None),
        TenantFactRequirement::Barrier | TenantFactRequirement::Publish => writer
            .backend
            .tenant_fact_frontier(&context.admission.tenant_scope_id)
            .await
            .map(Some),
    }
}

async fn classify_retained<B: StructuredHistoryBackend>(
    writer: &StructuredRunHistoryWriter<B>,
    run_id: RunId,
    intent: QualifiedRuntimeIntent,
    found: AppendAttemptLookup,
) -> super::Result<(RunId, StructuredAppendAttempt)> {
    let candidate = found.history.batches.last().cloned().ok_or_else(invalid)?;
    if compare_retained_intent(writer, &found.history, intent).is_err() {
        return Err(StructuredStoreError::AppendConflict);
    }
    super::semantic_open::load_and_compare(
        &writer.backend,
        &run_id,
        &writer.programs,
        &writer.physical,
    )
    .await?;
    Ok((
        run_id,
        runtime_attempt(
            candidate.clone(),
            HistoryAppendOutcome::ExistingSame(candidate),
            None,
        ),
    ))
}

fn compare_retained_intent<B: StructuredHistoryBackend>(
    writer: &StructuredRunHistoryWriter<B>,
    raw: &RawRunHistory,
    intent: QualifiedRuntimeIntent,
) -> super::Result<()> {
    let append_request_id = intent.append_request_id().clone();
    let mut prior_batches = raw.batches.clone();
    let candidate = prior_batches.pop().ok_or_else(invalid)?;
    let (context, previous, event, projection, retained) = if prior_batches.is_empty() {
        let QualifiedRuntimeIntent::Admission(command) = intent else {
            return Err(StructuredStoreError::AppendConflict);
        };
        let admission =
            qualify_admission_intent(writer.backend.identity(), *command, &writer.programs)?;
        let previous = ReducedRunState::empty(&admission);
        (
            admission,
            previous,
            QualifiedIntentEvent::Admission,
            None,
            BTreeSet::new(),
        )
    } else {
        let prior = super::semantic_open::verify_qualified(
            RawRunHistory {
                run_id: raw.run_id.clone(),
                batches: prior_batches,
            },
            &writer.programs,
            writer.physical.as_ref(),
        )?;
        let (context, event) = qualify_existing_intent(&prior.history.context, intent)?;
        let retained = prior.history.objects.objects().keys().cloned().collect();
        let projection = Some(prior.current_projection());
        (context, *prior.reduced, event, projection, retained)
    };
    let intended = reduce_event(&context, &previous, &QualifiedEvent::Intent(&event))?;
    let frontier = frontier_before(&candidate)?;
    let preview = compile_preview(
        writer.backend.identity(),
        &context,
        projection,
        frontier,
        &retained,
        &append_request_id,
        &intended,
    )?;
    (preview.committed() == &candidate)
        .then_some(())
        .ok_or(StructuredStoreError::AppendConflict)
}

fn frontier_before(batch: &CommittedBatch) -> super::Result<Option<TenantFactFrontier>> {
    match &batch.tenant_fact_coordinate {
        TenantFactCoordinate::None => Ok(None),
        TenantFactCoordinate::FactSelectionBarrier { frontier } => Ok(Some(frontier.clone())),
        TenantFactCoordinate::FactPublication { frontier } => Ok(Some(TenantFactFrontier::new(
            frontier.store_scope_id.clone(),
            frontier.store_epoch,
            frontier.tenant_scope_id.clone(),
            frontier.fact_order.checked_sub(1).ok_or_else(invalid)?,
        ))),
    }
}

async fn classify_backend<B: StructuredHistoryBackend>(
    writer: &StructuredRunHistoryWriter<B>,
    run_id: RunId,
    history: Arc<super::qualification::QualifiedHistory>,
    finalized: FinalizedReduction,
    outcome: BackendAppendOutcome,
) -> super::Result<(RunId, StructuredAppendAttempt)> {
    let candidate = finalized.committed.clone();
    let result = match outcome {
        BackendAppendOutcome::NewlyCommitted(committed) if committed == candidate => {
            let spec = finalized.fact_read_capability_spec.clone();
            let successor = VerifiedStructuredRun::from_finalized(history, finalized);
            let authorization =
                mint_committed_fact_read_capability(writer, spec, &committed, &successor)?;
            runtime_attempt(
                committed.clone(),
                HistoryAppendOutcome::NewlyCommitted(committed),
                authorization,
            )
        }
        BackendAppendOutcome::ExistingSame(committed) if committed == candidate => runtime_attempt(
            committed.clone(),
            HistoryAppendOutcome::ExistingSame(committed),
            None,
        ),
        BackendAppendOutcome::StaleHead => {
            let expected = finalized.run_projection.expected();
            match super::semantic_open::load_and_compare(
                &writer.backend,
                &run_id,
                &writer.programs,
                &writer.physical,
            )
            .await
            {
                Ok(actual) if Some(&actual.current_projection()) == expected => {
                    runtime_attempt(candidate, HistoryAppendOutcome::StaleHead, None)
                }
                Ok(_) => return Err(StructuredStoreError::CandidateRejected),
                Err(StructuredStoreError::RunNotFound) if expected.is_none() => {
                    runtime_attempt(candidate, HistoryAppendOutcome::StaleHead, None)
                }
                Err(StructuredStoreError::RunNotFound) => {
                    return Err(StructuredStoreError::InvalidHistory);
                }
                Err(error) => return Err(error),
            }
        }
        BackendAppendOutcome::AcknowledgementUnknown => runtime_attempt(
            candidate,
            HistoryAppendOutcome::AcknowledgementUnknown,
            None,
        ),
        _ => return Err(StructuredStoreError::InvalidHistory),
    };
    Ok((run_id, result))
}

fn candidate_rejected(error: StructuredStoreError) -> StructuredStoreError {
    match error {
        StructuredStoreError::InvalidHistory | StructuredStoreError::Certification => {
            StructuredStoreError::CandidateRejected
        }
        error => error,
    }
}

fn runtime_attempt(
    committed: CommittedBatch,
    outcome: HistoryAppendOutcome,
    authorization: Option<CommittedAccessAuthorization>,
) -> StructuredAppendAttempt {
    let closed = committed
        .records
        .iter()
        .any(|assigned| matches!(assigned.record, RunRecord::RunClosed(_)));
    StructuredAppendAttempt::from_store_attempt(
        committed.append_request_id,
        outcome,
        authorization,
        closed,
    )
}

type FactReadFuture = Pin<
    Box<dyn Future<Output = mfm_certify::structured::PriorRunFactScanCompletion> + Send + 'static>,
>;

fn mint_committed_fact_read_capability<B: StructuredHistoryBackend>(
    writer: &StructuredRunHistoryWriter<B>,
    spec: Option<FactScanPermitSpec>,
    committed: &CommittedBatch,
    successor: &VerifiedStructuredRun,
) -> super::Result<Option<CommittedAccessAuthorization>> {
    let [assigned] = committed.records.as_slice() else {
        return Ok(None);
    };
    let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
        return Ok(None);
    };
    let permit = super::fact_scan::build_committed_fact_read_capability(
        prior_run_fact_source(&writer.backend),
        Arc::clone(&writer.programs),
        Arc::clone(&writer.physical),
        spec,
        committed,
        successor,
    )?;
    let fact_read = permit.map(|permit| {
        Box::new(move |request| {
            let future: FactReadFuture = Box::pin(async move {
                match permit.invoke(request).await {
                    super::fact_scan::PriorRunFactScanCompletion::Returned(value) => {
                        mfm_certify::structured::PriorRunFactScanCompletion::Returned(value)
                    }
                    super::fact_scan::PriorRunFactScanCompletion::SafeFailure(value) => {
                        mfm_certify::structured::PriorRunFactScanCompletion::SafeFailure(value)
                    }
                    super::fact_scan::PriorRunFactScanCompletion::IntegrityFault(code) => {
                        mfm_certify::structured::PriorRunFactScanCompletion::IntegrityFault(code)
                    }
                }
            });
            future
        }) as Box<dyn FnOnce(mfm_facts::FactSelectionRequest) -> FactReadFuture + Send>
    });
    Ok(Some(
        CommittedAccessAuthorization::from_committed_successor(
            assigned.record_ref.clone(),
            authorization.clone(),
            fact_read,
            committed.predecessor.clone(),
            committed.head.clone(),
        ),
    ))
}
