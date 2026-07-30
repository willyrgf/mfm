use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex};

use mfm_canonical::{
    sha256_digest_bytes, CanonicalValue, RecoverabilityContract, ValidatedCanonicalValue,
};
use mfm_executor::{
    reference_safe_failure, verify_ensure_result, AccountSequencePolicy, AccountSequenceRequest,
    AllocationOutcome, BoundaryStage, CommittedEffectRequest, ContentRef, DeliveryAttemptOutcome,
    EffectExecutorOutcome, EffectExecutorOutcomeParts, EffectExecutorOutcomeView, EffectIdentity,
    Ensure, EvidenceBounds, ExecuteTargetOutcome, ExecutorAppendOutcome, ExecutorBinding,
    ExecutorContractDescriptor, ExecutorDeployment, ExecutorEffectSnapshot,
    ExecutorEnsureResultClaim, ExecutorError, ExecutorEvidenceRecord, ExecutorFuture,
    ExecutorLedgerAppend, ExecutorLedgerStore, ExecutorLedgerStoreIdentity,
    ExecutorResourceSnapshot, ExecutorRetainedClosureClaim, ExecutorRetainedClosureContract,
    ExecutorRetainedValue, ExecutorRetainedValueRelation, ExecutorStoreSnapshot,
    ExecutorTerminalEvidenceClaim, FailureClass, FencingRef, FiniteInventoryPolicy,
    FiniteInventoryRequest, KeyedExecutorLedger, MemoryConvergentDestination,
    MemoryDestinationCheckpoint, MemoryExecutorStore, MemoryLedgerCheckpoint, NonDomainDisposition,
    NonDomainEntryStatus, NonDomainFailure, NonDomainFailureCode, ReferenceContract,
    ReferenceCrashPoint, ReferenceDestination, ReferenceDestinationReturn, ReferenceDriveOutcome,
    ReferenceExecutor, ReferenceFailureCode, ReferenceRequest, ReferenceTargetBehavior,
    ReferenceTerminalProof, ResourceKeyRef, ResourceLedgerRecord, ResourceOwnership,
    ResourceOwnershipRef, ResourcePolicyBinding, RetainedValueContract,
    SchemaQualifiedCanonicalValue, TargetEntryAuthority, TerminalTombstone, TypedResourcePolicy,
    VerifiedExecutorBinding,
};
use mfm_ids::{
    AttemptId, DigestAlgorithm, NodeId, RunId, SemanticTypeId, StableId, StoreScopeId,
    TenantScopeId,
};

#[path = "../../../../tests/support/recoverability_v1.rs"]
mod recoverability_v1;

const CORPUS: &str = include_str!("../../../../contracts/recoverability/v1/corpus.json");

#[derive(Clone)]
struct RetryingObservationStore {
    inner: MemoryExecutorStore,
    return_absent_once: Arc<AtomicBool>,
    observation_failures: Arc<AtomicUsize>,
    observation_attempts: Arc<AtomicUsize>,
}

impl RetryingObservationStore {
    fn new(inner: MemoryExecutorStore, observation_failures: usize) -> Self {
        Self {
            inner,
            return_absent_once: Arc::new(AtomicBool::new(false)),
            observation_failures: Arc::new(AtomicUsize::new(observation_failures)),
            observation_attempts: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn return_absent_on_next_load(&self) {
        self.return_absent_once.store(true, Ordering::SeqCst);
    }

    fn observation_attempts(&self) -> usize {
        self.observation_attempts.load(Ordering::SeqCst)
    }
}

impl ExecutorLedgerStore for RetryingObservationStore {
    fn store_identity(&self) -> &ExecutorLedgerStoreIdentity {
        self.inner.store_identity()
    }

    fn load_effect<'a>(
        &'a self,
        effect_key: &'a mfm_executor::EffectKey,
    ) -> ExecutorFuture<'a, std::result::Result<Option<ExecutorEffectSnapshot>, ExecutorError>>
    {
        if self.return_absent_once.swap(false, Ordering::SeqCst) {
            Box::pin(async { Ok(None) })
        } else {
            self.inner.load_effect(effect_key)
        }
    }

    fn load_resource<'a>(
        &'a self,
        resource_ownership_ref: &'a ResourceOwnershipRef,
        resource_key_ref: &'a ResourceKeyRef,
    ) -> ExecutorFuture<'a, std::result::Result<ExecutorResourceSnapshot, ExecutorError>> {
        self.inner
            .load_resource(resource_ownership_ref, resource_key_ref)
    }

    fn load_content<'a>(
        &'a self,
        content_ref: &'a ContentRef,
    ) -> ExecutorFuture<'a, std::result::Result<Option<SchemaQualifiedCanonicalValue>, ExecutorError>>
    {
        self.inner.load_content(content_ref)
    }

    fn compare_and_append<'a>(
        &'a self,
        append: ExecutorLedgerAppend,
    ) -> ExecutorFuture<'a, std::result::Result<mfm_executor::ExecutorAppendOutcome, ExecutorError>>
    {
        let is_observation = append
            .effect_frontier()
            .appended_records()
            .iter()
            .any(|record| {
                matches!(
                    record,
                    ExecutorEvidenceRecord::DeliveryAttemptObserved { .. }
                )
            });
        if is_observation {
            self.observation_attempts.fetch_add(1, Ordering::SeqCst);
            if self
                .observation_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Box::pin(async { Err(ExecutorError::InvalidDurableSnapshot) });
            }
        }
        self.inner.compare_and_append(append)
    }
}

#[derive(Clone)]
struct AlwaysConflictingStore {
    inner: MemoryExecutorStore,
    append_attempts: Arc<AtomicUsize>,
}

impl AlwaysConflictingStore {
    fn new(inner: MemoryExecutorStore) -> Self {
        Self {
            inner,
            append_attempts: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn append_attempts(&self) -> usize {
        self.append_attempts.load(Ordering::SeqCst)
    }
}

impl ExecutorLedgerStore for AlwaysConflictingStore {
    fn store_identity(&self) -> &ExecutorLedgerStoreIdentity {
        self.inner.store_identity()
    }

    fn load_effect<'a>(
        &'a self,
        effect_key: &'a mfm_executor::EffectKey,
    ) -> ExecutorFuture<'a, std::result::Result<Option<ExecutorEffectSnapshot>, ExecutorError>>
    {
        self.inner.load_effect(effect_key)
    }

    fn load_resource<'a>(
        &'a self,
        resource_ownership_ref: &'a ResourceOwnershipRef,
        resource_key_ref: &'a ResourceKeyRef,
    ) -> ExecutorFuture<'a, std::result::Result<ExecutorResourceSnapshot, ExecutorError>> {
        self.inner
            .load_resource(resource_ownership_ref, resource_key_ref)
    }

    fn load_content<'a>(
        &'a self,
        content_ref: &'a ContentRef,
    ) -> ExecutorFuture<'a, std::result::Result<Option<SchemaQualifiedCanonicalValue>, ExecutorError>>
    {
        self.inner.load_content(content_ref)
    }

    fn compare_and_append<'a>(
        &'a self,
        _append: ExecutorLedgerAppend,
    ) -> ExecutorFuture<'a, std::result::Result<mfm_executor::ExecutorAppendOutcome, ExecutorError>>
    {
        self.append_attempts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(mfm_executor::ExecutorAppendOutcome::Conflict) })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalResolutionEvidence {
    Identical,
    Conflicting,
}

const FINAL_RESOLUTION_CASES: [(&str, ExecutorAppendOutcome, FinalResolutionEvidence); 4] = [
    (
        "applied",
        ExecutorAppendOutcome::Applied,
        FinalResolutionEvidence::Identical,
    ),
    (
        "already-applied",
        ExecutorAppendOutcome::AlreadyApplied,
        FinalResolutionEvidence::Identical,
    ),
    (
        "conflict-identical",
        ExecutorAppendOutcome::Conflict,
        FinalResolutionEvidence::Identical,
    ),
    (
        "conflict-conflicting",
        ExecutorAppendOutcome::Conflict,
        FinalResolutionEvidence::Conflicting,
    ),
];

#[derive(Clone)]
struct ScriptedFinalResolutionStore {
    before: MemoryExecutorStore,
    after: MemoryExecutorStore,
    after_effect_override: Option<ExecutorEffectSnapshot>,
    final_outcome: ExecutorAppendOutcome,
    append_attempts: Arc<AtomicUsize>,
}

impl ScriptedFinalResolutionStore {
    fn new(
        before: MemoryExecutorStore,
        after: MemoryExecutorStore,
        final_outcome: ExecutorAppendOutcome,
    ) -> Self {
        assert_eq!(before.store_identity(), after.store_identity());
        Self {
            before,
            after,
            after_effect_override: None,
            final_outcome,
            append_attempts: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn with_after_effect_override(mut self, snapshot: ExecutorEffectSnapshot) -> Self {
        self.after_effect_override = Some(snapshot);
        self
    }

    fn append_attempts(&self) -> usize {
        self.append_attempts.load(Ordering::SeqCst)
    }

    fn resolution_visible(&self) -> bool {
        self.append_attempts() >= 64
    }

    fn visible_store(&self) -> &MemoryExecutorStore {
        if self.resolution_visible() {
            &self.after
        } else {
            &self.before
        }
    }
}

impl ExecutorLedgerStore for ScriptedFinalResolutionStore {
    fn store_identity(&self) -> &ExecutorLedgerStoreIdentity {
        self.before.store_identity()
    }

    fn load_effect<'a>(
        &'a self,
        effect_key: &'a mfm_executor::EffectKey,
    ) -> ExecutorFuture<'a, std::result::Result<Option<ExecutorEffectSnapshot>, ExecutorError>>
    {
        if self.resolution_visible() {
            if let Some(snapshot) = &self.after_effect_override {
                let snapshot = snapshot.clone();
                return Box::pin(async move { Ok(Some(snapshot)) });
            }
        }
        self.visible_store().load_effect(effect_key)
    }

    fn load_resource<'a>(
        &'a self,
        resource_ownership_ref: &'a ResourceOwnershipRef,
        resource_key_ref: &'a ResourceKeyRef,
    ) -> ExecutorFuture<'a, std::result::Result<ExecutorResourceSnapshot, ExecutorError>> {
        self.visible_store()
            .load_resource(resource_ownership_ref, resource_key_ref)
    }

    fn load_content<'a>(
        &'a self,
        content_ref: &'a ContentRef,
    ) -> ExecutorFuture<'a, std::result::Result<Option<SchemaQualifiedCanonicalValue>, ExecutorError>>
    {
        self.visible_store().load_content(content_ref)
    }

    fn compare_and_append<'a>(
        &'a self,
        _append: ExecutorLedgerAppend,
    ) -> ExecutorFuture<'a, std::result::Result<ExecutorAppendOutcome, ExecutorError>> {
        let attempt = self.append_attempts.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(
            attempt <= 64,
            "the engine must never issue a 65th local CAS"
        );
        let outcome = if attempt == 64 {
            self.final_outcome
        } else {
            ExecutorAppendOutcome::Conflict
        };
        Box::pin(async move { Ok(outcome) })
    }
}

#[derive(Clone)]
struct BindingFixture {
    binding: VerifiedExecutorBinding,
    tenant_scope_id: TenantScopeId,
    generation_ref: ContentRef,
    destination_domain_ref: ContentRef,
}

fn contract() -> &'static RecoverabilityContract {
    RecoverabilityContract::embedded().expect("embedded contract")
}

fn reviewed_ref(label: &str) -> ContentRef {
    reviewed_value(label).reference().expect("content ref")
}

fn reviewed_value(label: &str) -> SchemaQualifiedCanonicalValue {
    let value = contract()
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String(label.to_owned()),
        )
        .expect("reviewed value");
    SchemaQualifiedCanonicalValue::from_validated(&value).expect("schema-qualified value")
}

fn retained_contract(label: &str, schema_contract: &str) -> RetainedValueContract {
    RetainedValueContract::new(
        contract()
            .schema_id(schema_contract)
            .expect("retained schema")
            .clone(),
        SemanticTypeId::new(
            "mfm",
            label,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(label.as_bytes()),
        )
        .expect("semantic type"),
        StableId::new(label).expect("retained role"),
        "application/json",
        reviewed_ref(&format!("{label}.evidence")),
    )
    .expect("retained value contract")
}

fn retained_closure_contract_with_domain(
    label: &str,
    domain_evidence_schema_contract: &str,
) -> ExecutorRetainedClosureContract {
    ExecutorRetainedClosureContract::new(
        retained_contract(
            &format!("{label}.ensure-result"),
            "mfm.executor-ensure-result.v1",
        ),
        retained_contract(
            &format!("{label}.delivery-audit"),
            "mfm.executor-delivery-frontier.v1",
        ),
        retained_contract(
            &format!("{label}.executor-frontier"),
            "mfm.executor-delivery-frontier.v1",
        ),
        retained_contract(
            &format!("{label}.terminal-evidence"),
            "mfm.terminal-effect-evidence.v1",
        ),
        retained_contract(
            &format!("{label}.terminal-tombstone"),
            "mfm.executor-terminal-tombstone.v1",
        ),
        retained_contract(
            &format!("{label}.terminal-proof"),
            "mfm.executor-reference-terminal-proof.v1",
        ),
        retained_contract(
            &format!("{label}.domain-evidence"),
            domain_evidence_schema_contract,
        ),
    )
    .expect("retained closure contract")
}

fn binding_fixture(label: &str, with_resource_owner: bool, max_attempts: u32) -> BindingFixture {
    binding_fixture_with_bounds(label, with_resource_owner, bounds(max_attempts))
}

fn binding_fixture_with_bounds(
    label: &str,
    with_resource_owner: bool,
    evidence_bounds: EvidenceBounds,
) -> BindingFixture {
    binding_fixture_with_outcome_contract(
        label,
        with_resource_owner,
        evidence_bounds,
        "mfm.executor-reference-queue-result.v1",
        reviewed_ref("reference.safe-failure"),
    )
}

