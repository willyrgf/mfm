use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mfm_ids::{
    AppendRequestId, InvocationIdentity, JournalCandidateDigest, RunId, StableId, TenantScopeId,
};
use mfm_journal::{
    ConfiguredValueBinding, JournalHead, JournalPredecessor, JournalPredecessorFields,
    ProducerBindingFields, TenantFactCoordinate, TenantFactCoordinateFields, TenantFactFrontier,
};

use super::backend::{AsyncStoreFuture, RunJournalBackend};
use super::objects::ObjectAuthorityState;
use super::{
    AdmissionSourceBackend, AdmissionSourceVerifier, AdmittedSupportGraph, AppendOutcome,
    AppendRejection, CommittedJournalCommit, CommittedObject, CommittedRunJournal,
    ConfiguredValueBackend, ConfiguredValueResolveVerifier, FactAttestationLoadVerifier,
    FactScanBackend, FactScanPage, FactScanPageVerifier, JournalAppendVerifier,
    JournalLoadVerifier, ObjectAuthorityKey, PersistedFactScanAttestation, PreparedAppendKind,
    QualifiedRunStore, RunAccessAuthorityIssuer, RunHistoryWriter, StoreAuthorityContext,
    StoreError, StoreIdentity, SupportBackend, SupportGraphAdmissionVerifier,
    VerifiedAdmissionSources, VerifiedConfiguredValue,
};

#[derive(Clone)]
struct MemoryRun {
    tenant_scope_id: TenantScopeId,
    commits: Vec<CommittedJournalCommit>,
}

#[derive(Clone)]
struct IdempotencyEntry {
    candidate_digest: JournalCandidateDigest,
    expected_predecessor: JournalPredecessor,
    run_sequence: u64,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AdmissionKey {
    tenant_scope_id: TenantScopeId,
    entry_point_operation_id: StableId,
    invocation_identity: InvocationIdentity,
}

#[derive(Clone)]
struct AdmissionEntry {
    run_id: RunId,
    candidate_digest: JournalCandidateDigest,
}

#[derive(Clone, Default)]
struct MemoryCore {
    runs: BTreeMap<RunId, MemoryRun>,
    objects: BTreeMap<ObjectAuthorityKey, CommittedObject>,
    idempotency: BTreeMap<(RunId, AppendRequestId), IdempotencyEntry>,
    admissions: BTreeMap<AdmissionKey, AdmissionEntry>,
    tenant_fact_heads: BTreeMap<TenantScopeId, u64>,
    fact_scan_attestations: Vec<PersistedFactScanAttestation>,
    next_committed_at: u64,
}

/// Test-only atomic commit failure or acknowledgement-loss point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryCommitFailurePoint {
    /// Fail after all validation but before committing any staged state.
    BeforeCommit,
    /// Commit the complete transaction, then lose its acknowledgement.
    AfterCommitBeforeAcknowledgement,
}

struct MemoryCommitFailureArm {
    run_id: RunId,
    batch_purpose: mfm_journal::BatchPurpose,
    point: MemoryCommitFailurePoint,
}

/// Test-only in-memory implementation hidden behind the affine history wrappers.
#[doc(hidden)]
pub struct InMemoryRunJournalBackend {
    authority: StoreAuthorityContext,
    core: Arc<Mutex<MemoryCore>>,
    commit_failure: Arc<Mutex<Option<MemoryCommitFailureArm>>>,
    #[cfg(test)]
    backend_loads: AtomicUsize,
}

/// Opens one empty test store as a pre-split qualified history assembly.
pub fn open_in_memory(
    identity: StoreIdentity,
) -> (
    QualifiedRunStore<InMemoryRunJournalBackend>,
    RunAccessAuthorityIssuer,
) {
    let (backend, issuer) = InMemoryRunJournalBackend::new(identity);
    (QualifiedRunStore::from_qualified_backend(backend), issuer)
}

impl QualifiedRunStore<InMemoryRunJournalBackend> {
    /// Provisions one immutable configured value before the history split.
    pub fn provision_configured_value(
        &self,
        binding: ConfiguredValueBinding,
        bytes: Vec<u8>,
    ) -> Result<(), StoreError> {
        self.backend().provision_configured_value(binding, bytes)
    }
}

