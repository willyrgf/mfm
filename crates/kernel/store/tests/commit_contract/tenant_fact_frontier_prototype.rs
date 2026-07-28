use super::*;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, TryLockError};
use std::thread;
use std::time::Duration;

const STORE_SCOPE_ID: &str = "mfm.store_scope.v1:prototype";
const STORE_EPOCH: &str = "mfm.store.epoch.v1:prototype";

#[derive(Debug, Clone, PartialEq, Eq)]
struct TenantFactFrontier {
    store_scope_id: String,
    store_epoch: String,
    tenant_scope_id: String,
    fact_order: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PrototypeFactRef {
    source_run_id: String,
    producing_transition_ref: String,
    fact_ordinal: u32,
    content_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublishedFact {
    tenant_scope_id: String,
    fact_order: u64,
    reference: PrototypeFactRef,
    descriptor: String,
    subject: String,
    response: String,
}

impl PublishedFact {
    fn has_valid_content_identity(&self) -> bool {
        self.reference.content_identity
            == content_identity(&self.descriptor, &self.subject, &self.response)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FactDraft {
    descriptor: String,
    subject: String,
    response: String,
}

impl FactDraft {
    fn new(descriptor: &str, subject: &str, response: &str) -> Self {
        Self {
            descriptor: descriptor.to_owned(),
            subject: subject.to_owned(),
            response: response.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FactPublication {
    fact_order: u64,
    source_run_id: String,
    producing_transition_ref: String,
    facts: Vec<PublishedFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FactSelectionBarrier {
    authorization_ref: String,
    frontier: TenantFactFrontier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AdmittedConsumer {
    tenant_scope_id: String,
    run_id: String,
}

impl AdmittedConsumer {
    fn new(tenant_scope_id: &str, run_id: &str) -> Self {
        Self {
            tenant_scope_id: tenant_scope_id.to_owned(),
            run_id: run_id.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FactSelectionQuery {
    descriptor: String,
    subject: String,
    limit: usize,
}

impl FactSelectionQuery {
    fn new(descriptor: &str, subject: &str, limit: usize) -> Self {
        Self {
            descriptor: descriptor.to_owned(),
            subject: subject.to_owned(),
            limit,
        }
    }

    fn digest(&self) -> String {
        format!("{}:{}:{}", self.descriptor, self.subject, self.limit)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FactSelectionResponse {
    request_digest: String,
    frontier: TenantFactFrontier,
    selected: Vec<PrototypeFactRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnverifiedReason {
    PortableBundle,
    PrefixVerificationUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FactSelectionCompleteness {
    SameStoreVerified {
        authorization_ref: String,
        frontier: TenantFactFrontier,
    },
    Unverified {
        reason: UnverifiedReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SameStoreReadPolicy {
    WriterOnly,
    QualifiedReplicaAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistorySourceVerdict {
    AuthoritativeWriter,
    QualifiedReplica,
    LaggingReplica,
    UnprovenReplica,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplicaApplicationProof {
    proof_is_trusted: bool,
    store_scope_id: String,
    store_epoch: String,
    applied_barriers: BTreeSet<String>,
    applied_fact_orders: BTreeMap<String, u64>,
}

impl ReplicaApplicationProof {
    fn qualified_for(barrier: &FactSelectionBarrier) -> Self {
        Self {
            proof_is_trusted: true,
            store_scope_id: barrier.frontier.store_scope_id.clone(),
            store_epoch: barrier.frontier.store_epoch.clone(),
            applied_barriers: BTreeSet::from([barrier.authorization_ref.clone()]),
            applied_fact_orders: BTreeMap::from([(
                barrier.frontier.tenant_scope_id.clone(),
                barrier.frontier.fact_order,
            )]),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HistoryReadSource<'a> {
    AuthoritativeWriter,
    Replica(&'a ReplicaApplicationProof),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentionMeasurement {
    lock_attempts: usize,
    contended_acquisitions: usize,
    maximum_waiters: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeError {
    EmptyPublication,
    CoordinateOverflow,
    SimulatedRollback { candidate_order: u64 },
    LockPoisoned,
    HookDisconnected,
    ResponseBindingMismatch,
    FrontierBindingMismatch,
    FactMissing,
    FactAfterFrontier,
    CurrentRunFact,
    OtherTenantFact,
    OmittedQualifyingFact,
    UnexpectedOrMisorderedFact,
    WriterRequired,
    LaggingReplica,
    UnprovenReplica,
    IncludedFactMissing,
    IncludedFactTampered,
}

#[derive(Default)]
struct TenantHistory {
    current_fact_order: u64,
    publications: Vec<FactPublication>,
}

#[derive(Default)]
struct TenantContention {
    lock_attempts: AtomicUsize,
    contended_acquisitions: AtomicUsize,
    current_waiters: AtomicUsize,
    maximum_waiters: AtomicUsize,
}

struct TenantLane {
    history: Mutex<TenantHistory>,
    contention: TenantContention,
}

impl TenantLane {
    fn new() -> Self {
        Self {
            history: Mutex::new(TenantHistory::default()),
            contention: TenantContention::default(),
        }
    }
}

#[derive(Default)]
struct LockHooks {
    acquired: Option<mpsc::Sender<()>>,
    release: Option<mpsc::Receiver<()>>,
    contended: Option<mpsc::Sender<()>>,
}

struct PrototypeStoreInner {
    tenant_lanes: Mutex<BTreeMap<String, Arc<TenantLane>>>,
    next_authorization_ref: AtomicU64,
}

#[derive(Clone)]
struct PrototypeFactStore {
    inner: Arc<PrototypeStoreInner>,
}

impl PrototypeFactStore {
    fn new() -> Self {
        Self {
            inner: Arc::new(PrototypeStoreInner {
                tenant_lanes: Mutex::new(BTreeMap::new()),
                next_authorization_ref: AtomicU64::new(1),
            }),
        }
    }

    fn publish(
        &self,
        tenant_scope_id: &str,
        source_run_id: &str,
        producing_transition_ref: &str,
        facts: Vec<FactDraft>,
    ) -> Result<FactPublication, PrototypeError> {
        self.publish_with_hooks(
            tenant_scope_id,
            source_run_id,
            producing_transition_ref,
            facts,
            true,
            LockHooks::default(),
        )
    }

    fn publish_with_hooks(
        &self,
        tenant_scope_id: &str,
        source_run_id: &str,
        producing_transition_ref: &str,
        facts: Vec<FactDraft>,
        commit: bool,
        hooks: LockHooks,
    ) -> Result<FactPublication, PrototypeError> {
        if facts.is_empty() {
            return Err(PrototypeError::EmptyPublication);
        }
        self.with_tenant_history(tenant_scope_id, hooks, |history| {
            let fact_order = history
                .current_fact_order
                .checked_add(1)
                .ok_or(PrototypeError::CoordinateOverflow)?;
            if !commit {
                return Err(PrototypeError::SimulatedRollback {
                    candidate_order: fact_order,
                });
            }
            let published = facts
                .into_iter()
                .enumerate()
                .map(|(ordinal, fact)| {
                    let fact_ordinal =
                        u32::try_from(ordinal).map_err(|_| PrototypeError::CoordinateOverflow)?;
                    Ok(PublishedFact {
                        tenant_scope_id: tenant_scope_id.to_owned(),
                        fact_order,
                        reference: PrototypeFactRef {
                            source_run_id: source_run_id.to_owned(),
                            producing_transition_ref: producing_transition_ref.to_owned(),
                            fact_ordinal,
                            content_identity: content_identity(
                                &fact.descriptor,
                                &fact.subject,
                                &fact.response,
                            ),
                        },
                        descriptor: fact.descriptor,
                        subject: fact.subject,
                        response: fact.response,
                    })
                })
                .collect::<Result<Vec<_>, PrototypeError>>()?;
            let publication = FactPublication {
                fact_order,
                source_run_id: source_run_id.to_owned(),
                producing_transition_ref: producing_transition_ref.to_owned(),
                facts: published,
            };
            history.publications.push(publication.clone());
            history.current_fact_order = fact_order;
            Ok(publication)
        })
    }

    fn authorize_selection(
        &self,
        consumer: &AdmittedConsumer,
    ) -> Result<FactSelectionBarrier, PrototypeError> {
        self.authorize_selection_with_hooks(consumer, LockHooks::default())
    }

    fn authorize_selection_with_hooks(
        &self,
        consumer: &AdmittedConsumer,
        hooks: LockHooks,
    ) -> Result<FactSelectionBarrier, PrototypeError> {
        self.with_tenant_history(&consumer.tenant_scope_id, hooks, |history| {
            let authorization_number = self
                .inner
                .next_authorization_ref
                .fetch_add(1, Ordering::Relaxed);
            Ok(FactSelectionBarrier {
                authorization_ref: format!("authorization:{authorization_number}"),
                frontier: TenantFactFrontier {
                    store_scope_id: STORE_SCOPE_ID.to_owned(),
                    store_epoch: STORE_EPOCH.to_owned(),
                    tenant_scope_id: consumer.tenant_scope_id.clone(),
                    fact_order: history.current_fact_order,
                },
            })
        })
    }

    fn response(
        &self,
        consumer: &AdmittedConsumer,
        query: &FactSelectionQuery,
        barrier: &FactSelectionBarrier,
    ) -> Result<FactSelectionResponse, PrototypeError> {
        self.verify_frontier_binding(consumer, &barrier.frontier)?;
        Ok(FactSelectionResponse {
            request_digest: query.digest(),
            frontier: barrier.frontier.clone(),
            selected: self
                .matching_facts(consumer, query, &barrier.frontier)?
                .into_iter()
                .map(|fact| fact.reference)
                .collect(),
        })
    }

    fn verify_same_store(
        &self,
        consumer: &AdmittedConsumer,
        query: &FactSelectionQuery,
        barrier: &FactSelectionBarrier,
        response: &FactSelectionResponse,
        source: HistoryReadSource<'_>,
        policy: SameStoreReadPolicy,
    ) -> Result<FactSelectionCompleteness, PrototypeError> {
        self.verify_response_binding(consumer, query, barrier, response)?;
        self.require_qualified_source(barrier, source, policy)?;
        for reference in &response.selected {
            let fact = self
                .find_fact(reference)?
                .ok_or(PrototypeError::FactMissing)?;
            if fact.tenant_scope_id != consumer.tenant_scope_id {
                return Err(PrototypeError::OtherTenantFact);
            }
            if fact.reference.source_run_id == consumer.run_id {
                return Err(PrototypeError::CurrentRunFact);
            }
            if fact.fact_order > barrier.frontier.fact_order {
                return Err(PrototypeError::FactAfterFrontier);
            }
            if !fact.has_valid_content_identity() {
                return Err(PrototypeError::IncludedFactTampered);
            }
        }

        let expected = self
            .matching_facts(consumer, query, &barrier.frontier)?
            .into_iter()
            .map(|fact| fact.reference)
            .collect::<Vec<_>>();
        if expected
            .iter()
            .any(|reference| !response.selected.contains(reference))
        {
            return Err(PrototypeError::OmittedQualifyingFact);
        }
        if response.selected != expected {
            return Err(PrototypeError::UnexpectedOrMisorderedFact);
        }

        Ok(FactSelectionCompleteness::SameStoreVerified {
            authorization_ref: barrier.authorization_ref.clone(),
            frontier: barrier.frontier.clone(),
        })
    }

    fn verify_portable_inclusions(
        &self,
        consumer: &AdmittedConsumer,
        query: &FactSelectionQuery,
        barrier: &FactSelectionBarrier,
        response: &FactSelectionResponse,
        included_facts: &[PublishedFact],
    ) -> Result<FactSelectionCompleteness, PrototypeError> {
        self.verify_response_binding(consumer, query, barrier, response)?;
        for reference in &response.selected {
            let included = included_facts
                .iter()
                .find(|fact| &fact.reference == reference)
                .ok_or(PrototypeError::IncludedFactMissing)?;
            if included.tenant_scope_id != consumer.tenant_scope_id {
                return Err(PrototypeError::OtherTenantFact);
            }
            if included.reference.source_run_id == consumer.run_id {
                return Err(PrototypeError::CurrentRunFact);
            }
            if included.fact_order > barrier.frontier.fact_order {
                return Err(PrototypeError::FactAfterFrontier);
            }
            if !included.has_valid_content_identity() {
                return Err(PrototypeError::IncludedFactTampered);
            }
        }
        Ok(FactSelectionCompleteness::Unverified {
            reason: UnverifiedReason::PortableBundle,
        })
    }

    fn prefix_unavailable_completeness() -> FactSelectionCompleteness {
        FactSelectionCompleteness::Unverified {
            reason: UnverifiedReason::PrefixVerificationUnavailable,
        }
    }

    fn history_source_verdict(
        &self,
        barrier: &FactSelectionBarrier,
        source: HistoryReadSource<'_>,
    ) -> HistorySourceVerdict {
        match source {
            HistoryReadSource::AuthoritativeWriter => HistorySourceVerdict::AuthoritativeWriter,
            HistoryReadSource::Replica(proof)
                if !proof.proof_is_trusted
                    || proof.store_scope_id != barrier.frontier.store_scope_id
                    || proof.store_epoch != barrier.frontier.store_epoch =>
            {
                HistorySourceVerdict::UnprovenReplica
            }
            HistoryReadSource::Replica(proof)
                if !proof.applied_barriers.contains(&barrier.authorization_ref)
                    || proof
                        .applied_fact_orders
                        .get(&barrier.frontier.tenant_scope_id)
                        .copied()
                        .unwrap_or_default()
                        < barrier.frontier.fact_order =>
            {
                HistorySourceVerdict::LaggingReplica
            }
            HistoryReadSource::Replica(_) => HistorySourceVerdict::QualifiedReplica,
        }
    }

    fn require_qualified_source(
        &self,
        barrier: &FactSelectionBarrier,
        source: HistoryReadSource<'_>,
        policy: SameStoreReadPolicy,
    ) -> Result<HistorySourceVerdict, PrototypeError> {
        let verdict = self.history_source_verdict(barrier, source);
        match (policy, verdict) {
            (_, HistorySourceVerdict::AuthoritativeWriter)
            | (
                SameStoreReadPolicy::QualifiedReplicaAllowed,
                HistorySourceVerdict::QualifiedReplica,
            ) => Ok(verdict),
            (SameStoreReadPolicy::WriterOnly, HistorySourceVerdict::QualifiedReplica) => {
                Err(PrototypeError::WriterRequired)
            }
            (_, HistorySourceVerdict::LaggingReplica) => Err(PrototypeError::LaggingReplica),
            (_, HistorySourceVerdict::UnprovenReplica) => Err(PrototypeError::UnprovenReplica),
        }
    }

    fn verify_response_binding(
        &self,
        consumer: &AdmittedConsumer,
        query: &FactSelectionQuery,
        barrier: &FactSelectionBarrier,
        response: &FactSelectionResponse,
    ) -> Result<(), PrototypeError> {
        self.verify_frontier_binding(consumer, &barrier.frontier)?;
        if response.request_digest != query.digest() || response.frontier != barrier.frontier {
            return Err(PrototypeError::ResponseBindingMismatch);
        }
        Ok(())
    }

    fn verify_frontier_binding(
        &self,
        consumer: &AdmittedConsumer,
        frontier: &TenantFactFrontier,
    ) -> Result<(), PrototypeError> {
        if frontier.store_scope_id != STORE_SCOPE_ID
            || frontier.store_epoch != STORE_EPOCH
            || frontier.tenant_scope_id != consumer.tenant_scope_id
        {
            return Err(PrototypeError::FrontierBindingMismatch);
        }
        Ok(())
    }

    fn matching_facts(
        &self,
        consumer: &AdmittedConsumer,
        query: &FactSelectionQuery,
        frontier: &TenantFactFrontier,
    ) -> Result<Vec<PublishedFact>, PrototypeError> {
        let lane = self.tenant_lane(&consumer.tenant_scope_id)?;
        let history = lane
            .history
            .lock()
            .map_err(|_| PrototypeError::LockPoisoned)?;
        let mut matches = history
            .publications
            .iter()
            .filter(|publication| publication.fact_order <= frontier.fact_order)
            .flat_map(|publication| publication.facts.iter())
            .filter(|fact| {
                fact.reference.source_run_id != consumer.run_id
                    && fact.descriptor == query.descriptor
                    && fact.subject == query.subject
            })
            .cloned()
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            left.fact_order
                .cmp(&right.fact_order)
                .then_with(|| left.reference.cmp(&right.reference))
        });
        matches.truncate(query.limit);
        Ok(matches)
    }

    fn find_fact(
        &self,
        reference: &PrototypeFactRef,
    ) -> Result<Option<PublishedFact>, PrototypeError> {
        let lanes = self
            .inner
            .tenant_lanes
            .lock()
            .map_err(|_| PrototypeError::LockPoisoned)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for lane in lanes {
            let history = lane
                .history
                .lock()
                .map_err(|_| PrototypeError::LockPoisoned)?;
            if let Some(fact) = history
                .publications
                .iter()
                .flat_map(|publication| publication.facts.iter())
                .find(|fact| &fact.reference == reference)
            {
                return Ok(Some(fact.clone()));
            }
        }
        Ok(None)
    }

    fn current_fact_order(&self, tenant_scope_id: &str) -> Result<u64, PrototypeError> {
        let lane = self.tenant_lane(tenant_scope_id)?;
        let history = lane
            .history
            .lock()
            .map_err(|_| PrototypeError::LockPoisoned)?;
        Ok(history.current_fact_order)
    }

    fn publication_orders(&self, tenant_scope_id: &str) -> Result<Vec<u64>, PrototypeError> {
        let lane = self.tenant_lane(tenant_scope_id)?;
        let history = lane
            .history
            .lock()
            .map_err(|_| PrototypeError::LockPoisoned)?;
        Ok(history
            .publications
            .iter()
            .map(|publication| publication.fact_order)
            .collect())
    }

    fn contention_measurement(
        &self,
        tenant_scope_id: &str,
    ) -> Result<ContentionMeasurement, PrototypeError> {
        let lane = self.tenant_lane(tenant_scope_id)?;
        Ok(ContentionMeasurement {
            lock_attempts: lane.contention.lock_attempts.load(Ordering::Relaxed),
            contended_acquisitions: lane
                .contention
                .contended_acquisitions
                .load(Ordering::Relaxed),
            maximum_waiters: lane.contention.maximum_waiters.load(Ordering::Relaxed),
        })
    }

    fn with_tenant_history<T>(
        &self,
        tenant_scope_id: &str,
        hooks: LockHooks,
        operation: impl FnOnce(&mut TenantHistory) -> Result<T, PrototypeError>,
    ) -> Result<T, PrototypeError> {
        let lane = self.tenant_lane(tenant_scope_id)?;
        lane.contention
            .lock_attempts
            .fetch_add(1, Ordering::Relaxed);
        let mut history = match lane.history.try_lock() {
            Ok(history) => history,
            Err(TryLockError::WouldBlock) => {
                lane.contention
                    .contended_acquisitions
                    .fetch_add(1, Ordering::Relaxed);
                let waiters = lane
                    .contention
                    .current_waiters
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                lane.contention
                    .maximum_waiters
                    .fetch_max(waiters, Ordering::Relaxed);
                if let Some(contended) = hooks.contended.as_ref() {
                    contended
                        .send(())
                        .map_err(|_| PrototypeError::HookDisconnected)?;
                }
                let history = lane
                    .history
                    .lock()
                    .map_err(|_| PrototypeError::LockPoisoned)?;
                lane.contention
                    .current_waiters
                    .fetch_sub(1, Ordering::Relaxed);
                history
            }
            Err(TryLockError::Poisoned(_)) => return Err(PrototypeError::LockPoisoned),
        };
        run_lock_hooks(hooks)?;
        operation(&mut history)
    }

    fn tenant_lane(&self, tenant_scope_id: &str) -> Result<Arc<TenantLane>, PrototypeError> {
        let mut lanes = self
            .inner
            .tenant_lanes
            .lock()
            .map_err(|_| PrototypeError::LockPoisoned)?;
        Ok(Arc::clone(
            lanes
                .entry(tenant_scope_id.to_owned())
                .or_insert_with(|| Arc::new(TenantLane::new())),
        ))
    }
}

fn run_lock_hooks(hooks: LockHooks) -> Result<(), PrototypeError> {
    if let Some(acquired) = hooks.acquired {
        acquired
            .send(())
            .map_err(|_| PrototypeError::HookDisconnected)?;
    }
    if let Some(release) = hooks.release {
        release
            .recv()
            .map_err(|_| PrototypeError::HookDisconnected)?;
    }
    Ok(())
}

fn content_identity(descriptor: &str, subject: &str, response: &str) -> String {
    format!("{descriptor}|{subject}|{response}")
}

fn one_fact(response: &str) -> Vec<FactDraft> {
    vec![FactDraft::new("balance", "wallet", response)]
}

fn hooks_holding_lock() -> (LockHooks, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    (
        LockHooks {
            acquired: Some(acquired_tx),
            release: Some(release_rx),
            contended: None,
        },
        acquired_rx,
        release_tx,
    )
}

fn contended_hooks(contended: mpsc::Sender<()>) -> LockHooks {
    LockHooks {
        contended: Some(contended),
        ..LockHooks::default()
    }
}

fn receive_signal(receiver: &mpsc::Receiver<()>, label: &str) {
    receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| panic!("{label} did not reach the expected lock boundary"));
}

#[test]
fn same_tenant_publication_and_barrier_are_totally_ordered() {
    let store = PrototypeFactStore::new();
    let consumer = AdmittedConsumer::new("tenant-publication-first", "consumer");
    let (publish_hooks, publish_acquired, release_publish) = hooks_holding_lock();
    let publishing_store = store.clone();
    let publication = thread::spawn(move || {
        publishing_store.publish_with_hooks(
            "tenant-publication-first",
            "producer",
            "transition:1",
            one_fact("10"),
            true,
            publish_hooks,
        )
    });
    receive_signal(&publish_acquired, "publication");

    let (contended_tx, contended_rx) = mpsc::channel();
    let barrier_store = store.clone();
    let barrier_consumer = consumer.clone();
    let barrier = thread::spawn(move || {
        barrier_store
            .authorize_selection_with_hooks(&barrier_consumer, contended_hooks(contended_tx))
    });
    receive_signal(&contended_rx, "barrier");
    release_publish.send(()).expect("release publication");
    let publication = publication
        .join()
        .expect("publication thread")
        .expect("publish");
    let barrier = barrier.join().expect("barrier thread").expect("barrier");
    assert_eq!(publication.fact_order, 1);
    assert_eq!(barrier.frontier.fact_order, 1);

    let barrier_first_consumer = AdmittedConsumer::new("tenant-barrier-first", "consumer");
    let (barrier_hooks, barrier_acquired, release_barrier) = hooks_holding_lock();
    let barrier_store = store.clone();
    let held_consumer = barrier_first_consumer.clone();
    let barrier = thread::spawn(move || {
        barrier_store.authorize_selection_with_hooks(&held_consumer, barrier_hooks)
    });
    receive_signal(&barrier_acquired, "barrier");

    let (contended_tx, contended_rx) = mpsc::channel();
    let publishing_store = store.clone();
    let publication = thread::spawn(move || {
        publishing_store.publish_with_hooks(
            "tenant-barrier-first",
            "producer",
            "transition:1",
            one_fact("10"),
            true,
            contended_hooks(contended_tx),
        )
    });
    receive_signal(&contended_rx, "publication");
    release_barrier.send(()).expect("release barrier");
    let barrier = barrier.join().expect("barrier thread").expect("barrier");
    let publication = publication
        .join()
        .expect("publication thread")
        .expect("publish");
    assert_eq!(barrier.frontier.fact_order, 0);
    assert_eq!(publication.fact_order, 1);
}

#[test]
fn different_tenants_have_independent_heads_and_lock_contention() {
    let store = PrototypeFactStore::new();
    let (hot_hooks, hot_acquired, release_hot) = hooks_holding_lock();
    let hot_store = store.clone();
    let hot = thread::spawn(move || {
        hot_store.publish_with_hooks(
            "tenant-hot",
            "hot-run",
            "transition:hot",
            one_fact("1"),
            true,
            hot_hooks,
        )
    });
    receive_signal(&hot_acquired, "hot tenant publication");

    let (cold_done_tx, cold_done_rx) = mpsc::channel();
    let cold_store = store.clone();
    let cold = thread::spawn(move || {
        let result =
            cold_store.publish("tenant-cold", "cold-run", "transition:cold", one_fact("1"));
        cold_done_tx.send(()).expect("report cold completion");
        result
    });
    receive_signal(&cold_done_rx, "cold tenant publication");
    release_hot.send(()).expect("release hot tenant");

    let hot = hot.join().expect("hot thread").expect("hot publication");
    let cold = cold.join().expect("cold thread").expect("cold publication");
    assert_eq!(hot.fact_order, 1);
    assert_eq!(cold.fact_order, 1);
    assert_eq!(
        store
            .contention_measurement("tenant-cold")
            .expect("cold measurement")
            .contended_acquisitions,
        0
    );
}

#[test]
fn publication_order_is_dense_rollback_free_and_multi_fact_per_coordinate() {
    let store = PrototypeFactStore::new();
    let first = store
        .publish(
            "tenant",
            "producer-a",
            "transition:multi",
            vec![
                FactDraft::new("balance", "wallet-a", "1"),
                FactDraft::new("balance", "wallet-b", "2"),
                FactDraft::new("balance", "wallet-c", "3"),
            ],
        )
        .expect("multi-fact publication");
    assert_eq!(first.fact_order, 1);
    assert_eq!(first.facts.len(), 3);
    assert!(first.facts.iter().all(|fact| fact.fact_order == 1));
    assert_eq!(
        first
            .facts
            .iter()
            .map(|fact| fact.reference.fact_ordinal)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    let rollback = store.publish_with_hooks(
        "tenant",
        "producer-b",
        "transition:rollback",
        one_fact("4"),
        false,
        LockHooks::default(),
    );
    assert_eq!(
        rollback,
        Err(PrototypeError::SimulatedRollback { candidate_order: 2 })
    );
    let second = store
        .publish("tenant", "producer-b", "transition:2", one_fact("4"))
        .expect("second publication");
    let third = store
        .publish("tenant", "producer-c", "transition:3", one_fact("5"))
        .expect("third publication");
    assert_eq!(second.fact_order, 2);
    assert_eq!(third.fact_order, 3);
    assert_eq!(
        store.publication_orders("tenant").expect("orders"),
        vec![1, 2, 3]
    );
    assert_eq!(store.current_fact_order("tenant").expect("head"), 3);
}

#[test]
fn a_barrier_only_tenant_has_zero_frontier_and_verified_empty_selection() {
    let store = PrototypeFactStore::new();
    let consumer = AdmittedConsumer::new("empty-tenant", "consumer");
    let query = FactSelectionQuery::new("balance", "wallet", 10);
    let barrier = store.authorize_selection(&consumer).expect("zero barrier");
    assert_eq!(barrier.frontier.fact_order, 0);
    assert_eq!(
        store.current_fact_order("empty-tenant").expect("zero head"),
        0
    );
    let response = store
        .response(&consumer, &query, &barrier)
        .expect("response");
    assert!(response.selected.is_empty());
    assert_eq!(
        store
            .verify_same_store(
                &consumer,
                &query,
                &barrier,
                &response,
                HistoryReadSource::AuthoritativeWriter,
                SameStoreReadPolicy::WriterOnly,
            )
            .expect("verified empty response"),
        FactSelectionCompleteness::SameStoreVerified {
            authorization_ref: barrier.authorization_ref.clone(),
            frontier: barrier.frontier.clone(),
        }
    );
}

#[test]
fn hot_tenant_structural_contention_is_scoped_and_measured() {
    const CONTENDERS: usize = 8;

    let store = PrototypeFactStore::new();
    let hot_consumer = AdmittedConsumer::new("hot-tenant", "consumer");
    let (held_hooks, held_acquired, release_held) = hooks_holding_lock();
    let held_store = store.clone();
    let held =
        thread::spawn(move || held_store.authorize_selection_with_hooks(&hot_consumer, held_hooks));
    receive_signal(&held_acquired, "held hot-tenant barrier");

    let (contended_tx, contended_rx) = mpsc::channel();
    let mut contenders = Vec::new();
    for index in 0..CONTENDERS {
        let contender_store = store.clone();
        let signal = contended_tx.clone();
        contenders.push(thread::spawn(move || {
            contender_store.publish_with_hooks(
                "hot-tenant",
                &format!("producer-{index}"),
                &format!("transition:{index}"),
                one_fact(&index.to_string()),
                true,
                contended_hooks(signal),
            )
        }));
    }
    drop(contended_tx);
    for _ in 0..CONTENDERS {
        receive_signal(&contended_rx, "hot-tenant contender");
    }

    for index in 0..CONTENDERS {
        let tenant = format!("cold-tenant-{index}");
        store
            .publish(&tenant, "cold-producer", "transition:cold", one_fact("1"))
            .expect("cold publication");
        assert_eq!(
            store
                .contention_measurement(&tenant)
                .expect("cold measurement")
                .contended_acquisitions,
            0
        );
    }

    let measured_while_held = store
        .contention_measurement("hot-tenant")
        .expect("hot measurement");
    assert_eq!(measured_while_held.lock_attempts, CONTENDERS + 1);
    assert_eq!(measured_while_held.contended_acquisitions, CONTENDERS);
    assert_eq!(measured_while_held.maximum_waiters, CONTENDERS);

    release_held.send(()).expect("release hot barrier");
    assert_eq!(
        held.join()
            .expect("held barrier thread")
            .expect("held barrier")
            .frontier
            .fact_order,
        0
    );
    for contender in contenders {
        contender
            .join()
            .expect("contender thread")
            .expect("contender publication");
    }
    assert_eq!(
        store
            .publication_orders("hot-tenant")
            .expect("hot publication orders"),
        (1..=u64::try_from(CONTENDERS).expect("contender count fits")).collect::<Vec<_>>()
    );
}

#[test]
fn writer_only_and_replica_application_verdicts_fail_closed() {
    let store = PrototypeFactStore::new();
    let consumer = AdmittedConsumer::new("tenant", "consumer");
    store
        .publish("tenant", "producer", "transition:1", one_fact("1"))
        .expect("publication");
    let barrier = store.authorize_selection(&consumer).expect("barrier");
    let query = FactSelectionQuery::new("balance", "wallet", 10);
    let response = store
        .response(&consumer, &query, &barrier)
        .expect("response");

    let qualified = ReplicaApplicationProof::qualified_for(&barrier);
    let mut lagging = qualified.clone();
    lagging
        .applied_fact_orders
        .insert("tenant".to_owned(), barrier.frontier.fact_order - 1);
    let mut missing_barrier = qualified.clone();
    missing_barrier.applied_barriers.clear();
    let mut unproven = qualified.clone();
    unproven.proof_is_trusted = false;
    let mut foreign_lineage = qualified.clone();
    foreign_lineage.store_epoch = "mfm.store.epoch.v1:foreign".to_owned();

    assert_eq!(
        store.history_source_verdict(&barrier, HistoryReadSource::AuthoritativeWriter),
        HistorySourceVerdict::AuthoritativeWriter
    );
    assert_eq!(
        store.history_source_verdict(&barrier, HistoryReadSource::Replica(&qualified)),
        HistorySourceVerdict::QualifiedReplica
    );
    assert_eq!(
        store.history_source_verdict(&barrier, HistoryReadSource::Replica(&lagging)),
        HistorySourceVerdict::LaggingReplica
    );
    assert_eq!(
        store.history_source_verdict(&barrier, HistoryReadSource::Replica(&missing_barrier)),
        HistorySourceVerdict::LaggingReplica
    );
    assert_eq!(
        store.history_source_verdict(&barrier, HistoryReadSource::Replica(&unproven)),
        HistorySourceVerdict::UnprovenReplica
    );
    assert_eq!(
        store.history_source_verdict(&barrier, HistoryReadSource::Replica(&foreign_lineage)),
        HistorySourceVerdict::UnprovenReplica
    );

    store
        .verify_same_store(
            &consumer,
            &query,
            &barrier,
            &response,
            HistoryReadSource::AuthoritativeWriter,
            SameStoreReadPolicy::WriterOnly,
        )
        .expect("writer-only baseline accepts the writer");
    assert_eq!(
        store.verify_same_store(
            &consumer,
            &query,
            &barrier,
            &response,
            HistoryReadSource::Replica(&qualified),
            SameStoreReadPolicy::WriterOnly,
        ),
        Err(PrototypeError::WriterRequired)
    );
    store
        .verify_same_store(
            &consumer,
            &query,
            &barrier,
            &response,
            HistoryReadSource::Replica(&qualified),
            SameStoreReadPolicy::QualifiedReplicaAllowed,
        )
        .expect("qualified replica is accepted only by the explicit policy");
    assert_eq!(
        store.verify_same_store(
            &consumer,
            &query,
            &barrier,
            &response,
            HistoryReadSource::Replica(&lagging),
            SameStoreReadPolicy::QualifiedReplicaAllowed,
        ),
        Err(PrototypeError::LaggingReplica)
    );
    assert_eq!(
        store.verify_same_store(
            &consumer,
            &query,
            &barrier,
            &response,
            HistoryReadSource::Replica(&unproven),
            SameStoreReadPolicy::QualifiedReplicaAllowed,
        ),
        Err(PrototypeError::UnprovenReplica)
    );
}

#[test]
fn same_store_verification_rejects_omission_current_run_and_other_tenant() {
    let store = PrototypeFactStore::new();
    let consumer = AdmittedConsumer::new("tenant-a", "consumer");
    let multi = store
        .publish(
            "tenant-a",
            "producer",
            "transition:multi",
            vec![
                FactDraft::new("balance", "wallet", "1"),
                FactDraft::new("balance", "wallet", "2"),
            ],
        )
        .expect("multi-fact publication");
    let current_run = store
        .publish("tenant-a", "consumer", "transition:current", one_fact("3"))
        .expect("current-run publication");
    let other_tenant = store
        .publish(
            "tenant-b",
            "other-producer",
            "transition:other-tenant",
            one_fact("4"),
        )
        .expect("other-tenant publication");
    let barrier = store.authorize_selection(&consumer).expect("barrier");
    let query = FactSelectionQuery::new("balance", "wallet", 10);
    let response = store
        .response(&consumer, &query, &barrier)
        .expect("response");
    assert_eq!(
        response.selected,
        multi
            .facts
            .iter()
            .map(|fact| fact.reference.clone())
            .collect::<Vec<_>>()
    );
    store
        .verify_same_store(
            &consumer,
            &query,
            &barrier,
            &response,
            HistoryReadSource::AuthoritativeWriter,
            SameStoreReadPolicy::WriterOnly,
        )
        .expect("complete same-store response");

    let mut omitted = response.clone();
    omitted.selected.pop();
    assert_eq!(
        store.verify_same_store(
            &consumer,
            &query,
            &barrier,
            &omitted,
            HistoryReadSource::AuthoritativeWriter,
            SameStoreReadPolicy::WriterOnly,
        ),
        Err(PrototypeError::OmittedQualifyingFact)
    );

    let mut injected_current = response.clone();
    injected_current
        .selected
        .push(current_run.facts[0].reference.clone());
    assert_eq!(
        store.verify_same_store(
            &consumer,
            &query,
            &barrier,
            &injected_current,
            HistoryReadSource::AuthoritativeWriter,
            SameStoreReadPolicy::WriterOnly,
        ),
        Err(PrototypeError::CurrentRunFact)
    );

    let mut injected_other_tenant = response.clone();
    injected_other_tenant
        .selected
        .push(other_tenant.facts[0].reference.clone());
    assert_eq!(
        store.verify_same_store(
            &consumer,
            &query,
            &barrier,
            &injected_other_tenant,
            HistoryReadSource::AuthoritativeWriter,
            SameStoreReadPolicy::WriterOnly,
        ),
        Err(PrototypeError::OtherTenantFact)
    );

    let mut other_tenant_barrier = barrier;
    other_tenant_barrier.frontier.tenant_scope_id = "tenant-b".to_owned();
    let mut response_with_other_tenant_frontier = response;
    response_with_other_tenant_frontier.frontier = other_tenant_barrier.frontier.clone();
    assert_eq!(
        store.verify_same_store(
            &consumer,
            &query,
            &other_tenant_barrier,
            &response_with_other_tenant_frontier,
            HistoryReadSource::AuthoritativeWriter,
            SameStoreReadPolicy::WriterOnly,
        ),
        Err(PrototypeError::FrontierBindingMismatch)
    );
}

#[test]
fn portable_bundle_verifies_inclusions_but_never_completeness() {
    let store = PrototypeFactStore::new();
    let consumer = AdmittedConsumer::new("tenant", "consumer");
    let publication = store
        .publish(
            "tenant",
            "producer",
            "transition:multi",
            vec![
                FactDraft::new("balance", "wallet", "1"),
                FactDraft::new("balance", "wallet", "2"),
            ],
        )
        .expect("publication");
    let barrier = store.authorize_selection(&consumer).expect("barrier");
    let query = FactSelectionQuery::new("balance", "wallet", 10);
    let incomplete = FactSelectionResponse {
        request_digest: query.digest(),
        frontier: barrier.frontier.clone(),
        selected: vec![publication.facts[0].reference.clone()],
    };
    assert_eq!(
        store.verify_same_store(
            &consumer,
            &query,
            &barrier,
            &incomplete,
            HistoryReadSource::AuthoritativeWriter,
            SameStoreReadPolicy::WriterOnly,
        ),
        Err(PrototypeError::OmittedQualifyingFact)
    );
    assert_eq!(
        store
            .verify_portable_inclusions(
                &consumer,
                &query,
                &barrier,
                &incomplete,
                &publication.facts[..1],
            )
            .expect("portable inclusion verification"),
        FactSelectionCompleteness::Unverified {
            reason: UnverifiedReason::PortableBundle,
        }
    );
    assert_eq!(
        PrototypeFactStore::prefix_unavailable_completeness(),
        FactSelectionCompleteness::Unverified {
            reason: UnverifiedReason::PrefixVerificationUnavailable,
        }
    );

    let mut tampered = publication.facts[0].clone();
    tampered.response = "tampered".to_owned();
    assert_eq!(
        store.verify_portable_inclusions(&consumer, &query, &barrier, &incomplete, &[tampered],),
        Err(PrototypeError::IncludedFactTampered)
    );
}