fn binding_fixture_with_outcome_contract(
    label: &str,
    with_resource_owner: bool,
    evidence_bounds: EvidenceBounds,
    domain_evidence_schema_contract: &str,
    safe_failure_contract_ref: ContentRef,
) -> BindingFixture {
    let tenant_scope_id = TenantScopeId::new(format!(
        "mfm.tenant_scope.v1:{:032x}",
        label.len() + usize::from(with_resource_owner) + 10
    ))
    .expect("tenant");
    let generation_ref = reviewed_ref(&format!("{label}.generation"));
    let destination_domain_ref = reviewed_ref(&format!("{label}.destination"));
    let resource_domain_ref =
        with_resource_owner.then(|| reviewed_ref(&format!("{label}.resource-domain")));
    let resource_ownership = resource_domain_ref.as_ref().map(|resource_domain_ref| {
        ResourceOwnership::new(
            reviewed_ref(&format!("{label}.coordination")),
            resource_domain_ref.clone(),
            generation_ref.clone(),
            Some(reviewed_ref(&format!("{label}.destination-fence"))),
        )
        .expect("resource ownership")
    });
    let deployment = ExecutorDeployment::new(
        reviewed_ref(&format!("{label}.namespace")),
        generation_ref.clone(),
        tenant_scope_id.clone(),
        reviewed_ref(&format!("{label}.evidence-authority")),
        resource_ownership
            .as_ref()
            .map(|value| value.reference().expect("owner ref")),
    )
    .expect("deployment");
    let executor_contract = ExecutorContractDescriptor::new(
        reviewed_ref(&format!("{label}.ensure-contract")),
        retained_contract(
            &format!("{label}.semantic-request"),
            "mfm.executor-reference-queue-request.v1",
        ),
        retained_contract(&format!("{label}.safe-failure"), "mfm.safe-failure.v1"),
        retained_closure_contract_with_domain(label, domain_evidence_schema_contract),
        safe_failure_contract_ref,
        destination_domain_ref.clone(),
        evidence_bounds,
        resource_domain_ref,
        Vec::new(),
    )
    .expect("executor contract");
    let binding = ExecutorBinding::new(
        executor_contract
            .reference()
            .expect("executor contract ref"),
        reviewed_ref(&format!("{label}.implementation")),
        deployment.reference().expect("deployment ref"),
    )
    .expect("binding");
    let binding = VerifiedExecutorBinding::verify(
        binding,
        executor_contract,
        deployment,
        resource_ownership,
        &tenant_scope_id,
    )
    .expect("verified binding");
    BindingFixture {
        binding,
        tenant_scope_id,
        generation_ref,
        destination_domain_ref,
    }
}

fn domain_outcome(schema_contract: &str, value: CanonicalValue) -> DeliveryAttemptOutcome {
    let value = contract()
        .encode(schema_contract, &value)
        .expect("domain outcome value");
    let value =
        SchemaQualifiedCanonicalValue::from_validated(&value).expect("schema-qualified outcome");
    DeliveryAttemptOutcome::returned(value).expect("returned outcome")
}

#[derive(Debug)]
struct CapturedTargetAuthority {
    identity: EffectIdentity,
    attempt_id: AttemptId,
    target_operation_ref: ContentRef,
    durable_ledger_generation_ref: ContentRef,
}

impl CapturedTargetAuthority {
    fn from_authority(authority: &TargetEntryAuthority) -> Self {
        Self {
            identity: authority.identity().clone(),
            attempt_id: authority.attempt_id().clone(),
            target_operation_ref: authority.target_operation_ref().clone(),
            durable_ledger_generation_ref: authority.durable_ledger_generation_ref().clone(),
        }
    }
}

fn bounds(max_attempts: u32) -> EvidenceBounds {
    EvidenceBounds::new(max_attempts, 256, 4_000_000, 16_384, 2, 16_384).expect("bounds")
}

fn reference_contract(fixture: &BindingFixture) -> ReferenceContract {
    ReferenceContract::new(
        fixture.destination_domain_ref.clone(),
        reviewed_value("reference.enqueue"),
        "applied",
        reviewed_ref("reference.assurance"),
        reviewed_ref("reference.safe-failure"),
    )
    .expect("reference contract")
}

fn adversarial_delivery_outcomes(
    fixture: &BindingFixture,
) -> [(&'static str, DeliveryAttemptOutcome); 4] {
    let wrong_outcome_tuple = DeliveryAttemptOutcome::did_not_enter(
        reference_safe_failure(
            fixture
                .binding
                .contract()
                .safe_failure_contract_ref()
                .clone(),
            ReferenceFailureCode::DestinationUnavailable,
            FailureClass::Transport,
            BoundaryStage::BoundaryEntry,
        )
        .expect("structurally valid wrong outcome tuple"),
    )
    .expect("wrong tuple candidate");
    let illegal_fact_layer = DeliveryAttemptOutcome::non_domain_failure(
        NonDomainFailure::new(
            NonDomainEntryStatus::MayHaveEntered,
            NonDomainDisposition::RetryableOperational,
            NonDomainFailureCode::FactStoreUnavailable,
        )
        .expect("globally valid fact-layer failure"),
    )
    .expect("fact-layer candidate");
    let wrong_safe_failure_contract = DeliveryAttemptOutcome::did_not_enter(
        reference_safe_failure(
            reviewed_ref("hostile.safe-failure-contract"),
            ReferenceFailureCode::GenerationFenced,
            FailureClass::Authorization,
            BoundaryStage::BeforeBoundaryEntry,
        )
        .expect("wrong-contract safe failure"),
    )
    .expect("wrong-contract candidate");
    let wrong_returned_schema =
        DeliveryAttemptOutcome::returned(reviewed_value("hostile.returned-schema"))
            .expect("wrong-schema returned candidate");
    [
        ("wrong-outcome-tuple", wrong_outcome_tuple),
        ("illegal-fact-layer", illegal_fact_layer),
        ("wrong-safe-failure-contract", wrong_safe_failure_contract),
        ("wrong-returned-schema", wrong_returned_schema),
    ]
}

fn assert_adapter_contract_violation(outcome: &DeliveryAttemptOutcome) {
    assert!(outcome.returned_outcome().is_none());
    assert!(outcome.did_not_enter_failure().is_none());
    assert!(outcome.indeterminate_failure().is_none());
    let fields = outcome
        .non_domain_failure_value()
        .expect("adapter failure outcome")
        .fields();
    assert_eq!(fields.entry_status, NonDomainEntryStatus::MayHaveEntered);
    assert_eq!(fields.disposition, NonDomainDisposition::IntegrityBlocked);
    assert_eq!(fields.code, NonDomainFailureCode::AdapterContractViolation);
}

fn identity_inputs(seed: u8) -> (StoreScopeId, RunId, NodeId) {
    let scope = StoreScopeId::new(format!(
        "mfm.store_scope.v1:{:032x}",
        u128::from(seed) + 100
    ))
    .expect("scope");
    let run = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[seed, 1]),
    );
    let node = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[seed, 2]),
    );
    (scope, run, node)
}

fn committed(
    fixture: &BindingFixture,
    seed: u8,
    operation: &str,
    payload_role: &str,
) -> CommittedEffectRequest<ReferenceRequest> {
    let (scope, run, node) = identity_inputs(seed);
    CommittedEffectRequest::new(
        fixture.binding.binding_ref().clone(),
        fixture.tenant_scope_id.clone(),
        &scope,
        &run,
        &node,
        ReferenceRequest::new(operation, payload_value_ref(payload_role))
            .expect("reference request"),
    )
    .expect("committed request")
}

fn reference_fixture(
    max_attempts: u32,
) -> (
    BindingFixture,
    MemoryConvergentDestination,
    ReferenceExecutor<MemoryExecutorStore, MemoryConvergentDestination>,
) {
    let fixture = binding_fixture("reference", false, max_attempts);
    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate generation");
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let executor =
        ReferenceExecutor::new(ledger, destination.clone(), reference_contract(&fixture));
    (fixture, destination, executor)
}

#[derive(Clone)]
struct HeldFollowerDestination {
    inner: MemoryConvergentDestination,
    authorized: Arc<Barrier>,
    followers_released: Arc<(Mutex<bool>, Condvar)>,
}

impl HeldFollowerDestination {
    fn new(inner: MemoryConvergentDestination, worker_count: usize) -> Self {
        Self {
            inner,
            authorized: Arc::new(Barrier::new(worker_count)),
            followers_released: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    fn release_followers(&self) {
        let (released, wake) = &*self.followers_released;
        *released.lock().expect("follower release lock") = true;
        wake.notify_all();
    }
}

impl ReferenceDestination for HeldFollowerDestination {
    fn enqueue<'a>(
        &'a self,
        authority: TargetEntryAuthority,
        request: &'a ReferenceRequest,
        contract: &'a ReferenceContract,
        behavior: ReferenceTargetBehavior,
    ) -> ExecutorFuture<'a, ReferenceDestinationReturn> {
        let first_attempt = mfm_executor::derive_attempt_id(
            authority.identity(),
            0,
            authority.target_operation_ref(),
        )
        .expect("first attempt identity");
        let is_first_attempt = authority.attempt_id() == &first_attempt;
        self.authorized.wait();
        if !is_first_attempt {
            let (released, wake) = &*self.followers_released;
            let mut released = released.lock().expect("follower release lock");
            while !*released {
                released = wake.wait(released).expect("follower release wait");
            }
        }
        self.inner.enqueue(authority, request, contract, behavior)
    }
}

fn block_on<Future>(future: Future) -> Future::Output
where
    Future: std::future::Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(future)
}

fn restore_memory_store(
    binding: &VerifiedExecutorBinding,
    checkpoint: &MemoryLedgerCheckpoint,
) -> MemoryExecutorStore {
    MemoryExecutorStore::restore(binding, checkpoint.clone()).expect("restore memory checkpoint")
}

fn terminalize_allocated_effect(
    ledger: &KeyedExecutorLedger<MemoryExecutorStore>,
    fixture: &BindingFixture,
    request: &CommittedEffectRequest<ReferenceRequest>,
    policy_binding: &ResourcePolicyBinding,
) {
    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate generation");
    let contract = reference_contract(fixture);
    let current = block_on(ledger.effect_view(request.identity()))
        .expect("effect view")
        .expect("bound effect");
    let expected_head = current.delivery_audit().head_ref().expect("head");
    let outcome = block_on(ledger.execute_target_once(
        request.identity(),
        &expected_head,
        contract.enqueue_operation().clone(),
        Some(policy_binding),
        |authority| async {
            destination
                .enqueue(
                    authority,
                    request.request(),
                    &contract,
                    ReferenceTargetBehavior::Available,
                )
                .await
                .into_outcome()
        },
    ))
    .expect("allocated target");
    let ExecuteTargetOutcome::Observed(view) = outcome else {
        panic!("allocated target must be uncontended")
    };
    let attempt = view
        .delivery_audit()
        .attempts()
        .expect("attempts")
        .into_iter()
        .last()
        .expect("target attempt");
    let attempt_id = attempt.attempt_id().clone();
    let returned_outcome = attempt
        .outcome()
        .expect("observed outcome")
        .returned_outcome()
        .cloned()
        .expect("returned outcome");
    let attempts = view.delivery_audit().attempts().expect("folded attempts");
    let observation_ref = attempts
        .iter()
        .find(|attempt| attempt.attempt_id() == &attempt_id)
        .and_then(mfm_executor::DeliveryAttemptView::returned_observation_ref)
        .cloned()
        .expect("returned observation ref");
    let proof = ReferenceTerminalProof::new(attempt_id, returned_outcome, observation_ref)
        .expect("terminal proof");
    let tombstone = TerminalTombstone::new(
        request.request().external_operation_identity(),
        contract.terminal_outcome(),
        proof,
    )
    .expect("terminal tombstone");
    block_on(ledger.append_terminal_tombstone(request.identity(), tombstone))
        .expect("terminal tombstone append");
}

#[test]
fn exact_binding_tenant_and_request_are_immutable() {
    let (fixture, _, executor) = reference_fixture(8);
    let original = committed(&fixture, 1, "operation.1", "payload.1");
    let original_key = original.identity().effect_key().clone();

    let terminal = block_on(executor.drive(&original)).expect("terminal");
    let Ensure::Terminal { evidence } = terminal.outcome() else {
        panic!("expected terminal claim");
    };
    assert_eq!(evidence.claim().identity(), original.identity());
    assert_eq!(
        EffectIdentity::reconstruct(
            &fixture.binding,
            original.identity().tenant_scope_id().clone(),
            original.identity().executor_binding_ref().clone(),
            original.identity().effect_key().clone(),
            original.identity().request_digest().clone(),
        )
        .expect("retained identity"),
        original.identity().clone()
    );

    let changed = committed(&fixture, 1, "operation.1", "payload.changed");
    assert_eq!(changed.identity().effect_key(), &original_key);
    assert_eq!(
        block_on(executor.drive(&changed)).expect_err("same key with changed request"),
        ExecutorError::EffectBindingConflict
    );

    let other = binding_fixture("other", false, 8);
    let other_request = committed(&other, 1, "operation.1", "payload.1");
    assert_eq!(
        block_on(executor.drive(&other_request)).expect_err("different binding"),
        ExecutorError::WrongExecutorBinding
    );

    let (scope, run, node) = identity_inputs(2);
    let wrong_tenant =
        TenantScopeId::new("mfm.tenant_scope.v1:ffffffffffffffffffffffffffffffff").expect("tenant");
    let wrong_tenant_request = CommittedEffectRequest::new(
        fixture.binding.binding_ref().clone(),
        wrong_tenant,
        &scope,
        &run,
        &node,
        ReferenceRequest::new("operation.tenant", payload_value_ref("payload")).expect("request"),
    )
    .expect("committed");
    assert_eq!(
        block_on(executor.drive(&wrong_tenant_request)).expect_err("wrong tenant"),
        ExecutorError::TenantScopeMismatch
    );
}