impl RunHistoryWriter<InMemoryRunJournalBackend> {
    /// Arms one exact test-only commit failure or acknowledgement loss.
    pub fn inject_commit_failure(
        &self,
        run_id: RunId,
        batch_purpose: mfm_journal::BatchPurpose,
        point: MemoryCommitFailurePoint,
    ) -> Result<(), StoreError> {
        self.backend()
            .inject_commit_failure(run_id, batch_purpose, point)
    }

    /// Returns whether a test-only commit failure selector remains armed.
    pub fn commit_failure_is_armed(&self) -> Result<bool, StoreError> {
        self.backend().commit_failure_is_armed()
    }

    #[cfg(test)]
    pub(super) fn remove_run_for_trace_test(&self, run_id: &RunId) -> Result<(), StoreError> {
        self.backend().remove_run_for_trace_test(run_id)
    }

    #[cfg(test)]
    pub(super) fn recorded_run_for_observation_test(
        &self,
        run_id: &RunId,
    ) -> Result<
        (
            TenantScopeId,
            Vec<CommittedJournalCommit>,
            Vec<CommittedObject>,
        ),
        StoreError,
    > {
        self.backend().recorded_run_for_observation_test(run_id)
    }

    #[cfg(test)]
    pub(super) fn remove_transition_output_for_trace_test(
        &self,
        run_id: &RunId,
    ) -> Result<(), StoreError> {
        self.backend()
            .remove_transition_output_for_trace_test(run_id)
    }
}

#[cfg(test)]
impl super::RunHistoryReader<InMemoryRunJournalBackend> {
    pub(super) fn backend_load_count_for_test(&self) -> usize {
        self.backend().backend_loads.load(Ordering::SeqCst)
    }
}

impl InMemoryRunJournalBackend {
    fn new(identity: StoreIdentity) -> (Self, RunAccessAuthorityIssuer) {
        let (authority, issuer) = StoreAuthorityContext::bootstrap(identity);
        (
            Self {
                authority,
                core: Arc::new(Mutex::new(MemoryCore {
                    next_committed_at: 1,
                    ..MemoryCore::default()
                })),
                commit_failure: Arc::new(Mutex::new(None)),
                #[cfg(test)]
                backend_loads: AtomicUsize::new(0),
            },
            issuer,
        )
    }

    #[cfg(test)]
    pub(super) fn remove_run_for_trace_test(&self, run_id: &RunId) -> Result<(), StoreError> {
        let mut core = self
            .core
            .lock()
            .map_err(|_| StoreError::MemoryLockPoisoned)?;
        core.runs.remove(run_id).ok_or(StoreError::RunNotFound)?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn recorded_run_for_observation_test(
        &self,
        run_id: &RunId,
    ) -> Result<
        (
            TenantScopeId,
            Vec<CommittedJournalCommit>,
            Vec<CommittedObject>,
        ),
        StoreError,
    > {
        let core = self
            .core
            .lock()
            .map_err(|_| StoreError::MemoryLockPoisoned)?;
        let run = core.runs.get(run_id).ok_or(StoreError::RunNotFound)?;
        Ok((
            run.tenant_scope_id.clone(),
            run.commits.clone(),
            reachable_objects(&core, run)?,
        ))
    }

    #[cfg(test)]
    pub(super) fn remove_transition_output_for_trace_test(
        &self,
        run_id: &RunId,
    ) -> Result<(), StoreError> {
        let mut core = self
            .core
            .lock()
            .map_err(|_| StoreError::MemoryLockPoisoned)?;
        let key = core
            .objects
            .iter()
            .find_map(|(key, object)| {
                object
                    .value_ref()
                    .fields()
                    .and_then(|fields| fields.producer_binding.fields())
                    .is_ok_and(|producer| {
                        matches!(
                            producer,
                            ProducerBindingFields::TransitionOutput {
                                run_id: ref producer_run_id,
                                ..
                            } if producer_run_id == run_id
                        )
                    })
                    .then(|| key.clone())
            })
            .ok_or(StoreError::ObjectNotReachable)?;
        core.objects.remove(&key);
        Ok(())
    }

    /// Arms one exact run-and-purpose commit failure or acknowledgement loss.
    ///
    /// Unrelated appends do not consume the selector. A matching append consumes it once.
    fn inject_commit_failure(
        &self,
        run_id: RunId,
        batch_purpose: mfm_journal::BatchPurpose,
        point: MemoryCommitFailurePoint,
    ) -> Result<(), StoreError> {
        let mut armed = self
            .commit_failure
            .lock()
            .map_err(|_| StoreError::MemoryLockPoisoned)?;
        if armed.is_some() {
            return Err(StoreError::MemoryFailureSelectorAlreadyArmed);
        }
        *armed = Some(MemoryCommitFailureArm {
            run_id,
            batch_purpose,
            point,
        });
        Ok(())
    }

    /// Reports whether an exact test-only commit failure selector remains armed.
    fn commit_failure_is_armed(&self) -> Result<bool, StoreError> {
        self.commit_failure
            .lock()
            .map_err(|_| StoreError::MemoryLockPoisoned)
            .map(|armed| armed.is_some())
    }

    /// Provisions one exact immutable configured value for test-support workflows.
    fn provision_configured_value(
        &self,
        binding: ConfiguredValueBinding,
        bytes: Vec<u8>,
    ) -> Result<(), StoreError> {
        let fields = binding.fields()?;
        let producer = fields.value_ref.fields()?.producer_binding.fields()?;
        let ProducerBindingFields::ConfiguredValue {
            store_scope_id,
            tenant_scope_id,
            entry_point_id,
            target,
        } = producer
        else {
            return Err(StoreError::InvalidObjectAuthority {
                message: "test configured binding has a non-configured producer",
            });
        };
        let key = fields.key.fields()?;
        if store_scope_id != key.store_scope_id
            || tenant_scope_id != key.tenant_scope_id
            || entry_point_id != key.entry_point_id
            || target != key.target
            || store_scope_id != *self.authority.store_identity().store_scope_id()
        {
            return Err(StoreError::InvalidObjectAuthority {
                message: "test configured binding disagrees with its producer or store",
            });
        }
        let candidate = CommittedObject::from_persisted(fields.value_ref, bytes)?;
        let mut guard = self
            .core
            .lock()
            .map_err(|_| StoreError::MemoryLockPoisoned)?;
        match guard.objects.get(candidate.key()) {
            Some(existing)
                if existing.value_ref().as_bytes() == candidate.value_ref().as_bytes()
                    && existing.bytes() == candidate.bytes() =>
            {
                Ok(())
            }
            Some(_) => Err(StoreError::ObjectAuthorityConflict {
                artifact_id: candidate.key().artifact_id().clone(),
            }),
            None => {
                guard.objects.insert(candidate.key().clone(), candidate);
                Ok(())
            }
        }
    }

    fn take_commit_failure(
        &self,
        run_id: &RunId,
        batch_purpose: mfm_journal::BatchPurpose,
    ) -> Result<Option<MemoryCommitFailurePoint>, StoreError> {
        let mut armed = self
            .commit_failure
            .lock()
            .map_err(|_| StoreError::MemoryLockPoisoned)?;
        if armed.as_ref().is_some_and(|selector| {
            selector.run_id == *run_id && selector.batch_purpose == batch_purpose
        }) {
            return Ok(armed.take().map(|selector| selector.point));
        }
        Ok(None)
    }

    fn append_verified(
        &self,
        verifier: JournalAppendVerifier,
    ) -> Result<AppendOutcome, StoreError> {
        let mut guard = self
            .core
            .lock()
            .map_err(|_| StoreError::MemoryLockPoisoned)?;
        let mut staged = guard.clone();

        if let Some(existing) = staged.idempotency.get(&(
            verifier.run_id().clone(),
            verifier.append_request_id().clone(),
        )) {
            if existing.candidate_digest != *verifier.candidate_digest()
                || existing.expected_predecessor != *verifier.expected_predecessor()
            {
                return Ok(verifier.rejected(AppendRejection::AppendRequestConflict));
            }
            let run = staged
                .runs
                .get(verifier.run_id())
                .ok_or(StoreError::RunNotFound)?;
            let commit = run
                .commits
                .get(
                    usize::try_from(existing.run_sequence - 1)
                        .map_err(|_| StoreError::SequenceOverflow)?,
                )
                .ok_or(StoreError::RunNotFound)?;
            if let Some(pending) = verifier.pending_fact_scan_attestation() {
                let expected = pending.persisted_for_commit(commit)?;
                let retained = staged
                    .fact_scan_attestations
                    .iter()
                    .find(|row| row.authorization_ref() == expected.authorization_ref())
                    .ok_or(StoreError::FactScanBindingMismatch)?;
                if retained != &expected {
                    return Err(StoreError::FactScanBindingMismatch);
                }
            }
            let frontier = fact_frontier(
                self.authority.store_identity(),
                &run.tenant_scope_id,
                commit,
            )?;
            let commits = run.commits.clone();
            let objects = reachable_objects(&staged, run)?;
            return verifier.already_committed_rows(
                commits,
                objects,
                existing.run_sequence,
                frontier,
            );
        }

        if verifier.kind() == PreparedAppendKind::AdmitRun {
            let key = admission_key(&verifier)?;
            if let Some(existing) = staged.admissions.get(&key) {
                if existing.run_id != *verifier.run_id()
                    || existing.candidate_digest != *verifier.candidate_digest()
                {
                    return Ok(verifier.rejected(AppendRejection::AdmissionConflict));
                }
                let run = staged
                    .runs
                    .get(&existing.run_id)
                    .ok_or(StoreError::RunNotFound)?;
                let commit = run.commits.first().ok_or(StoreError::EmptyJournal)?;
                let frontier = fact_frontier(
                    self.authority.store_identity(),
                    &run.tenant_scope_id,
                    commit,
                )?;
                let commits = run.commits.clone();
                let objects = reachable_objects(&staged, run)?;
                return verifier.already_committed_rows(commits, objects, 1, frontier);
            }
        }

        match verifier.kind() {
            PreparedAppendKind::AdmitRun => {
                append_admission_to_core(self.authority.store_identity(), &mut staged, &verifier)?;
            }
            PreparedAppendKind::CommitTransition
            | PreparedAppendKind::AuthorizeExternalAccess
            | PreparedAppendKind::ObserveExternalAccess => {
                let run = staged.runs.get(verifier.run_id()).ok_or_else(|| {
                    StoreError::AppendRunNotFound {
                        run_id: verifier.run_id().clone(),
                    }
                })?;
                if run.tenant_scope_id != *verifier.tenant_scope_id() {
                    return Err(StoreError::AccessDenied { purpose: "drive" });
                }
                let current = run
                    .commits
                    .last()
                    .ok_or(StoreError::EmptyJournal)?
                    .envelope()
                    .journal_head()?;
                let expected = predecessor_head(verifier.expected_predecessor())?;
                if expected.as_ref() != Some(&current) {
                    let expected = expected.unwrap_or_else(|| current.clone());
                    return Ok(verifier.rejected(AppendRejection::StaleHead {
                        expected,
                        actual: Box::new(current),
                    }));
                }
                let view = verified_view_for(
                    self.authority.store_identity(),
                    &staged,
                    verifier.tenant_scope_id(),
                    verifier.run_id(),
                )?;
                verifier.verify_successor(&view)?;
            }
        }

        let run_sequence = match staged.runs.get(verifier.run_id()) {
            Some(run) => u64::try_from(run.commits.len())
                .map_err(|_| StoreError::SequenceOverflow)?
                .checked_add(1)
                .ok_or(StoreError::SequenceOverflow)?,
            None => 1,
        };
        let coordinate = assign_coordinate(&mut staged, &verifier)?;
        let append_tenant_scope_id = verifier.tenant_scope_id().clone();
        let failure_run_id = verifier.run_id().clone();
        let failure_batch_purpose = verifier.batch_purpose()?;
        let committed_at = staged.next_committed_at;
        staged.next_committed_at = staged
            .next_committed_at
            .checked_add(1)
            .ok_or(StoreError::SequenceOverflow)?;
        let assigned = verifier.assign(run_sequence, coordinate, committed_at)?;

        let mut authority_state =
            ObjectAuthorityState::from_committed(staged.objects.values().cloned())?;
        authority_state.apply(assigned.objects())?;
        staged.objects = authority_state
            .values()
            .cloned()
            .map(|object| (object.key().clone(), object))
            .collect();

        let persisted_attestation = assigned.persisted_fact_scan_attestation()?;
        if let Some(persisted) = &persisted_attestation {
            if staged
                .fact_scan_attestations
                .iter()
                .any(|row| row.authorization_ref() == persisted.authorization_ref())
            {
                return Err(StoreError::FactScanBindingMismatch);
            }
        }

        let commit = assigned.commit().clone();
        let append_request_id = commit.envelope().fields()?.core.append_request_id;
        let expected_predecessor = commit.envelope().fields()?.core.predecessor;
        let candidate_digest = commit.envelope().fields()?.core.candidate_digest;
        staged
            .runs
            .entry(verifier_run_id_from_commit(&commit)?)
            .and_modify(|run| run.commits.push(commit.clone()))
            .or_insert_with(|| MemoryRun {
                tenant_scope_id: append_tenant_scope_id.clone(),
                commits: vec![commit.clone()],
            });
        staged.idempotency.insert(
            (verifier_run_id_from_commit(&commit)?, append_request_id),
            IdempotencyEntry {
                candidate_digest,
                expected_predecessor,
                run_sequence,
            },
        );
        if let Some(persisted) = persisted_attestation {
            staged.fact_scan_attestations.push(persisted);
        }

        // Re-run the one physical verifier and semantic reducer over the exact staged
        // successor before the atomic swap. This also validates objects produced by the
        // candidate; no pre-append preview is allowed to substitute for the resulting prefix.
        verified_view_for(
            self.authority.store_identity(),
            &staged,
            &append_tenant_scope_id,
            &verifier_run_id_from_commit(&commit)?,
        )?;

        let failure = self.take_commit_failure(&failure_run_id, failure_batch_purpose)?;
        if failure == Some(MemoryCommitFailurePoint::BeforeCommit) {
            return Err(StoreError::InjectedFailure {
                point: "before_commit",
            });
        }
        *guard = staged;
        if failure == Some(MemoryCommitFailurePoint::AfterCommitBeforeAcknowledgement) {
            return Ok(assigned.outcome_unknown());
        }
        assigned.directly_committed()
    }
}

impl RunJournalBackend for InMemoryRunJournalBackend {
    type Error = StoreError;