#[test]
fn descriptor_binding_and_domain_request_bytes_are_strictly_separate() {
    let fixture = binding_fixture("descriptor", false, 8);
    let descriptor = fixture.binding.contract().clone();
    let decoded = ExecutorContractDescriptor::strict_decode(
        descriptor.validated().expect("descriptor").as_bytes(),
    )
    .expect("strict descriptor");
    assert_eq!(decoded, descriptor);
    assert_eq!(
        RetainedValueContract::strict_decode(
            descriptor
                .semantic_request_contract()
                .validated()
                .expect("request contract")
                .as_bytes(),
        )
        .expect("strict retained value contract"),
        descriptor.semantic_request_contract().clone()
    );
    assert_eq!(
        ExecutorRetainedClosureContract::strict_decode(
            descriptor
                .retained_closure_contract()
                .validated()
                .expect("closure contract")
                .as_bytes(),
        )
        .expect("strict retained closure contract"),
        descriptor.retained_closure_contract().clone()
    );
    let closure = descriptor.retained_closure_contract();
    assert_eq!(
        ExecutorRetainedClosureContract::new(
            closure.domain_evidence_contract().clone(),
            closure.delivery_audit_contract().clone(),
            closure.executor_frontier_contract().clone(),
            closure.terminal_evidence_contract().clone(),
            closure.terminal_tombstone_contract().clone(),
            closure.terminal_proof_contract().clone(),
            closure.domain_evidence_contract().clone(),
        )
        .expect_err("outer result schema substitution"),
        ExecutorError::RetainedValueContractMismatch
    );

    let deployment = fixture.binding.deployment().clone();
    let wrong_descriptor = ExecutorContractDescriptor::new(
        reviewed_ref("descriptor.changed-ensure"),
        descriptor.semantic_request_contract().clone(),
        descriptor.safe_failure_value_contract().clone(),
        descriptor.retained_closure_contract().clone(),
        descriptor.safe_failure_contract_ref().clone(),
        descriptor.downstream_convergence_contract_ref().clone(),
        descriptor.evidence_bounds().clone(),
        descriptor.resource_domain_requirement().cloned(),
        descriptor.required_plan_expansions().to_vec(),
    )
    .expect("wrong descriptor");
    assert_eq!(
        VerifiedExecutorBinding::verify(
            fixture.binding.binding().clone(),
            wrong_descriptor,
            deployment,
            None,
            &fixture.tenant_scope_id,
        )
        .expect_err("descriptor substitution"),
        ExecutorError::ExecutorContractReferenceMismatch
    );

    let arbitrary_domain_value = SchemaQualifiedCanonicalValue::new(
        contract()
            .schema_id("mfm.primitive-stable_id.v1")
            .expect("schema")
            .clone(),
        br#"{"domain_specific":[1,true,"value"]}"#,
    )
    .expect("schema-qualified domain value");
    let (scope, run, node) = identity_inputs(91);
    CommittedEffectRequest::new(
        fixture.binding.binding_ref().clone(),
        fixture.tenant_scope_id,
        &scope,
        &run,
        &node,
        arbitrary_domain_value,
    )
    .expect("request digest does not annex-validate domain shape");
}

#[test]
fn retained_verifier_rejects_missing_objects_and_terminal_substitution() {
    let (fixture, _, executor) = reference_fixture(8);
    let request = committed(&fixture, 92, "operation.retained", "payload");
    let terminal = block_on(executor.drive(&request)).expect("terminal");
    let Ensure::Terminal { evidence } = terminal.outcome() else {
        panic!("expected terminal");
    };
    let claim = evidence.claim().clone();
    let identity = terminal.identity().clone();
    let head_ref = claim.delivery_audit_ref().as_content_ref().clone();
    assert_eq!(
        verify_ensure_result(
            identity.clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::pending(claim.delivery_audit_ref().clone()),
            ExecutorRetainedClosureClaim::new(terminal.retained_closure().members().cloned(),)
                .expect("closure claim"),
        )
        .expect_err("terminal audit cannot regress to pending"),
        ExecutorError::TerminalTombstoneConflict
    );
    let without_head = ExecutorRetainedClosureClaim::new(
        terminal
            .retained_closure()
            .members()
            .filter(|member| member.reference().expect("reference") != head_ref)
            .cloned(),
    )
    .expect("closure");
    assert_eq!(
        verify_ensure_result(
            identity.clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::terminal(claim.clone()),
            without_head,
        )
        .expect_err("missing head"),
        ExecutorError::RetainedClosureIncomplete
    );

    let substituted = ExecutorTerminalEvidenceClaim::new(
        identity.clone(),
        claim.delivery_audit_ref().clone(),
        claim.terminal_tombstone_ref().clone(),
        claim.external_operation_identity(),
        claim.terminal_outcome(),
        claim.assurance_policy_ref().clone(),
        claim.proof_basis().clone(),
        head_ref,
    )
    .expect("substituted claim");
    assert_eq!(
        verify_ensure_result(
            identity,
            &fixture.binding,
            ExecutorEnsureResultClaim::terminal(substituted),
            ExecutorRetainedClosureClaim::new(terminal.retained_closure().members().cloned(),)
                .expect("closure claim"),
        )
        .expect_err("domain evidence substitution"),
        ExecutorError::TerminalProofMismatch
    );
}

#[test]
fn verified_results_and_executor_outcomes_preserve_public_values() {
    let (fixture, _, executor) = reference_fixture(8);
    let request = committed(&fixture, 94, "operation.outcome", "payload");
    let verified = block_on(executor.drive(&request)).expect("verified result");
    let expected_identity = verified.identity().clone();
    let expected_outcome = verified.outcome().clone();
    let expected_audit = verified.delivery_audit().clone();
    let expected_closure = verified.retained_closure().clone();
    let Ensure::Terminal { evidence } = verified.outcome() else {
        panic!("expected terminal result");
    };
    let returned_outcome = evidence.terminal_proof().returned_outcome();
    let expected_safe_result = returned_outcome.safe_result().clone();
    let expected_safe_result_ref = expected_safe_result.reference().expect("safe result ref");
    assert_eq!(returned_outcome.safe_result(), &expected_safe_result);
    assert_eq!(
        returned_outcome.safe_result_ref(),
        &expected_safe_result_ref
    );
    let cloned_returned_outcome = returned_outcome.clone();
    assert_eq!(&cloned_returned_outcome, returned_outcome);
    assert_eq!(cloned_returned_outcome.safe_result(), &expected_safe_result);
    assert_eq!(
        cloned_returned_outcome.safe_result_ref(),
        &expected_safe_result_ref
    );

    let returned = EffectExecutorOutcome::returned(verified.clone());
    assert_eq!(
        returned.view(),
        EffectExecutorOutcomeView::Returned(&verified)
    );
    assert_eq!(
        returned.into_parts(),
        EffectExecutorOutcomeParts::Returned(verified.clone())
    );
    assert_eq!(
        verified.into_parts(),
        (
            expected_identity,
            expected_outcome,
            expected_audit,
            expected_closure,
        )
    );

    let safe_failure_contract_ref = fixture
        .binding
        .contract()
        .safe_failure_contract_ref()
        .clone();
    let did_not_enter_failure = reference_safe_failure(
        safe_failure_contract_ref.clone(),
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BeforeBoundaryEntry,
    )
    .expect("did-not-enter failure");
    let did_not_enter =
        EffectExecutorOutcome::did_not_enter(did_not_enter_failure.clone()).expect("outcome");
    assert_eq!(
        did_not_enter.view(),
        EffectExecutorOutcomeView::DidNotEnter(&did_not_enter_failure)
    );
    assert_eq!(
        did_not_enter.into_parts(),
        EffectExecutorOutcomeParts::DidNotEnter(did_not_enter_failure)
    );

    let indeterminate_failure = reference_safe_failure(
        safe_failure_contract_ref,
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BoundaryEntry,
    )
    .expect("indeterminate failure");
    let indeterminate =
        EffectExecutorOutcome::indeterminate(indeterminate_failure.clone()).expect("outcome");
    assert_eq!(
        indeterminate.view(),
        EffectExecutorOutcomeView::Indeterminate(&indeterminate_failure)
    );
    assert_eq!(
        indeterminate.into_parts(),
        EffectExecutorOutcomeParts::Indeterminate(indeterminate_failure)
    );
}

#[test]
fn retained_closure_is_binding_qualified_complete_and_exact() {
    let (fixture, _, executor) = reference_fixture(8);
    let request = committed(&fixture, 93, "operation.closure", "payload");
    let terminal = block_on(executor.drive(&request)).expect("terminal");
    let Ensure::Terminal { evidence } = terminal.outcome() else {
        panic!("expected terminal");
    };
    assert_eq!(
        terminal.retained_closure().executor_binding_ref(),
        fixture.binding.binding_ref()
    );
    assert_eq!(
        terminal.retained_closure().contract(),
        fixture.binding.contract().retained_closure_contract()
    );
    let members = terminal
        .retained_closure()
        .members()
        .cloned()
        .collect::<Vec<_>>();
    let relations = members
        .iter()
        .map(ExecutorRetainedValue::relation)
        .collect::<Vec<_>>();
    assert!(relations.contains(&ExecutorRetainedValueRelation::DeliveryAudit));
    assert!(relations.contains(&ExecutorRetainedValueRelation::TerminalTombstone));
    assert!(relations.contains(&ExecutorRetainedValueRelation::TerminalProof));
    assert!(relations.contains(&ExecutorRetainedValueRelation::DomainEvidence));

    for missing_relation in [
        ExecutorRetainedValueRelation::DeliveryAudit,
        ExecutorRetainedValueRelation::ExecutorFrontier,
        ExecutorRetainedValueRelation::TerminalTombstone,
        ExecutorRetainedValueRelation::TerminalProof,
        ExecutorRetainedValueRelation::DomainEvidence,
    ] {
        let mut removed = false;
        let incomplete = members
            .iter()
            .filter(|member| {
                if !removed && member.relation() == missing_relation {
                    removed = true;
                    false
                } else {
                    true
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        assert!(removed, "fixture lacks {missing_relation:?}");
        assert_eq!(
            verify_ensure_result(
                terminal.identity().clone(),
                &fixture.binding,
                ExecutorEnsureResultClaim::terminal(evidence.claim().clone()),
                ExecutorRetainedClosureClaim::new(incomplete).expect("incomplete claim"),
            )
            .expect_err("missing reachable member"),
            ExecutorError::RetainedClosureIncomplete,
            "{missing_relation:?}"
        );
    }

    assert_eq!(
        ExecutorRetainedClosureClaim::new(
            members
                .iter()
                .cloned()
                .chain(std::iter::once(members[0].clone())),
        )
        .expect_err("duplicate member"),
        ExecutorError::RetainedClosureDuplicate
    );

    let domain_index = members
        .iter()
        .position(|member| member.relation() == ExecutorRetainedValueRelation::DomainEvidence)
        .expect("domain evidence");
    let domain = &members[domain_index];
    let selected = domain.contract();
    let wrong_contract = RetainedValueContract::new(
        selected.schema_id().clone(),
        selected.semantic_type_id().clone(),
        StableId::new("wrong.domain-evidence").expect("role"),
        selected.media_type(),
        selected.evidence_contract_ref().clone(),
    )
    .expect("wrong contract");
    let mut wrong_contract_members = members.clone();
    wrong_contract_members[domain_index] = ExecutorRetainedValue::new(
        ExecutorRetainedValueRelation::DomainEvidence,
        wrong_contract,
        domain.value().clone(),
    )
    .expect("member");
    assert_eq!(
        verify_ensure_result(
            terminal.identity().clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::terminal(evidence.claim().clone()),
            ExecutorRetainedClosureClaim::new(wrong_contract_members).expect("claim"),
        )
        .expect_err("metadata substitution"),
        ExecutorError::RetainedValueContractMismatch
    );

    let extra_value =
        SchemaQualifiedCanonicalValue::new(selected.schema_id().clone(), br#"{"extra":true}"#)
            .expect("extra value");
    let extra_member = ExecutorRetainedValue::new(
        ExecutorRetainedValueRelation::DomainEvidence,
        selected.clone(),
        extra_value,
    )
    .expect("extra member");
    assert_eq!(
        verify_ensure_result(
            terminal.identity().clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::terminal(evidence.claim().clone()),
            ExecutorRetainedClosureClaim::new(
                members.iter().cloned().chain(std::iter::once(extra_member)),
            )
            .expect("extra claim"),
        )
        .expect_err("unreachable extra"),
        ExecutorError::RetainedClosureExtra
    );

    let head = members
        .iter()
        .find(|member| member.relation() == ExecutorRetainedValueRelation::DeliveryAudit)
        .expect("head");
    let fork_member = ExecutorRetainedValue::new(
        ExecutorRetainedValueRelation::ExecutorFrontier,
        fixture
            .binding
            .contract()
            .retained_closure_contract()
            .executor_frontier_contract()
            .clone(),
        head.value().clone(),
    )
    .expect("fork member");
    assert_eq!(
        verify_ensure_result(
            terminal.identity().clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::terminal(evidence.claim().clone()),
            ExecutorRetainedClosureClaim::new(
                members.iter().cloned().chain(std::iter::once(fork_member)),
            )
            .expect("fork claim"),
        )
        .expect_err("forked frontier set"),
        ExecutorError::FrontierFork
    );

    let other = binding_fixture("closure-other", false, 8);
    assert_eq!(
        verify_ensure_result(
            terminal.identity().clone(),
            &other.binding,
            ExecutorEnsureResultClaim::terminal(evidence.claim().clone()),
            ExecutorRetainedClosureClaim::new(members).expect("claim"),
        )
        .expect_err("other binding"),
        ExecutorError::WrongExecutorBinding
    );
}

#[test]
fn concurrent_delayed_and_post_terminal_drives_converge() {
    const WORKER_COUNT: usize = 4;

    let fixture = binding_fixture("reference", false, 64);
    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate generation");
    let controlled_destination = HeldFollowerDestination::new(destination.clone(), WORKER_COUNT);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let executor = ReferenceExecutor::new(
        ledger,
        controlled_destination.clone(),
        reference_contract(&fixture),
    );
    let request = committed(&fixture, 3, "operation.concurrent", "payload");
    let barrier = Arc::new(Barrier::new(WORKER_COUNT));
    let (result_sender, result_receiver) = std::sync::mpsc::channel();
    let mut workers = Vec::new();
    for _ in 0..WORKER_COUNT {
        let barrier = Arc::clone(&barrier);
        let executor = executor.clone();
        let request = request.clone();
        let result_sender = result_sender.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            result_sender
                .send(block_on(executor.drive(&request)))
                .expect("send drive result");
        }));
    }
    drop(result_sender);
    let first = match result_receiver.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(result) => result,
        Err(error) => {
            controlled_destination.release_followers();
            panic!("first observed drive did not complete: {error}");
        }
    };
    controlled_destination.release_followers();
    let mut results = vec![first];
    for _ in 1..WORKER_COUNT {
        results.push(
            result_receiver
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("receive follower drive result"),
        );
    }
    for worker in workers {
        worker.join().expect("worker");
    }
    for result in results {
        let result = result.expect("drive");
        assert!(matches!(result.outcome(), Ensure::Terminal { .. }));
    }
    assert_eq!(destination.semantic_mutation_count().expect("mutations"), 1);

    let entries_before = destination.target_entry_count().expect("entries");
    let delayed = block_on(executor.drive(&request)).expect("delayed");
    assert!(matches!(delayed.outcome(), Ensure::Terminal { .. }));
    assert_eq!(
        destination.target_entry_count().expect("entries"),
        entries_before
    );
}