    fn store_authority_context(&self) -> &StoreAuthorityContext {
        &self.authority
    }

    fn backend_append<'a>(
        &'a self,
        verifier: JournalAppendVerifier,
    ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error> {
        Box::pin(async move { self.append_verified(verifier) })
    }

    fn backend_load<'a>(
        &'a self,
        verifier: JournalLoadVerifier,
    ) -> AsyncStoreFuture<'a, CommittedRunJournal, Self::Error> {
        #[cfg(test)]
        self.backend_loads.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let guard = self
                .core
                .lock()
                .map_err(|_| StoreError::MemoryLockPoisoned)?;
            load_from_core(&guard, verifier)
        })
    }
}

impl AdmissionSourceBackend for InMemoryRunJournalBackend {
    fn backend_verify_admission_sources<'a>(
        &'a self,
        mut verifier: AdmissionSourceVerifier,
    ) -> AsyncStoreFuture<'a, VerifiedAdmissionSources, Self::Error> {
        Box::pin(async move {
            while let Some(load) = verifier.pending_source_load_verifier() {
                let journal = {
                    let guard = self
                        .core
                        .lock()
                        .map_err(|_| StoreError::MemoryLockPoisoned)?;
                    load_from_core(&guard, load)?
                };
                verifier.accept_verified_source(journal)?;
            }
            verifier.complete()
        })
    }
}