#[test]
fn every_crash_boundary_recovers_without_duplicate_mutation() {
    for (seed, point, expected_mutations) in [
        (4, ReferenceCrashPoint::BeforeTargetEntry, 0),
        (6, ReferenceCrashPoint::AfterObservationBeforeTombstone, 1),
        (7, ReferenceCrashPoint::AfterTombstoneBeforeReturn, 1),
    ] {
        let (fixture, destination, executor) = reference_fixture(12);
        let request = committed(
            &fixture,
            seed,
            &format!("operation.crash.{seed}"),
            "payload",
        );
        assert_eq!(
            block_on(executor.drive_conformance(
                &request,
                ReferenceTargetBehavior::Available,
                Some(point),
            ))
            .expect("injected drive"),
            ReferenceDriveOutcome::Crashed(point)
        );
        assert_eq!(
            destination.semantic_mutation_count().expect("mutations"),
            expected_mutations
        );
        let terminal = block_on(executor.drive(&request)).expect("recovery drive");
        assert!(matches!(terminal.outcome(), Ensure::Terminal { .. }));
        assert_eq!(destination.semantic_mutation_count().expect("mutations"), 1);
    }
}

#[test]
fn safe_failures_remain_pending_and_attempt_bounds_fail_closed() {
    let (fixture, destination, executor) = reference_fixture(1);
    let request = committed(&fixture, 8, "operation.failure", "payload");
    let pending = block_on(executor.drive_conformance(
        &request,
        ReferenceTargetBehavior::CancelledBeforeEntry,
        None,
    ))
    .expect("cancelled");
    let ReferenceDriveOutcome::Returned(pending) = pending else {
        panic!("expected returned pending result");
    };
    assert!(matches!(pending.outcome(), Ensure::Pending { .. }));
    assert_eq!(pending.delivery_audit().attempt_count(), 1);
    assert_eq!(destination.target_entry_count().expect("entries"), 0);

    let bounded = block_on(executor.drive(&request)).expect("bounded pending");
    assert!(matches!(bounded.outcome(), Ensure::Pending { .. }));
    assert_eq!(bounded.delivery_audit().attempt_count(), 1);
    assert_eq!(destination.target_entry_count().expect("entries"), 0);
}

#[test]
fn bind_effect_resolves_every_64th_append_outcome_without_a_65th_cas_or_target_entry() {
    for (label, final_outcome, evidence) in FINAL_RESOLUTION_CASES {
        let fixture = binding_fixture(&format!("final-bind-{label}"), false, 8);
        let request = committed(&fixture, 99, "operation.final-bind", "payload");
        let before = MemoryExecutorStore::new(&fixture.binding);
        let after = MemoryExecutorStore::new(&fixture.binding);
        let after_ledger = KeyedExecutorLedger::new(after.clone(), fixture.binding.clone())
            .expect("after-state ledger");
        let effect_override = match evidence {
            FinalResolutionEvidence::Identical => {
                block_on(after_ledger.bind_effect(request.identity())).expect("identical binding");
                None
            }
            FinalResolutionEvidence::Conflicting => {
                let other = committed(&fixture, 100, "operation.final-bind-conflict", "payload");
                block_on(after_ledger.bind_effect(other.identity())).expect("conflicting binding");
                Some(
                    block_on(after.load_effect(other.identity().effect_key()))
                        .expect("load conflicting binding")
                        .expect("conflicting snapshot"),
                )
            }
        };
        let mut store = ScriptedFinalResolutionStore::new(before, after, final_outcome);
        if let Some(snapshot) = effect_override {
            store = store.with_after_effect_override(snapshot);
        }
        let ledger = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
            .expect("scripted ledger");
        let destination = MemoryConvergentDestination::new();

        match evidence {
            FinalResolutionEvidence::Identical => {
                let view = block_on(ledger.bind_effect(request.identity()))
                    .expect("resolved exact binding");
                assert_eq!(view.identity(), request.identity());
                assert_eq!(view.delivery_audit().attempt_count(), 0);
            }
            FinalResolutionEvidence::Conflicting => {
                assert_eq!(
                    block_on(ledger.bind_effect(request.identity()))
                        .expect_err("resolved binding conflict"),
                    ExecutorError::EffectBindingConflict
                );
            }
        }
        assert_eq!(store.append_attempts(), 64, "{label}");
        assert_eq!(
            destination.target_entry_count().expect("target entries"),
            0,
            "{label}"
        );
    }
}

#[test]
fn bind_and_allocate_resolves_every_64th_append_outcome_without_a_65th_cas_or_target_entry() {
    for (label, final_outcome, evidence) in FINAL_RESOLUTION_CASES {
        let fixture = binding_fixture(&format!("final-allocation-{label}"), true, 8);
        let request = committed(&fixture, 101, "operation.final-allocation", "payload");
        let policy_request = AccountSequenceRequest::new("final-allocation-sender", "chain-1")
            .expect("policy request");
        let policy = AccountSequencePolicy::new(
            ResourcePolicyBinding::new(
                reviewed_ref(&format!("final-allocation-{label}.policy")),
                reviewed_ref(&format!("final-allocation-{label}.configuration")),
            ),
            51,
            None,
        );
        let before = MemoryExecutorStore::new(&fixture.binding);
        let after = MemoryExecutorStore::new(&fixture.binding);
        let after_ledger = KeyedExecutorLedger::new(after.clone(), fixture.binding.clone())
            .expect("after-state ledger");
        match evidence {
            FinalResolutionEvidence::Identical => {
                block_on(after_ledger.try_bind_and_allocate(
                    request.identity(),
                    None,
                    &policy,
                    &policy_request,
                ))
                .expect("identical allocation");
            }
            FinalResolutionEvidence::Conflicting => {
                let conflicting_policy = AccountSequencePolicy::new(
                    ResourcePolicyBinding::new(
                        reviewed_ref(&format!("final-allocation-{label}.other-policy")),
                        reviewed_ref(&format!("final-allocation-{label}.other-configuration")),
                    ),
                    61,
                    None,
                );
                block_on(after_ledger.try_bind_and_allocate(
                    request.identity(),
                    None,
                    &conflicting_policy,
                    &policy_request,
                ))
                .expect("conflicting allocation");
            }
        }
        let store = ScriptedFinalResolutionStore::new(before, after, final_outcome);
        let ledger = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
            .expect("scripted ledger");
        let destination = MemoryConvergentDestination::new();

        let result = block_on(ledger.try_bind_and_allocate(
            request.identity(),
            None,
            &policy,
            &policy_request,
        ));
        match evidence {
            FinalResolutionEvidence::Conflicting => {
                assert_eq!(
                    result.expect_err("resolved allocation conflict"),
                    ExecutorError::ResourceAllocationConflict
                );
            }
            FinalResolutionEvidence::Identical => match result.expect("resolved allocation") {
                AllocationOutcome::Allocated { allocation, .. } => {
                    assert_eq!(final_outcome, ExecutorAppendOutcome::Applied);
                    assert_eq!(allocation.sequence(), 51);
                }
                AllocationOutcome::Existing { allocation, .. } => {
                    assert_ne!(final_outcome, ExecutorAppendOutcome::Applied);
                    assert_eq!(allocation.sequence(), 51);
                }
            },
        }
        assert_eq!(store.append_attempts(), 64, "{label}");
        assert_eq!(
            destination.target_entry_count().expect("target entries"),
            0,
            "{label}"
        );
    }
}

#[test]
fn terminal_tombstone_resolves_every_64th_append_outcome_without_reinvocation_or_a_65th_cas() {
    for (label, final_outcome, evidence) in FINAL_RESOLUTION_CASES {
        let fixture = binding_fixture(&format!("final-tombstone-{label}"), false, 8);
        let raw = MemoryExecutorStore::new(&fixture.binding);
        let ledger = KeyedExecutorLedger::new(raw.clone(), fixture.binding.clone())
            .expect("preparation ledger");
        let destination = MemoryConvergentDestination::new();
        destination
            .activate_generation(fixture.generation_ref.clone())
            .expect("activate generation");
        let contract = reference_contract(&fixture);
        let request = committed(&fixture, 102, "operation.final-tombstone", "payload");
        let bound = block_on(ledger.bind_effect(request.identity())).expect("bind effect");
        let observed = block_on(ledger.execute_target_once(
            request.identity(),
            &bound.delivery_audit().head_ref().expect("bound head"),
            contract.enqueue_operation().clone(),
            None,
            |authority| async {
                destination
                    .enqueue(
                        authority,
                        request.request(),
                        &contract,
                        ReferenceTargetBehavior::Available,
                    )
                    .await
                    .into_outcome()
            },
        ))
        .expect("observed target");
        let ExecuteTargetOutcome::Observed(view) = observed else {
            panic!("fresh target authorization must be observed")
        };
        let attempt = view
            .delivery_audit()
            .attempts()
            .expect("attempts")
            .into_iter()
            .last()
            .expect("observed attempt");
        let proof = ReferenceTerminalProof::new(
            attempt.attempt_id().clone(),
            attempt
                .outcome()
                .and_then(DeliveryAttemptOutcome::returned_outcome)
                .cloned()
                .expect("returned outcome"),
            attempt
                .returned_observation_ref()
                .cloned()
                .expect("returned observation"),
        )
        .expect("terminal proof");
        let tombstone =
            TerminalTombstone::new("operation.final-tombstone", "applied", proof.clone())
                .expect("terminal tombstone");
        let base = raw.checkpoint().expect("observed checkpoint");
        let before = restore_memory_store(&fixture.binding, &base);
        let exact_after = restore_memory_store(&fixture.binding, &base);
        let exact_ledger = KeyedExecutorLedger::new(exact_after.clone(), fixture.binding.clone())
            .expect("exact after-state ledger");
        block_on(exact_ledger.append_terminal_tombstone(request.identity(), tombstone.clone()))
            .expect("exact durable tombstone");
        let after = match evidence {
            FinalResolutionEvidence::Identical => exact_after,
            FinalResolutionEvidence::Conflicting => {
                let conflicting_after = restore_memory_store(&fixture.binding, &base);
                let conflicting_ledger =
                    KeyedExecutorLedger::new(conflicting_after.clone(), fixture.binding.clone())
                        .expect("conflicting after-state ledger");
                let conflicting =
                    TerminalTombstone::new("operation.final-tombstone-conflict", "applied", proof)
                        .expect("conflicting tombstone");
                block_on(
                    conflicting_ledger.append_terminal_tombstone(request.identity(), conflicting),
                )
                .expect("conflicting durable tombstone");
                conflicting_after
            }
        };
        let store = ScriptedFinalResolutionStore::new(before, after, final_outcome);
        let scripted = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
            .expect("scripted ledger");

        match evidence {
            FinalResolutionEvidence::Identical => {
                let resolved = block_on(
                    scripted.append_terminal_tombstone(request.identity(), tombstone.clone()),
                )
                .expect("resolved exact tombstone");
                assert_eq!(
                    resolved.terminal_tombstone().map(|(_, value)| value),
                    Some(&tombstone)
                );
            }
            FinalResolutionEvidence::Conflicting => {
                assert_eq!(
                    block_on(
                        scripted.append_terminal_tombstone(request.identity(), tombstone.clone()),
                    )
                    .expect_err("resolved tombstone conflict"),
                    ExecutorError::TerminalTombstoneConflict
                );
            }
        }
        assert_eq!(store.append_attempts(), 64, "{label}");
        assert_eq!(
            destination.target_entry_count().expect("target entries"),
            1,
            "{label}"
        );
    }
}