impl SupportBackend for InMemoryRunJournalBackend {
    fn backend_admit_support_graph<'a>(
        &'a self,
        verifier: SupportGraphAdmissionVerifier,
    ) -> AsyncStoreFuture<'a, AdmittedSupportGraph, Self::Error> {
        Box::pin(async move {
            if verifier.prepared().store_identity() != self.authority.store_identity() {
                return Err(StoreError::AccessDenied {
                    purpose: "admit_support_graph",
                });
            }
            let mut guard = self
                .core
                .lock()
                .map_err(|_| StoreError::MemoryLockPoisoned)?;
            let mut staged = guard.clone();
            for member in verifier.prepared().members().values() {
                let candidate = CommittedObject::from_persisted(
                    member.value_ref().clone(),
                    member.bytes().to_vec(),
                )?;
                match staged.objects.get(candidate.key()) {
                    Some(existing)
                        if existing.value_ref().as_bytes() == candidate.value_ref().as_bytes()
                            && existing.bytes() == candidate.bytes() => {}
                    Some(_) => {
                        return Err(StoreError::ObjectAuthorityConflict {
                            artifact_id: candidate.key().artifact_id().clone(),
                        });
                    }
                    None => {
                        staged.objects.insert(candidate.key().clone(), candidate);
                    }
                }
            }
            *guard = staged;
            Ok(verifier.complete())
        })
    }
}