#[test]
fn bind_effect_local_contention_stops_after_exactly_64_attempts_without_target_entry() {
    let fixture = binding_fixture("bounded-bind-contention", false, 8);
    let store = AlwaysConflictingStore::new(MemoryExecutorStore::new(&fixture.binding));
    let ledger = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
        .expect("keyed executor ledger");
    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate generation");
    let executor =
        ReferenceExecutor::new(ledger, destination.clone(), reference_contract(&fixture));
    let request = committed(&fixture, 96, "operation.bounded-bind", "payload");

    assert_eq!(
        block_on(executor.drive(&request)).expect_err("bounded local contention"),
        ExecutorError::LocalContention
    );
    assert_eq!(store.append_attempts(), 64);
    assert_eq!(destination.target_entry_count().expect("target entries"), 0);
}

#[test]
fn bind_and_allocate_local_contention_stops_after_exactly_64_attempts_without_target_entry() {
    let fixture = binding_fixture("bounded-allocation-contention", true, 8);
    let store = AlwaysConflictingStore::new(MemoryExecutorStore::new(&fixture.binding));
    let ledger = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
        .expect("keyed executor ledger");
    let policy = AccountSequencePolicy::new(
        ResourcePolicyBinding::new(
            reviewed_ref("bounded-allocation.policy"),
            reviewed_ref("bounded-allocation.configuration"),
        ),
        51,
        None,
    );
    let policy_request = AccountSequenceRequest::new("bounded-allocation-sender", "chain-1")
        .expect("policy request");
    let request = committed(&fixture, 97, "operation.bounded-allocation", "payload");
    let destination = MemoryConvergentDestination::new();

    assert_eq!(
        block_on(ledger.try_bind_and_allocate(request.identity(), None, &policy, &policy_request,))
            .expect_err("bounded local contention"),
        ExecutorError::LocalContention
    );
    assert_eq!(store.append_attempts(), 64);
    assert_eq!(destination.target_entry_count().expect("target entries"), 0);
}

#[test]
fn terminal_tombstone_local_contention_stops_after_64_attempts_without_reinvocation() {
    let fixture = binding_fixture("bounded-tombstone-contention", false, 8);
    let raw = MemoryExecutorStore::new(&fixture.binding);
    let ledger = KeyedExecutorLedger::new(raw.clone(), fixture.binding.clone())
        .expect("keyed executor ledger");
    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate generation");
    let contract = reference_contract(&fixture);
    let request = committed(&fixture, 98, "operation.bounded-tombstone", "payload");
    let bound = block_on(ledger.bind_effect(request.identity())).expect("bind effect");
    let observed = block_on(ledger.execute_target_once(
        request.identity(),
        &bound.delivery_audit().head_ref().expect("bound head"),
        contract.enqueue_operation().clone(),
        None,
        |authority| async {
            destination
                .enqueue(
                    authority,
                    request.request(),
                    &contract,
                    ReferenceTargetBehavior::Available,
                )
                .await
                .into_outcome()
        },
    ))
    .expect("observed target");
    let ExecuteTargetOutcome::Observed(view) = observed else {
        panic!("fresh target authorization must be observed")
    };
    let attempt = view
        .delivery_audit()
        .attempts()
        .expect("attempts")
        .into_iter()
        .last()
        .expect("observed attempt");
    let proof = ReferenceTerminalProof::new(
        attempt.attempt_id().clone(),
        attempt
            .outcome()
            .and_then(DeliveryAttemptOutcome::returned_outcome)
            .cloned()
            .expect("returned outcome"),
        attempt
            .returned_observation_ref()
            .cloned()
            .expect("returned observation"),
    )
    .expect("terminal proof");
    let tombstone = TerminalTombstone::new("operation.bounded-tombstone", "applied", proof)
        .expect("terminal tombstone");

    let store = AlwaysConflictingStore::new(raw);
    let contended = KeyedExecutorLedger::new(store.clone(), fixture.binding)
        .expect("contended executor ledger");
    assert_eq!(
        block_on(contended.append_terminal_tombstone(request.identity(), tombstone))
            .expect_err("bounded local contention"),
        ExecutorError::LocalContention
    );
    assert_eq!(store.append_attempts(), 64);
    assert_eq!(destination.target_entry_count().expect("target entries"), 1);
}

#[test]
fn exact_head_authorization_rejects_a_stale_planner_before_target_entry() {
    let fixture = binding_fixture("exact-head-authorization", false, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let request = committed(&fixture, 91, "operation.exact-head", "payload");
    let initial = block_on(ledger.bind_effect(request.identity())).expect("bind");
    let expected_head = initial.delivery_audit().head_ref().expect("initial head");

    let target_outcome = mfm_executor::DeliveryAttemptOutcome::did_not_enter(
        reference_safe_failure(
            fixture
                .binding
                .contract()
                .safe_failure_contract_ref()
                .clone(),
            ReferenceFailureCode::DestinationUnavailable,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
        )
        .expect("safe failure"),
    )
    .expect("target outcome");
    let first = block_on(ledger.execute_target_once(
        request.identity(),
        &expected_head,
        reviewed_value("exact-head.target"),
        None,
        {
            let target_outcome = target_outcome.clone();
            move |_authority| async move { target_outcome }
        },
    ))
    .expect("first authorization");
    assert!(matches!(first, ExecuteTargetOutcome::Observed(_)));

    assert!(
        matches!(
            block_on(ledger.execute_target_once(
                request.identity(),
                &expected_head,
                reviewed_value("exact-head.target"),
                None,
                move |_authority| async move { target_outcome },
            ))
            .expect("stale authorization decision"),
            ExecuteTargetOutcome::Contended
        ),
        "a stale plan must not receive target-entry authority"
    );
    let current = block_on(ledger.effect_view(request.identity()))
        .expect("load effect")
        .expect("bound effect");
    assert_eq!(current.delivery_audit().attempt_count(), 1);
}

#[test]
fn oversized_valid_target_result_is_totalized_by_the_private_completion_seal() {
    let completion_limit = 8 * 1024;
    let fixture = binding_fixture_with_bounds(
        "completion-totalization",
        false,
        EvidenceBounds::new(1, 8, 1_000_000, completion_limit, 2, completion_limit)
            .expect("bounded completion contract"),
    );
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let request = committed(&fixture, 95, "operation.completion-totalization", "payload");
    let initial = block_on(ledger.bind_effect(request.identity())).expect("bind");
    let expected_head = initial.delivery_audit().head_ref().expect("initial head");
    let oversized_bytes =
        serde_json::to_vec(&"x".repeat(32 * 1024)).expect("canonical JSON string");
    let oversized = SchemaQualifiedCanonicalValue::new(
        fixture
            .binding
            .contract()
            .retained_closure_contract()
            .domain_evidence_contract()
            .schema_id()
            .clone(),
        &oversized_bytes,
    )
    .expect("oversized schema-qualified result");
    let oversized =
        mfm_executor::DeliveryAttemptOutcome::returned(oversized).expect("returned outcome");
    let invocations = Arc::new(AtomicUsize::new(0));
    let invoked = Arc::clone(&invocations);

    let observed = block_on(ledger.execute_target_once(
        request.identity(),
        &expected_head,
        reviewed_value("completion-totalization.target"),
        None,
        move |_authority| async move {
            invoked.fetch_add(1, Ordering::SeqCst);
            oversized
        },
    ))
    .expect("bounded observation");
    let ExecuteTargetOutcome::Observed(view) = observed else {
        panic!("fresh authorization must be observed")
    };
    let attempts = view.delivery_audit().attempts().expect("attempts");
    let outcome = attempts[0].outcome().expect("observed outcome");
    let failure = outcome
        .indeterminate_failure()
        .expect("oversized result must be totalized");
    assert_eq!(
        failure.stable_code(),
        &ReferenceFailureCode::ResultUnrepresentable
    );
    assert_eq!(invocations.load(Ordering::SeqCst), 1);

    let checkpoint = ledger.store().checkpoint().expect("checkpoint");
    let restored =
        MemoryExecutorStore::restore(&fixture.binding, checkpoint).expect("strict restart");
    let reopened = KeyedExecutorLedger::new(restored, fixture.binding.clone())
        .expect("reopened keyed executor ledger");
    let view = block_on(reopened.effect_view(request.identity()))
        .expect("reopened effect view")
        .expect("persisted effect");
    let attempts = view.delivery_audit().attempts().expect("reopened attempts");
    let failure = attempts[0]
        .outcome()
        .expect("reopened observed outcome")
        .indeterminate_failure()
        .expect("reopened oversized result must remain totalized");
    assert_eq!(
        failure.stable_code(),
        &ReferenceFailureCode::ResultUnrepresentable
    );
}

#[test]
fn affine_completion_normalizes_all_unbound_outcomes_before_memory_persistence() {
    let fixture = binding_fixture("completion-binding-seal", false, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut identities = Vec::new();

    for (index, (label, hostile)) in adversarial_delivery_outcomes(&fixture)
        .into_iter()
        .enumerate()
    {
        let seed = u8::try_from(100 + index).expect("test seed");
        let request = committed(&fixture, seed, label, "binding-seal");
        identities.push(request.identity().clone());
        let initial = block_on(ledger.bind_effect(request.identity())).expect("bind");
        let expected_head = initial.delivery_audit().head_ref().expect("head");
        let invoked = Arc::clone(&invocations);
        let observed = block_on(ledger.execute_target_once(
            request.identity(),
            &expected_head,
            reviewed_value(&format!("{label}.target")),
            None,
            move |_authority| async move {
                invoked.fetch_add(1, Ordering::SeqCst);
                hostile
            },
        ))
        .expect("sealed observation");
        let ExecuteTargetOutcome::Observed(view) = observed else {
            panic!("fresh target authority must be observed")
        };
        let attempts = view.delivery_audit().attempts().expect("attempts");
        assert_eq!(attempts.len(), 1, "{label}");
        assert_adapter_contract_violation(attempts[0].outcome().expect("one exact observation"));
    }
    assert_eq!(invocations.load(Ordering::SeqCst), 4);

    let checkpoint = ledger.store().checkpoint().expect("checkpoint");
    let restored =
        MemoryExecutorStore::restore(&fixture.binding, checkpoint).expect("strict restart");
    let reopened = KeyedExecutorLedger::new(restored, fixture.binding.clone())
        .expect("reopened keyed executor ledger");
    for identity in identities {
        let view = block_on(reopened.effect_view(&identity))
            .expect("reopened view")
            .expect("persisted effect");
        let attempts = view.delivery_audit().attempts().expect("attempts");
        assert_eq!(attempts.len(), 1);
        assert_adapter_contract_violation(
            attempts[0].outcome().expect("reopened exact observation"),
        );
    }
}

#[test]
fn concurrent_same_binding_swapped_outcomes_keep_each_authorization_identity() {
    let fixture = binding_fixture_with_outcome_contract(
        "same-binding-outcome-swap",
        false,
        bounds(8),
        "mfm.primitive-stable_id.v1",
        reviewed_ref("same-binding-outcome-swap.safe-failure"),
    );
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let first = committed(&fixture, 180, "operation.same-binding.first", "payload");
    let second = committed(&fixture, 181, "operation.same-binding.second", "payload");
    let first_head = block_on(ledger.bind_effect(first.identity()))
        .expect("bind first")
        .delivery_audit()
        .head_ref()
        .expect("first head");
    let second_head = block_on(ledger.bind_effect(second.identity()))
        .expect("bind second")
        .delivery_audit()
        .head_ref()
        .expect("second head");
    let first_stale_head = first_head.clone();
    let first_target = reviewed_value("same-binding.target.first");
    let second_target = reviewed_value("same-binding.target.second");
    let first_target_ref = first_target.reference().expect("first target ref");
    let second_target_ref = second_target.reference().expect("second target ref");
    let first_outcome = domain_outcome(
        "mfm.primitive-stable_id.v1",
        CanonicalValue::String("same-binding.outcome.first".to_owned()),
    );
    let second_outcome = domain_outcome(
        "mfm.primitive-stable_id.v1",
        CanonicalValue::String("same-binding.outcome.second".to_owned()),
    );
    let barrier = Arc::new(Barrier::new(2));
    let invocations = Arc::new(AtomicUsize::new(0));
    let (authority_tx, authority_rx) = std::sync::mpsc::channel();

    let first_task = {
        let ledger = ledger.clone();
        let identity = first.identity().clone();
        let target = first_target.clone();
        let returned = second_outcome.clone();
        let barrier = Arc::clone(&barrier);
        let invocations = Arc::clone(&invocations);
        let authority_tx = authority_tx.clone();
        std::thread::spawn(move || {
            block_on(ledger.execute_target_once(
                &identity,
                &first_head,
                target,
                None,
                move |authority| {
                    invocations.fetch_add(1, Ordering::SeqCst);
                    authority_tx
                        .send((0_u8, CapturedTargetAuthority::from_authority(&authority)))
                        .expect("first authority capture");
                    barrier.wait();
                    async move { returned }
                },
            ))
        })
    };
    let second_task = {
        let ledger = ledger.clone();
        let identity = second.identity().clone();
        let target = second_target.clone();
        let returned = first_outcome.clone();
        let barrier = Arc::clone(&barrier);
        let invocations = Arc::clone(&invocations);
        let authority_tx = authority_tx;
        std::thread::spawn(move || {
            block_on(ledger.execute_target_once(
                &identity,
                &second_head,
                target,
                None,
                move |authority| {
                    invocations.fetch_add(1, Ordering::SeqCst);
                    authority_tx
                        .send((1_u8, CapturedTargetAuthority::from_authority(&authority)))
                        .expect("second authority capture");
                    barrier.wait();
                    async move { returned }
                },
            ))
        })
    };

    let ExecuteTargetOutcome::Observed(first_view) = first_task
        .join()
        .expect("first worker")
        .expect("first observation")
    else {
        panic!("first authorization must be observed")
    };
    let ExecuteTargetOutcome::Observed(second_view) = second_task
        .join()
        .expect("second worker")
        .expect("second observation")
    else {
        panic!("second authorization must be observed")
    };
    assert_eq!(invocations.load(Ordering::SeqCst), 2);
    let first_attempts = first_view
        .delivery_audit()
        .attempts()
        .expect("first attempts");
    let second_attempts = second_view
        .delivery_audit()
        .attempts()
        .expect("second attempts");
    assert_eq!(first_attempts.len(), 1);
    assert_eq!(second_attempts.len(), 1);
    assert_eq!(
        first_attempts[0].outcome().expect("first outcome"),
        &second_outcome
    );
    assert_eq!(
        second_attempts[0].outcome().expect("second outcome"),
        &first_outcome
    );
    assert_ne!(
        first_attempts[0].attempt_id(),
        second_attempts[0].attempt_id()
    );

    let mut captures = [
        authority_rx.recv().expect("first capture"),
        authority_rx.recv().expect("second capture"),
    ];
    captures.sort_by_key(|(ordinal, _)| *ordinal);
    for (capture, expected_identity, expected_attempt, expected_target_ref) in [
        (
            &captures[0].1,
            first.identity(),
            first_attempts[0].attempt_id(),
            &first_target_ref,
        ),
        (
            &captures[1].1,
            second.identity(),
            second_attempts[0].attempt_id(),
            &second_target_ref,
        ),
    ] {
        assert_eq!(&capture.identity, expected_identity);
        assert_eq!(&capture.attempt_id, expected_attempt);
        assert_eq!(&capture.target_operation_ref, expected_target_ref);
        assert_eq!(
            &capture.durable_ledger_generation_ref,
            &fixture.generation_ref
        );
    }

    let stale_invocations = Arc::clone(&invocations);
    let stale = block_on(ledger.execute_target_once(
        first.identity(),
        &first_stale_head,
        first_target,
        None,
        move |_authority| async move {
            stale_invocations.fetch_add(1, Ordering::SeqCst);
            first_outcome
        },
    ))
    .expect("stale authorization decision");
    assert!(matches!(stale, ExecuteTargetOutcome::Contended));
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        2,
        "a stale retry must not remint authority"
    );
}

#[test]
fn cross_contract_schema_and_generation_swaps_normalize_under_each_local_seal() {
    let first_fixture = binding_fixture_with_outcome_contract(
        "cross-outcome-first",
        false,
        bounds(8),
        "mfm.primitive-stable_id.v1",
        reviewed_ref("cross-outcome-first.safe-failure"),
    );
    let second_fixture = binding_fixture_with_outcome_contract(
        "cross-outcome-second",
        false,
        bounds(8),
        "mfm.executor-reference-failure-code.v1",
        reviewed_ref("cross-outcome-second.safe-failure"),
    );
    assert_ne!(
        first_fixture.generation_ref, second_fixture.generation_ref,
        "the adversarial bindings must use distinct durable generations"
    );
    assert_ne!(
        first_fixture.binding.contract().safe_failure_contract_ref(),
        second_fixture
            .binding
            .contract()
            .safe_failure_contract_ref()
    );
    assert_ne!(
        first_fixture
            .binding
            .contract()
            .retained_closure_contract()
            .domain_evidence_contract()
            .schema_id(),
        second_fixture
            .binding
            .contract()
            .retained_closure_contract()
            .domain_evidence_contract()
            .schema_id()
    );

    let first_ledger = KeyedExecutorLedger::new(
        MemoryExecutorStore::new(&first_fixture.binding),
        first_fixture.binding.clone(),
    )
    .expect("first ledger");
    let second_ledger = KeyedExecutorLedger::new(
        MemoryExecutorStore::new(&second_fixture.binding),
        second_fixture.binding.clone(),
    )
    .expect("second ledger");
    let first = committed(
        &first_fixture,
        182,
        "operation.cross-outcome.first",
        "payload",
    );
    let second = committed(
        &second_fixture,
        183,
        "operation.cross-outcome.second",
        "payload",
    );
    let first_head = block_on(first_ledger.bind_effect(first.identity()))
        .expect("bind first")
        .delivery_audit()
        .head_ref()
        .expect("first head");
    let second_head = block_on(second_ledger.bind_effect(second.identity()))
        .expect("bind second")
        .delivery_audit()
        .head_ref()
        .expect("second head");
    let first_stale_head = first_head.clone();
    let second_stale_head = second_head.clone();
    let first_target = reviewed_value("cross-outcome.target.first");
    let second_target = reviewed_value("cross-outcome.target.second");
    let first_target_ref = first_target.reference().expect("first target ref");
    let second_target_ref = second_target.reference().expect("second target ref");
    let first_contract_outcome = DeliveryAttemptOutcome::did_not_enter(
        reference_safe_failure(
            first_fixture
                .binding
                .contract()
                .safe_failure_contract_ref()
                .clone(),
            ReferenceFailureCode::DestinationUnavailable,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
        )
        .expect("first safe failure"),
    )
    .expect("first contract outcome");
    let second_schema_outcome = domain_outcome(
        "mfm.executor-reference-failure-code.v1",
        CanonicalValue::String("destination_unavailable".to_owned()),
    );
    let barrier = Arc::new(Barrier::new(2));
    let invocations = Arc::new(AtomicUsize::new(0));
    let (authority_tx, authority_rx) = std::sync::mpsc::channel();

    let first_task = {
        let ledger = first_ledger.clone();
        let identity = first.identity().clone();
        let target = first_target.clone();
        let returned = second_schema_outcome;
        let barrier = Arc::clone(&barrier);
        let invocations = Arc::clone(&invocations);
        let authority_tx = authority_tx.clone();
        std::thread::spawn(move || {
            block_on(ledger.execute_target_once(
                &identity,
                &first_head,
                target,
                None,
                move |authority| {
                    invocations.fetch_add(1, Ordering::SeqCst);
                    authority_tx
                        .send((0_u8, CapturedTargetAuthority::from_authority(&authority)))
                        .expect("first authority capture");
                    barrier.wait();
                    async move { returned }
                },
            ))
        })
    };
    let second_task = {
        let ledger = second_ledger.clone();
        let identity = second.identity().clone();
        let target = second_target.clone();
        let returned = first_contract_outcome;
        let barrier = Arc::clone(&barrier);
        let invocations = Arc::clone(&invocations);
        let authority_tx = authority_tx;
        std::thread::spawn(move || {
            block_on(ledger.execute_target_once(
                &identity,
                &second_head,
                target,
                None,
                move |authority| {
                    invocations.fetch_add(1, Ordering::SeqCst);
                    authority_tx
                        .send((1_u8, CapturedTargetAuthority::from_authority(&authority)))
                        .expect("second authority capture");
                    barrier.wait();
                    async move { returned }
                },
            ))
        })
    };

    let ExecuteTargetOutcome::Observed(first_view) = first_task
        .join()
        .expect("first worker")
        .expect("first observation")
    else {
        panic!("first authorization must be observed")
    };
    let ExecuteTargetOutcome::Observed(second_view) = second_task
        .join()
        .expect("second worker")
        .expect("second observation")
    else {
        panic!("second authorization must be observed")
    };
    assert_eq!(invocations.load(Ordering::SeqCst), 2);
    let first_attempts = first_view
        .delivery_audit()
        .attempts()
        .expect("first attempts");
    let second_attempts = second_view
        .delivery_audit()
        .attempts()
        .expect("second attempts");
    assert_eq!(first_attempts.len(), 1);
    assert_eq!(second_attempts.len(), 1);
    assert_adapter_contract_violation(first_attempts[0].outcome().expect("first outcome"));
    assert_adapter_contract_violation(second_attempts[0].outcome().expect("second outcome"));

    let mut captures = [
        authority_rx.recv().expect("first capture"),
        authority_rx.recv().expect("second capture"),
    ];
    captures.sort_by_key(|(ordinal, _)| *ordinal);
    for (
        capture,
        expected_identity,
        expected_attempt,
        expected_target_ref,
        expected_generation_ref,
    ) in [
        (
            &captures[0].1,
            first.identity(),
            first_attempts[0].attempt_id(),
            &first_target_ref,
            &first_fixture.generation_ref,
        ),
        (
            &captures[1].1,
            second.identity(),
            second_attempts[0].attempt_id(),
            &second_target_ref,
            &second_fixture.generation_ref,
        ),
    ] {
        assert_eq!(&capture.identity, expected_identity);
        assert_eq!(&capture.attempt_id, expected_attempt);
        assert_eq!(&capture.target_operation_ref, expected_target_ref);
        assert_eq!(
            &capture.durable_ledger_generation_ref,
            expected_generation_ref
        );
    }

    for (ledger, identity, stale_head, target) in [
        (
            &first_ledger,
            first.identity(),
            &first_stale_head,
            first_target,
        ),
        (
            &second_ledger,
            second.identity(),
            &second_stale_head,
            second_target,
        ),
    ] {
        let stale_invocations = Arc::clone(&invocations);
        let stale = block_on(ledger.execute_target_once(
            identity,
            stale_head,
            target,
            None,
            move |_authority| async move {
                stale_invocations.fetch_add(1, Ordering::SeqCst);
                domain_outcome(
                    "mfm.primitive-stable_id.v1",
                    CanonicalValue::String("must-not-run".to_owned()),
                )
            },
        ))
        .expect("stale authorization decision");
        assert!(matches!(stale, ExecuteTargetOutcome::Contended));
    }
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        2,
        "stale retries must not remint either authority"
    );
}

#[test]
fn post_return_completion_seal_retries_absent_history_and_non_operational_store_errors() {
    let fixture = binding_fixture("post-return-retry", false, 8);
    let raw = MemoryExecutorStore::new(&fixture.binding);
    let store = RetryingObservationStore::new(raw, 1);
    let ledger = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
        .expect("keyed executor ledger");
    let request = committed(&fixture, 92, "operation.post-return-retry", "payload");
    let initial = block_on(ledger.bind_effect(request.identity())).expect("bind");
    let expected_head = initial.delivery_audit().head_ref().expect("initial head");
    let failure = reference_safe_failure(
        fixture
            .binding
            .contract()
            .safe_failure_contract_ref()
            .clone(),
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BeforeBoundaryEntry,
    )
    .expect("failure");
    let outcome =
        mfm_executor::DeliveryAttemptOutcome::did_not_enter(failure).expect("target outcome");
    let invocations = Arc::new(AtomicUsize::new(0));
    let invoked = Arc::clone(&invocations);
    let store_for_completion = store.clone();
    let observed = block_on(ledger.execute_target_once(
        request.identity(),
        &expected_head,
        reviewed_value("post-return-retry.target"),
        None,
        move |_authority| async move {
            invoked.fetch_add(1, Ordering::SeqCst);
            store_for_completion.return_absent_on_next_load();
            outcome
        },
    ))
    .expect("eventually sealed");
    assert!(matches!(observed, ExecuteTargetOutcome::Observed(_)));
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert_eq!(store.observation_attempts(), 2);
}

#[test]
fn cancelling_before_target_return_leaves_only_the_authorized_attempt() {
    let fixture = binding_fixture("pre-return-cancel", false, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store.clone(), fixture.binding.clone()).expect("keyed ledger");
    let request = committed(&fixture, 94, "operation.pre-return-cancel", "payload");
    let initial = block_on(ledger.bind_effect(request.identity())).expect("bind");
    let expected_head = initial.delivery_audit().head_ref().expect("initial head");
    let callback_started = Arc::new(AtomicBool::new(false));
    let started = Arc::clone(&callback_started);
    let cancelled = block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_millis(40),
            ledger.execute_target_once(
                request.identity(),
                &expected_head,
                reviewed_value("pre-return-cancel.target"),
                None,
                move |_authority| async move {
                    started.store(true, Ordering::SeqCst);
                    std::future::pending::<()>().await;
                    unreachable!("the cancelled target callback cannot return")
                },
            ),
        )
        .await
    });
    assert!(
        cancelled.is_err(),
        "timeout must cancel the target callback before return"
    );
    assert!(
        callback_started.load(Ordering::SeqCst),
        "the target callback must have received fresh authority"
    );

    let reopened = KeyedExecutorLedger::new(store, fixture.binding).expect("reopened ledger");
    let pending = block_on(reopened.effect_view(request.identity()))
        .expect("load authorized effect")
        .expect("bound effect");
    let attempts = pending.delivery_audit().attempts().expect("attempts");
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].outcome().is_none());
}

#[test]
fn cancelling_after_target_return_drops_only_the_completion_retry_future() {
    let fixture = binding_fixture("post-return-cancel", false, 8);
    let raw = MemoryExecutorStore::new(&fixture.binding);
    let store = RetryingObservationStore::new(raw.clone(), usize::MAX);
    let ledger = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
        .expect("keyed executor ledger");
    let request = committed(&fixture, 93, "operation.post-return-cancel", "payload");
    let initial = block_on(ledger.bind_effect(request.identity())).expect("bind");
    let expected_head = initial.delivery_audit().head_ref().expect("initial head");
    let failure = reference_safe_failure(
        fixture
            .binding
            .contract()
            .safe_failure_contract_ref()
            .clone(),
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BeforeBoundaryEntry,
    )
    .expect("failure");
    let outcome =
        mfm_executor::DeliveryAttemptOutcome::did_not_enter(failure).expect("target outcome");
    let invocations = Arc::new(AtomicUsize::new(0));
    let invoked = Arc::clone(&invocations);
    let callback_returned = Arc::new(AtomicBool::new(false));
    let returned = Arc::clone(&callback_returned);
    let cancelled = block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_millis(40),
            ledger.execute_target_once(
                request.identity(),
                &expected_head,
                reviewed_value("post-return-cancel.target"),
                None,
                move |_authority| async move {
                    invoked.fetch_add(1, Ordering::SeqCst);
                    returned.store(true, Ordering::SeqCst);
                    outcome
                },
            ),
        )
        .await
    });
    assert!(
        cancelled.is_err(),
        "timeout must cancel the completion retry future"
    );
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert!(
        callback_returned.load(Ordering::SeqCst),
        "cancellation must occur after the target callback returned"
    );
    assert!(store.observation_attempts() >= 1);

    let reopened =
        KeyedExecutorLedger::new(raw, fixture.binding.clone()).expect("reopened executor ledger");
    let pending = block_on(reopened.effect_view(request.identity()))
        .expect("load authorized effect")
        .expect("bound effect");
    let attempts = pending.delivery_audit().attempts().expect("attempts");
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].outcome().is_none());
}