impl ConfiguredValueBackend for InMemoryRunJournalBackend {
    fn backend_resolve_configured_value<'a>(
        &'a self,
        verifier: ConfiguredValueResolveVerifier,
    ) -> AsyncStoreFuture<'a, VerifiedConfiguredValue, Self::Error> {
        Box::pin(async move {
            let expected_key = verifier.key().fields()?;
            let guard = self
                .core
                .lock()
                .map_err(|_| StoreError::MemoryLockPoisoned)?;
            let mut matches = guard.objects.values().filter(|object| {
                object
                    .value_ref()
                    .fields()
                    .and_then(|fields| fields.producer_binding.fields())
                    .is_ok_and(|producer| {
                        matches!(
                            producer,
                            ProducerBindingFields::ConfiguredValue {
                                store_scope_id,
                                tenant_scope_id,
                                entry_point_id,
                                target,
                            } if store_scope_id == expected_key.store_scope_id
                                && tenant_scope_id == expected_key.tenant_scope_id
                                && entry_point_id == expected_key.entry_point_id
                                && target == expected_key.target
                        )
                    })
            });
            let object = matches.next().ok_or(StoreError::ObjectNotReachable)?;
            if matches.next().is_some() {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "configured-value key resolves to more than one exact authority",
                });
            }
            let binding = ConfiguredValueBinding::new(verifier.key(), object.value_ref())?;
            verifier.complete(binding, object.bytes().to_vec())
        })
    }
}

impl FactScanBackend for InMemoryRunJournalBackend {
    fn backend_fact_scan_page<'a>(
        &'a self,
        mut verifier: FactScanPageVerifier,
    ) -> AsyncStoreFuture<'a, FactScanPage, Self::Error> {
        Box::pin(async move {
            if verifier.store_identity() != self.authority.store_identity() {
                return Err(StoreError::AccessDenied {
                    purpose: "fact_scan",
                });
            }
            let guard = self
                .core
                .lock()
                .map_err(|_| StoreError::MemoryLockPoisoned)?;
            let mut routes = Vec::new();
            for (run_id, run) in &guard.runs {
                if &run.tenant_scope_id != verifier.tenant_scope_id() {
                    continue;
                }
                for commit in &run.commits {
                    let coordinate = commit
                        .envelope()
                        .fields()?
                        .core
                        .tenant_fact_coordinate
                        .fields()?;
                    let TenantFactCoordinateFields::FactPublication {
                        tenant_scope_id,
                        fact_order,
                    } = coordinate
                    else {
                        continue;
                    };
                    if tenant_scope_id != *verifier.tenant_scope_id()
                        || fact_order < verifier.next_fact_order()
                        || fact_order > verifier.frontier_fact_order()
                    {
                        continue;
                    }
                    routes.push((fact_order, run_id.clone()));
                }
            }
            routes.sort_by_key(|(fact_order, _)| *fact_order);

            let publication_load_limit = verifier.publication_load_limit();
            for (fact_order, run_id) in routes.into_iter().take(publication_load_limit) {
                let run = guard.runs.get(&run_id).ok_or(StoreError::RunNotFound)?;
                let page_full = verifier.submit_publication(
                    fact_order,
                    run_id,
                    run.commits.clone(),
                    reachable_objects(&guard, run)?,
                )?;
                if page_full {
                    break;
                }
            }
            verifier.complete()
        })
    }

    fn backend_load_fact_attestations<'a>(
        &'a self,
        verifier: FactAttestationLoadVerifier,
    ) -> AsyncStoreFuture<'a, Vec<PersistedFactScanAttestation>, Self::Error> {
        Box::pin(async move {
            if verifier.store_identity() != self.authority.store_identity() {
                return Err(StoreError::AccessDenied {
                    purpose: "fact_attestations",
                });
            }
            let guard = self
                .core
                .lock()
                .map_err(|_| StoreError::MemoryLockPoisoned)?;
            let run = guard
                .runs
                .get(verifier.run_id())
                .ok_or(StoreError::RunNotFound)?;
            if &run.tenant_scope_id != verifier.tenant_scope_id() {
                return Err(StoreError::AccessDenied {
                    purpose: "fact_attestations",
                });
            }
            Ok(guard
                .fact_scan_attestations
                .iter()
                .filter_map(|row| {
                    row.observation_ref()
                        .fields()
                        .ok()
                        .filter(|fields| fields.run_id == *verifier.run_id())
                        .map(|_| row.clone())
                })
                .collect())
        })
    }
}