#[test]
fn target_bracket_tombstone_and_restart_are_stable_without_a_second_exchange() {
    let fixture = binding_fixture("target-bracket-restart", false, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
        .expect("keyed executor ledger");
    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate");
    let contract = reference_contract(&fixture);
    let request = committed(&fixture, 9, "operation.bracket", "payload");
    let identity = request.identity();
    let bound = block_on(ledger.bind_effect(identity)).expect("bind");
    let expected_head = bound.delivery_audit().head_ref().expect("bound head");
    let outcome = block_on(ledger.execute_target_once(
        identity,
        &expected_head,
        contract.enqueue_operation().clone(),
        None,
        |authority| async {
            destination
                .enqueue(
                    authority,
                    request.request(),
                    &contract,
                    ReferenceTargetBehavior::Available,
                )
                .await
                .into_outcome()
        },
    ))
    .expect("target bracket");
    let ExecuteTargetOutcome::Observed(view) = outcome else {
        panic!("fresh target bracket must be uncontended")
    };
    let attempt = view
        .delivery_audit()
        .attempts()
        .expect("attempts")
        .into_iter()
        .last()
        .expect("observed attempt");
    let proof = ReferenceTerminalProof::new(
        attempt.attempt_id().clone(),
        attempt
            .outcome()
            .and_then(mfm_executor::DeliveryAttemptOutcome::returned_outcome)
            .cloned()
            .expect("returned outcome"),
        attempt
            .returned_observation_ref()
            .cloned()
            .expect("observation ref"),
    )
    .expect("proof");
    assert_eq!(
        TerminalTombstone::new("x".repeat(257), "applied", proof.clone())
            .expect_err("oversized tombstone identity"),
        ExecutorError::InvalidReferenceEffectIdentifier
    );
    block_on(ledger.append_terminal_tombstone(
        identity,
        TerminalTombstone::new("operation.bracket", "applied", proof).expect("tombstone"),
    ))
    .expect("tombstone append");

    let checkpoint = store.checkpoint().expect("checkpoint");
    let restored_store =
        MemoryExecutorStore::restore(&fixture.binding, checkpoint).expect("restore");
    let restored =
        KeyedExecutorLedger::new(restored_store, fixture.binding.clone()).expect("restored ledger");
    let terminal = block_on(restored.effect_view(identity))
        .expect("restored view")
        .expect("bound effect");
    assert!(terminal.terminal_tombstone().is_some());
    assert_eq!(destination.target_entry_count().expect("entry count"), 1);
    assert_eq!(
        block_on(restored.execute_target_once(
            identity,
            &terminal.delivery_audit().head_ref().expect("terminal head"),
            contract.enqueue_operation().clone(),
            None,
            |_| async { panic!("terminal effect must not mint target authority") },
        ))
        .expect_err("terminal authorization"),
        ExecutorError::EffectAlreadyTerminal
    );
    assert_eq!(destination.target_entry_count().expect("entry count"), 1);
}

#[test]
fn reference_effect_identifier_bound_is_exact_and_pre_persistence() {
    assert!(ReferenceRequest::new("x".repeat(256), payload_value_ref("payload"),).is_ok());
    assert_eq!(
        ReferenceRequest::new("x".repeat(257), payload_value_ref("payload"),)
            .expect_err("257 bytes"),
        ExecutorError::InvalidReferenceEffectIdentifier
    );
    assert_eq!(
        ReferenceRequest::new("", payload_value_ref("payload")).expect_err("empty"),
        ExecutorError::InvalidReferenceEffectIdentifier
    );
}

#[test]
fn prebound_effect_upgrades_to_one_atomic_resource_allocation() {
    let fixture = binding_fixture("prebound-account-policy", true, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let policy = AccountSequencePolicy::new(
        ResourcePolicyBinding::new(
            reviewed_ref("prebound.account.policy"),
            reviewed_ref("prebound.account.configuration"),
        ),
        31,
        None,
    );
    let policy_request =
        AccountSequenceRequest::new("prebound-sender", "chain-1").expect("policy request");
    let request = committed(&fixture, 17, "operation.prebound", "payload");

    block_on(ledger.bind_effect(request.identity())).expect("prebind effect");
    let allocation =
        block_on(ledger.try_bind_and_allocate(request.identity(), None, &policy, &policy_request))
            .expect("upgrade prebound effect");
    let AllocationOutcome::Allocated {
        allocation,
        resource_head,
        ..
    } = allocation
    else {
        panic!("prebound effect must allocate once");
    };
    assert_eq!(allocation.sequence(), 31);

    let view = block_on(ledger.effect_view(request.identity()))
        .expect("effect view")
        .expect("effect");
    assert_eq!(
        view.delivery_audit()
            .frontiers()
            .iter()
            .flat_map(|frontier| frontier.appended_records())
            .filter(|record| matches!(record, ExecutorEvidenceRecord::EffectBound { .. }))
            .count(),
        1
    );
    assert_eq!(
        view.delivery_audit()
            .frontiers()
            .iter()
            .flat_map(|frontier| frontier.appended_records())
            .filter(|record| matches!(record, ExecutorEvidenceRecord::ResourceAllocated(_)))
            .count(),
        1
    );
    let resource_key = policy.resource_key(&policy_request).expect("resource key");
    assert_eq!(
        block_on(ledger.resource_head(&resource_key)).expect("resource head"),
        Some(resource_head)
    );
    assert_eq!(
        block_on(ledger.resource_records(&resource_key))
            .expect("resource records")
            .len(),
        1
    );
    assert!(matches!(
        block_on(ledger.try_bind_and_allocate(request.identity(), None, &policy, &policy_request,))
            .expect("idempotent allocation"),
        AllocationOutcome::Existing { .. }
    ));
}

#[test]
fn strict_store_refold_requires_exact_content_closure_and_durable_codecs() {
    let fixture = binding_fixture("strict-store-refold", true, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let policy = AccountSequencePolicy::new(
        ResourcePolicyBinding::new(
            reviewed_ref("strict.account.policy"),
            reviewed_ref("strict.account.configuration"),
        ),
        41,
        None,
    );
    let policy_request =
        AccountSequenceRequest::new("strict-sender", "chain-1").expect("policy request");
    let request = committed(&fixture, 18, "operation.strict", "payload");
    block_on(ledger.try_bind_and_allocate(request.identity(), None, &policy, &policy_request))
        .expect("allocation");
    let target_operation = reviewed_value("strict.target-operation");
    let target_operation_ref = target_operation.reference().expect("target operation ref");
    let current = block_on(ledger.effect_view(request.identity()))
        .expect("effect view")
        .expect("bound effect");
    let safe_failure = reference_safe_failure(
        fixture
            .binding
            .contract()
            .safe_failure_contract_ref()
            .clone(),
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BeforeBoundaryEntry,
    )
    .expect("safe failure");
    let target_outcome =
        mfm_executor::DeliveryAttemptOutcome::did_not_enter(safe_failure).expect("outcome");
    let appended = block_on(ledger.execute_target_once(
        request.identity(),
        &current.delivery_audit().head_ref().expect("head"),
        target_operation.clone(),
        Some(policy.binding()),
        move |_authority| async move { target_outcome },
    ))
    .expect("target bracket");
    let ExecuteTargetOutcome::Observed(view) = appended else {
        panic!("target bracket must be uncontended")
    };
    let attempt_id = view
        .delivery_audit()
        .attempts()
        .expect("attempts")
        .into_iter()
        .last()
        .expect("attempt")
        .attempt_id()
        .clone();
    assert_eq!(
        block_on(ledger.target_operation(request.identity(), &attempt_id))
            .expect("target operation"),
        target_operation
    );
    let restored_store = MemoryExecutorStore::restore(
        &fixture.binding,
        MemoryLedgerCheckpoint::from_durable_bytes(
            &ledger
                .store()
                .checkpoint()
                .expect("checkpoint")
                .to_durable_bytes()
                .expect("checkpoint bytes"),
        )
        .expect("checkpoint decode"),
    )
    .expect("checkpoint restore");
    let restored = KeyedExecutorLedger::new(restored_store, fixture.binding.clone())
        .expect("restored keyed executor");
    assert_eq!(
        block_on(restored.target_operation(request.identity(), &attempt_id))
            .expect("restored target operation")
            .reference()
            .expect("restored target ref"),
        target_operation_ref
    );

    let snapshot = ledger.store().full_snapshot().expect("full snapshot");
    ledger.strict_refold(&snapshot).expect("strict refold");
    for effect in snapshot.effects() {
        for frontier in effect.frontiers() {
            assert_eq!(
                mfm_executor::DeliveryAuditFrontier::from_durable_bytes(
                    &frontier.to_durable_bytes().expect("frontier bytes"),
                )
                .expect("frontier decode"),
                *frontier
            );
        }
    }
    for resource in snapshot.resources() {
        for record in resource.records() {
            assert_eq!(
                ResourceLedgerRecord::from_durable_bytes(
                    &record.to_durable_bytes().expect("resource bytes"),
                )
                .expect("resource decode"),
                *record
            );
        }
    }

    let mut missing_content = snapshot.content().to_vec();
    missing_content.retain(|value| value.reference().as_ref() != Ok(&target_operation_ref));
    let missing = ExecutorStoreSnapshot::from_store_parts(
        snapshot.store_identity().clone(),
        snapshot.effects().to_vec(),
        snapshot.resources().to_vec(),
        missing_content,
    );
    assert_eq!(
        ledger.strict_refold(&missing).expect_err("missing content"),
        ExecutorError::RetainedClosureIncomplete
    );

    let mut duplicate_content = snapshot.content().to_vec();
    duplicate_content.push(snapshot.content()[0].clone());
    let duplicate = ExecutorStoreSnapshot::from_store_parts(
        snapshot.store_identity().clone(),
        snapshot.effects().to_vec(),
        snapshot.resources().to_vec(),
        duplicate_content,
    );
    assert_eq!(
        ledger
            .strict_refold(&duplicate)
            .expect_err("duplicate content"),
        ExecutorError::RetainedClosureDuplicate
    );

    let mut extra_content = snapshot.content().to_vec();
    extra_content.push(
        SchemaQualifiedCanonicalValue::from_validated(&payload_value_ref("unreachable"))
            .expect("extra content"),
    );
    let extra = ExecutorStoreSnapshot::from_store_parts(
        snapshot.store_identity().clone(),
        snapshot.effects().to_vec(),
        snapshot.resources().to_vec(),
        extra_content,
    );
    assert_eq!(
        ledger.strict_refold(&extra).expect_err("extra content"),
        ExecutorError::RetainedClosureExtra
    );
}

#[test]
fn account_sequence_policy_is_atomic_and_refolds_after_restart() {
    let fixture = binding_fixture("account-policy", true, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let policy_binding = ResourcePolicyBinding::new(
        reviewed_ref("account.policy"),
        reviewed_ref("account.configuration"),
    );
    let policy = AccountSequencePolicy::new(
        policy_binding.clone(),
        7,
        Some(FencingRef::from_reviewed(reviewed_ref("account.fence"))),
    );
    let policy_request = AccountSequenceRequest::new("alice", "chain-1").expect("policy request");
    let first = committed(&fixture, 10, "operation.sequence.1", "payload");
    let second = committed(&fixture, 11, "operation.sequence.2", "payload");
    let barrier = Arc::new(Barrier::new(2));

    let mut workers = Vec::new();
    for identity in [first.identity().clone(), second.identity().clone()] {
        let ledger = ledger.clone();
        let policy = policy.clone();
        let request = policy_request.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            (
                identity.clone(),
                block_on(ledger.try_bind_and_allocate(&identity, None, &policy, &request)),
            )
        }));
    }
    let results = workers
        .into_iter()
        .map(|worker| worker.join().expect("worker"))
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| matches!(result, Ok(AllocationOutcome::Allocated { .. })))
            .count(),
        1
    );
    let loser = results
        .iter()
        .find(|(_, result)| matches!(result, Err(ExecutorError::ResourceCasMismatch)))
        .map(|(identity, _)| identity.clone())
        .expect("CAS loser");
    let winner = results
        .iter()
        .find(|(_, result)| matches!(result, Ok(AllocationOutcome::Allocated { .. })))
        .map(|(identity, _)| identity.clone())
        .expect("CAS winner");
    let winner_request = if first.identity() == &winner {
        &first
    } else {
        &second
    };
    let resource_key = policy.resource_key(&policy_request).expect("resource key");
    let head = block_on(ledger.resource_head(&resource_key))
        .expect("head")
        .expect("head present");
    assert_eq!(
        block_on(ledger.try_bind_and_allocate(&loser, Some(&head), &policy, &policy_request,))
            .expect_err("pending predecessor"),
        ExecutorError::ResourcePolicy(mfm_executor::PolicyError::PriorSequencePending)
    );
    terminalize_allocated_effect(&ledger, &fixture, winner_request, &policy_binding);
    let retry =
        block_on(ledger.try_bind_and_allocate(&loser, Some(&head), &policy, &policy_request))
            .expect("retry");
    let AllocationOutcome::Allocated { allocation, .. } = retry else {
        panic!("expected new allocation");
    };
    assert_eq!(allocation.sequence(), 8);

    let checkpoint = ledger.store().checkpoint().expect("checkpoint");
    let bytes = checkpoint.to_durable_bytes().expect("bytes");
    let decoded = MemoryLedgerCheckpoint::from_durable_bytes(&bytes).expect("decode");
    let restored_store = MemoryExecutorStore::restore(&fixture.binding, decoded).expect("restore");
    let restored = KeyedExecutorLedger::new(restored_store, fixture.binding.clone())
        .expect("restored keyed executor");
    let existing = block_on(restored.try_bind_and_allocate(&loser, None, &policy, &policy_request))
        .expect("existing allocation");
    assert!(matches!(existing, AllocationOutcome::Existing { .. }));

    let wrong_policy = AccountSequencePolicy::new(
        ResourcePolicyBinding::new(
            reviewed_ref("account.policy"),
            reviewed_ref("account.changed-configuration"),
        ),
        7,
        Some(FencingRef::from_reviewed(reviewed_ref("account.fence"))),
    );
    assert_eq!(
        block_on(restored.try_bind_and_allocate(&loser, None, &wrong_policy, &policy_request,))
            .expect_err("policy swap"),
        ExecutorError::ResourceAllocationConflict
    );
    let current = block_on(restored.effect_view(&loser))
        .expect("effect view")
        .expect("bound effect");
    let expected_head = current.delivery_audit().head_ref().expect("head");
    assert_eq!(
        block_on(restored.execute_target_once(
            &loser,
            &expected_head,
            reviewed_value("account.submit"),
            None,
            |_| async { panic!("invalid policy pair must not mint authority") },
        ))
        .expect_err("missing policy pair"),
        ExecutorError::ResourcePolicyNotRevalidated
    );
    let failure = reference_safe_failure(
        fixture
            .binding
            .contract()
            .safe_failure_contract_ref()
            .clone(),
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BeforeBoundaryEntry,
    )
    .expect("failure");
    let outcome = mfm_executor::DeliveryAttemptOutcome::did_not_enter(failure).expect("outcome");
    let appended = block_on(restored.execute_target_once(
        &loser,
        &expected_head,
        reviewed_value("account.submit"),
        Some(&policy_binding),
        move |_authority| async move { outcome },
    ))
    .expect("exact policy pair");
    assert!(matches!(appended, ExecuteTargetOutcome::Observed(_)));
}

#[test]
fn finite_inventory_is_permanent_and_exhausts() {
    let fixture = binding_fixture("inventory-policy", true, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let policy = FiniteInventoryPolicy::new(
        ResourcePolicyBinding::new(
            reviewed_ref("inventory.policy"),
            reviewed_ref("inventory.configuration"),
        ),
        "warehouse",
        [("widget".to_owned(), 2)],
        reviewed_ref("inventory.destination-precondition"),
    )
    .expect("policy");
    let policy_request = FiniteInventoryRequest::new("warehouse", "widget").expect("request");
    let identities = [12_u8, 13, 14].map(|seed| {
        committed(
            &fixture,
            seed,
            &format!("operation.inventory.{seed}"),
            "payload",
        )
    });
    let first = block_on(ledger.try_bind_and_allocate(
        identities[0].identity(),
        None,
        &policy,
        &policy_request,
    ))
    .expect("first");
    let first_head = match first {
        AllocationOutcome::Allocated { resource_head, .. } => resource_head,
        AllocationOutcome::Existing { .. } => {
            panic!("new effect must allocate")
        }
    };
    let second = block_on(ledger.try_bind_and_allocate(
        identities[1].identity(),
        Some(&first_head),
        &policy,
        &policy_request,
    ))
    .expect("second");
    let second_head = match second {
        AllocationOutcome::Allocated { resource_head, .. } => resource_head,
        AllocationOutcome::Existing { .. } => {
            panic!("new effect must allocate")
        }
    };
    assert_eq!(
        block_on(ledger.try_bind_and_allocate(
            identities[2].identity(),
            Some(&second_head),
            &policy,
            &policy_request,
        ))
        .expect_err("inventory exhausted"),
        ExecutorError::ResourcePolicy(mfm_executor::PolicyError::InventoryExhausted)
    );
}

#[test]
fn checkpoints_reject_corruption_and_restore_only_contract_bounds() {
    let fixture = binding_fixture("checkpoint", false, 4);
    let expected_bounds = bounds(4);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let request = committed(&fixture, 15, "operation.checkpoint", "payload");
    block_on(ledger.bind_effect(request.identity())).expect("bind effect");
    let bytes = ledger
        .store()
        .checkpoint()
        .expect("checkpoint")
        .to_durable_bytes()
        .expect("bytes");

    let mut corrupt = bytes.clone();
    let index = corrupt.len() / 2;
    corrupt[index] ^= 0x80;
    assert_eq!(
        MemoryLedgerCheckpoint::from_durable_bytes(&corrupt).expect_err("corrupt"),
        ExecutorError::InvalidDurableSnapshot
    );
    assert_eq!(
        MemoryLedgerCheckpoint::from_durable_bytes(&bytes[..bytes.len() - 1])
            .expect_err("truncated"),
        ExecutorError::InvalidDurableSnapshot
    );
    let mut wrong_family = bytes.clone();
    wrong_family[..8].copy_from_slice(b"MFMBAD01");
    let payload_end = wrong_family.len() - 32;
    let wrong_family_digest = sha256_digest_bytes(&wrong_family[..payload_end]);
    wrong_family[payload_end..].copy_from_slice(wrong_family_digest.as_bytes());
    assert_eq!(
        MemoryLedgerCheckpoint::from_durable_bytes(&wrong_family)
            .expect_err("wrong checkpoint family"),
        ExecutorError::InvalidDurableSnapshot
    );
    let checkpoint = MemoryLedgerCheckpoint::from_durable_bytes(&bytes).expect("decode");
    let restored = MemoryExecutorStore::restore(&fixture.binding, checkpoint).expect("restore");
    let restored =
        KeyedExecutorLedger::new(restored, fixture.binding.clone()).expect("restored ledger");
    assert_eq!(restored.evidence_bounds(), &expected_bounds);

    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.generation_ref)
        .expect("activate");
    let destination_bytes = destination
        .checkpoint()
        .expect("checkpoint")
        .to_durable_bytes()
        .expect("bytes");
    let mut corrupt = destination_bytes;
    corrupt[9] ^= 0x01;
    assert_eq!(
        MemoryDestinationCheckpoint::from_durable_bytes(&corrupt).expect_err("corrupt destination"),
        ExecutorError::InvalidDurableSnapshot
    );
}

#[derive(Clone)]
struct SecretInjectingDestination {
    inner: MemoryConvergentDestination,
    credential: Arc<String>,
}

impl ReferenceDestination for SecretInjectingDestination {
    fn enqueue<'a>(
        &'a self,
        authority: TargetEntryAuthority,
        request: &'a ReferenceRequest,
        contract: &'a ReferenceContract,
        behavior: ReferenceTargetBehavior,
    ) -> ExecutorFuture<'a, ReferenceDestinationReturn> {
        let _credential_used_below_boundary = self.credential.as_bytes();
        self.inner.enqueue(authority, request, contract, behavior)
    }
}

#[test]
fn credentials_injected_below_target_entry_never_reach_retained_surfaces() {
    let fixture = binding_fixture("secret-scan", false, 8);
    let inner = MemoryConvergentDestination::new();
    inner
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate");
    let secret = "correct horse battery staple bearer credential".to_owned();
    let destination = SecretInjectingDestination {
        inner: inner.clone(),
        credential: Arc::new(secret.clone()),
    };
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let executor =
        ReferenceExecutor::new(ledger.clone(), destination, reference_contract(&fixture));
    let request = committed(&fixture, 16, "operation.secret", "payload");
    let terminal = block_on(executor.drive(&request)).expect("terminal");
    let Ensure::Terminal { evidence } = terminal.outcome() else {
        panic!("expected terminal");
    };

    let mut surfaces = vec![
        ledger
            .store()
            .checkpoint()
            .expect("checkpoint")
            .to_durable_bytes()
            .expect("ledger bytes"),
        inner
            .checkpoint()
            .expect("checkpoint")
            .to_durable_bytes()
            .expect("destination bytes"),
        evidence
            .terminal_tombstone()
            .validated()
            .expect("tombstone")
            .as_bytes()
            .to_vec(),
        evidence
            .terminal_proof()
            .validated()
            .expect("proof")
            .as_bytes()
            .to_vec(),
        evidence.domain_evidence().as_bytes().to_vec(),
    ];
    for frontier in terminal.delivery_audit().frontiers() {
        surfaces.push(frontier.validated().expect("frontier").as_bytes().to_vec());
    }
    let fingerprint = sha256_digest_bytes(secret.as_bytes()).to_string();
    for surface in surfaces {
        assert!(!contains(&surface, secret.as_bytes()));
        assert!(!contains(&surface, fingerprint.as_bytes()));
    }
}

#[test]
fn all_587_frozen_vectors_are_consumed_by_the_shared_authority() {
    recoverability_v1::run_consumer("mfm-executor", |owner| {
        recoverability_v1::assert_lower_layer_owner_vector(owner);
        assert_executor_owner_vector(owner);
    });
}

fn assert_executor_owner_vector(owner: recoverability_v1::OwnerVector<'_>) {
    let vector = owner.vector();
    match owner {
        recoverability_v1::OwnerVector::RelationalPositive(_) => match owner.kind() {
            "frontier_order" => assert_eq!(
                recoverability_v1::string(vector, "expected"),
                "ancestor_or_equal_does_not_regress_descendant_advances"
            ),
            "relational_acceptance" => assert_eq!(
                recoverability_v1::string(vector, "expected"),
                "exact_returned_observation_matches"
            ),
            "request_identity" => assert_eq!(
                recoverability_v1::string(vector, "expected"),
                "different_request_digest"
            ),
            "resource_refold" => assert_eq!(
                recoverability_v1::string(vector, "expected"),
                "restored_policy_and_configuration_match_before_authorization"
            ),
            "type_separation" => assert_eq!(
                recoverability_v1::string(vector, "expected"),
                "non_substitutable"
            ),
            "commit_coordinate_separation"
            | "content_identity_separation"
            | "export_identity"
            | "identity_separation"
            | "read_returned_validation_verdict"
            | "read_safe_failure_verdict"
            | "schema_identity" => {
                assert!(!owner.id().is_empty());
            }
            other => panic!(
                "{}: unclassified executor callback kind {other}",
                owner.id()
            ),
        },
        recoverability_v1::OwnerVector::RelationalRejection(_) => {
            let target = recoverability_v1::string(vector, "target");
            let expected = recoverability_v1::string(vector, "expected_error");
            let mapped = match (target, expected) {
                ("mfm.initial-binding.v1", "binding_conflict") => {
                    Some(ExecutorError::EffectBindingConflict)
                }
                ("mfm.evidence-bounds.v1", "reserve_exhausted") => {
                    Some(ExecutorError::EvidenceBoundsExhausted)
                }
                ("mfm.executor-delivery-frontier.v1", "evidence_chain_fork") => {
                    Some(ExecutorError::FrontierFork)
                }
                (
                    "mfm.executor-reference-terminal-proof.v1",
                    "attempt_observation_mismatch" | "receipt_link_missing",
                )
                | ("mfm.executor-terminal-tombstone.v1", "tombstone_proof_mismatch") => {
                    Some(ExecutorError::TerminalProofMismatch)
                }
                ("mfm.executor-resource-ledger-record.v1", "resource_policy_mismatch") => {
                    Some(ExecutorError::ResourcePolicyNotRevalidated)
                }
                ("mfm.tenant-scope-id.v1", "tenant_mismatch") => {
                    Some(ExecutorError::TenantScopeMismatch)
                }
                ("mfm.content-ref.v1", "content_ref_requires_sha256_v1")
                | ("mfm.value-ref.v1", "value_ref_required") => {
                    Some(ExecutorError::SchemaReferenceMismatch)
                }
                (
                    "batch_legality"
                    | "content_addressing"
                    | "identity_domains"
                    | "identity_encodings"
                    | "limits"
                    | "logical_keys"
                    | "mfm.canonical-expansion-path.v1"
                    | "mfm.commit-envelope.v1"
                    | "mfm.fact-publication-routing.v1"
                    | "mfm.fact-selection-completeness.v1"
                    | "mfm.journal-predecessor.v1"
                    | "mfm.legal-commit-batch.v1"
                    | "mfm.non-domain-failure.v1"
                    | "recoverability_annex"
                    | "schema_algebra"
                    | "schema_arrays"
                    | "schema_registry"
                    | "transient_authority_types",
                    _,
                ) => None,
                _ => panic!(
                    "{}: unclassified executor rejection {target}/{expected}",
                    owner.id()
                ),
            };
            if let Some(error) = mapped {
                assert!(!error.to_string().is_empty(), "{}", owner.id());
            }
        }
    }
}

fn payload_value_ref(role: &str) -> ValidatedCanonicalValue {
    let corpus: serde_json::Value = serde_json::from_str(CORPUS).expect("corpus");
    let vector = corpus["positive_vectors"]
        .as_array()
        .expect("vectors")
        .iter()
        .find(|vector| vector["schema_contract"].as_str() == Some("mfm.value-ref.v1"))
        .expect("value-ref vector");
    let bytes = decode_hex(vector["input_hex"].as_str().expect("hex"));
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("value-ref json");
    value["role"] = serde_json::Value::String(role.to_owned());
    let bytes = serde_json::to_vec(&value).expect("json bytes");
    contract()
        .strict_decode("mfm.value-ref.v1", &bytes)
        .expect("value ref")
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]))
        .collect()
}

fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("invalid lowercase hex"),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}