fn append_admission_to_core(
    _identity: &StoreIdentity,
    core: &mut MemoryCore,
    verifier: &JournalAppendVerifier,
) -> Result<(), StoreError> {
    let key = admission_key(verifier)?;
    if core.runs.contains_key(verifier.run_id()) {
        return Err(StoreError::AdmissionConflict);
    }
    core.admissions.insert(
        key,
        AdmissionEntry {
            run_id: verifier.run_id().clone(),
            candidate_digest: verifier.candidate_digest().clone(),
        },
    );
    Ok(())
}

fn admission_key(verifier: &JournalAppendVerifier) -> Result<AdmissionKey, StoreError> {
    Ok(AdmissionKey {
        tenant_scope_id: verifier.tenant_scope_id().clone(),
        entry_point_operation_id: verifier.admission_entry_point_operation_id()?.ok_or(
            StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "admission logical key is absent",
            },
        )?,
        invocation_identity: verifier.admission_invocation_identity()?.ok_or(
            StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "admission invocation identity is absent",
            },
        )?,
    })
}

fn predecessor_head(predecessor: &JournalPredecessor) -> Result<Option<JournalHead>, StoreError> {
    match predecessor.fields()? {
        JournalPredecessorFields::Genesis { .. } => Ok(None),
        JournalPredecessorFields::JournalHead(fields) => {
            JournalHead::new(fields.run_sequence, &fields.commit_digest)
                .map(Some)
                .map_err(Into::into)
        }
    }
}

fn assign_coordinate(
    core: &mut MemoryCore,
    verifier: &JournalAppendVerifier,
) -> Result<TenantFactCoordinate, StoreError> {
    let tenant = verifier.tenant_scope_id();
    let current = core.tenant_fact_heads.get(tenant).copied().unwrap_or(0);
    if verifier.emits_facts() {
        let next = current
            .checked_add(1)
            .ok_or_else(|| StoreError::FactOrderOverflow {
                tenant_scope_id: tenant.clone(),
            })?;
        core.tenant_fact_heads.insert(tenant.clone(), next);
        TenantFactCoordinate::fact_publication(tenant, next).map_err(Into::into)
    } else if verifier.reserves_fact_selection_barrier() {
        TenantFactCoordinate::fact_selection_barrier(tenant, current).map_err(Into::into)
    } else {
        TenantFactCoordinate::none().map_err(Into::into)
    }
}

fn verifier_run_id_from_commit(commit: &CommittedJournalCommit) -> Result<RunId, StoreError> {
    Ok(commit.envelope().fields()?.core.run_id)
}

fn reachable_objects(
    core: &MemoryCore,
    run: &MemoryRun,
) -> Result<Vec<CommittedObject>, StoreError> {
    let mut keys = BTreeSet::new();
    for commit in &run.commits {
        for intent in &commit.envelope().fields()?.core.artifact_admission_intents {
            let fields = intent.fields()?;
            keys.insert(ObjectAuthorityKey::from_value_ref(&fields.value_ref)?);
        }
    }
    keys.into_iter()
        .map(|key| {
            core.objects
                .get(&key)
                .cloned()
                .ok_or(StoreError::MissingObjectAuthority {
                    artifact_id: key.artifact_id().clone(),
                })
        })
        .collect()
}

fn load_from_core(
    core: &MemoryCore,
    verifier: JournalLoadVerifier,
) -> Result<CommittedRunJournal, StoreError> {
    let run = core
        .runs
        .get(verifier.run_id())
        .ok_or(StoreError::RunNotFound)?;
    if &run.tenant_scope_id != verifier.tenant_scope_id() {
        return Err(StoreError::RunNotFound);
    }
    verifier.verify(run.commits.clone(), reachable_objects(core, run)?)
}

fn verified_view_for(
    identity: &StoreIdentity,
    core: &MemoryCore,
    tenant_scope_id: &TenantScopeId,
    run_id: &RunId,
) -> Result<super::VerifiedRunView, StoreError> {
    JournalLoadVerifier::new(identity.clone(), tenant_scope_id.clone(), run_id.clone())
        .verify(
            core.runs
                .get(run_id)
                .ok_or(StoreError::RunNotFound)?
                .commits
                .clone(),
            reachable_objects(core, core.runs.get(run_id).ok_or(StoreError::RunNotFound)?)?,
        )?
        .verify_recorded_history()
}

fn fact_frontier(
    identity: &StoreIdentity,
    tenant_scope_id: &TenantScopeId,
    commit: &CommittedJournalCommit,
) -> Result<Option<TenantFactFrontier>, StoreError> {
    let order = match commit
        .envelope()
        .fields()?
        .core
        .tenant_fact_coordinate
        .fields()?
    {
        TenantFactCoordinateFields::None => return Ok(None),
        TenantFactCoordinateFields::FactPublication { fact_order, .. } => fact_order,
        TenantFactCoordinateFields::FactSelectionBarrier {
            frontier_fact_order,
            ..
        } => frontier_fact_order,
    };
    TenantFactFrontier::new(
        identity.store_scope_id(),
        identity.store_epoch(),
        tenant_scope_id,
        order,
    )
    .map(Some)
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use mfm_ids::{RunId, StoreEpoch, StoreScopeId};
    use mfm_journal::BatchPurpose;

    use super::{open_in_memory, MemoryCommitFailurePoint};
    use crate::{StoreError, StoreIdentity};

    #[test]
    fn reader_clone_does_not_require_the_concrete_backend_to_clone() {
        let identity = StoreIdentity::new(
            StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "ab".repeat(16)))
                .expect("store scope"),
            StoreEpoch::new(1),
        );
        let (store, _) = open_in_memory(identity);
        let (_writer, reader) = store.split();

        let cloned = reader.clone();
        assert_eq!(reader.store_identity(), cloned.store_identity());
    }

    #[test]
    fn commit_failure_selector_is_exact_non_consuming_and_one_shot() {
        let identity = StoreIdentity::new(
            StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "a".repeat(32)))
                .expect("store scope"),
            StoreEpoch::new(1),
        );
        let selected = run_id('b');
        let unrelated = run_id('c');
        let (store, _) = open_in_memory(identity);
        let (writer, _reader) = store.split();

        writer
            .inject_commit_failure(
                selected.clone(),
                BatchPurpose::PureSettlement,
                MemoryCommitFailurePoint::BeforeCommit,
            )
            .expect("arm exact commit failure");
        assert!(writer.commit_failure_is_armed().expect("inspect selector"));
        assert_eq!(
            writer.inject_commit_failure(
                selected.clone(),
                BatchPurpose::PureSettlement,
                MemoryCommitFailurePoint::AfterCommitBeforeAcknowledgement,
            ),
            Err(StoreError::MemoryFailureSelectorAlreadyArmed)
        );
        assert_eq!(
            writer
                .backend()
                .take_commit_failure(&unrelated, BatchPurpose::PureSettlement)
                .expect("unrelated run"),
            None
        );
        assert_eq!(
            writer
                .backend()
                .take_commit_failure(&selected, BatchPurpose::ReadSettlement)
                .expect("unrelated purpose"),
            None
        );
        assert!(writer.commit_failure_is_armed().expect("selector remains"));
        assert_eq!(
            writer
                .backend()
                .take_commit_failure(&selected, BatchPurpose::PureSettlement)
                .expect("matching append"),
            Some(MemoryCommitFailurePoint::BeforeCommit)
        );
        assert!(!writer.commit_failure_is_armed().expect("selector consumed"));
    }

    fn run_id(fill: char) -> RunId {
        RunId::parse(format!("run:sha256-jcs-v1:{}", fill.to_string().repeat(64))).expect("run id")
    }
}
