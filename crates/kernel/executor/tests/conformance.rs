use std::sync::{Arc, Barrier};

use mfm_canonical::{
    sha256_digest_bytes, CanonicalValue, RecoverabilityContractV1, ValidatedCanonicalValueV1,
};
use mfm_executor::{
    verify_ensure_result, AccountSequencePolicy, AccountSequenceRequest, AllocationOutcome,
    CommittedEffectRequest, ContentRef, EffectIdentity, Ensure, EvidenceBounds, ExecutorBinding,
    ExecutorContractDescriptor, ExecutorDeployment, ExecutorEnsureResultClaim, ExecutorError,
    ExecutorEvidenceRecord, ExecutorFuture, ExecutorRetainedClosureClaim,
    ExecutorRetainedClosureContract, ExecutorRetainedValue, ExecutorRetainedValueRelation,
    ExecutorStoreSnapshot, ExecutorTerminalEvidenceClaim, FencingRef, FiniteInventoryPolicy,
    FiniteInventoryRequest, KeyedExecutorLedger, MemoryConvergentDestination,
    MemoryDestinationCheckpoint, MemoryExecutorStore, MemoryLedgerCheckpoint, ReferenceContract,
    ReferenceCrashPoint, ReferenceDestination, ReferenceDestinationReturn, ReferenceDriveOutcome,
    ReferenceExecutor, ReferenceRequest, ReferenceTargetBehavior, ReferenceTerminalProof,
    ResourceLedgerRecord, ResourceOwnership, ResourcePolicyBinding, RetainedValueContract,
    SchemaQualifiedCanonicalValue, TargetEntryAuthority, TerminalTombstone, TypedResourcePolicy,
    VerifiedExecutorBinding,
};
use mfm_ids::{
    DigestAlgorithm, NodeId, RunId, SemanticTypeId, StableId, StoreScopeId, TenantScopeId,
};

#[path = "../../../../tests/support/recoverability_v1.rs"]
mod recoverability_v1;

const CORPUS: &str = include_str!("../../../../contracts/recoverability/v1/corpus.json");

#[derive(Clone)]
struct BindingFixture {
    binding: VerifiedExecutorBinding,
    tenant_scope_id: TenantScopeId,
    generation_ref: ContentRef,
    destination_domain_ref: ContentRef,
}

fn contract() -> &'static RecoverabilityContractV1 {
    RecoverabilityContractV1::embedded().expect("embedded contract")
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

fn retained_closure_contract(label: &str) -> ExecutorRetainedClosureContract {
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
            "mfm.executor-reference-queue-result.v1",
        ),
    )
    .expect("retained closure contract")
}

fn binding_fixture(label: &str, with_resource_owner: bool, max_attempts: u32) -> BindingFixture {
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
        retained_closure_contract(label),
        reviewed_ref("reference.safe-failure"),
        destination_domain_ref.clone(),
        bounds(max_attempts),
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

fn bounds(max_attempts: u32) -> EvidenceBounds {
    EvidenceBounds::new(max_attempts, 256, 4_000_000, 2, 16_384).expect("bounds")
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
    let authority = block_on(ledger.authorize_target(
        request.identity(),
        contract.enqueue_operation().clone(),
        Some(policy_binding),
    ))
    .expect("allocated target authority");
    let returned = block_on(destination.enqueue(
        authority,
        request.request(),
        &contract,
        ReferenceTargetBehavior::Available,
    ))
    .expect("allocated target");
    let attempt_id = returned.receipt().attempt_id().clone();
    let returned_outcome = returned
        .receipt()
        .outcome()
        .returned_outcome()
        .cloned()
        .expect("returned outcome");
    let view = block_on(ledger.observe_target(returned.into_target_receipt()))
        .expect("allocated observation");
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
    let (fixture, destination, executor) = reference_fixture(64);
    let request = committed(&fixture, 3, "operation.concurrent", "payload");
    let barrier = Arc::new(Barrier::new(12));
    let mut workers = Vec::new();
    for _ in 0..12 {
        let barrier = Arc::clone(&barrier);
        let executor = executor.clone();
        let request = request.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            block_on(executor.drive(&request))
        }));
    }
    for worker in workers {
        let result = worker.join().expect("worker").expect("drive");
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
        (
            5,
            ReferenceCrashPoint::AfterTargetMutationBeforeObservation,
            1,
        ),
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
fn exact_head_authorization_rejects_a_stale_planner_before_target_entry() {
    let fixture = binding_fixture("exact-head-authorization", false, 8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let request = committed(&fixture, 91, "operation.exact-head", "payload");
    let initial = block_on(ledger.bind_effect(request.identity())).expect("bind");
    let expected_head = initial.delivery_audit().head_ref().expect("initial head");

    let first = block_on(ledger.try_authorize_target(
        request.identity(),
        &expected_head,
        reviewed_value("exact-head.target"),
        None,
    ))
    .expect("first authorization")
    .expect("fresh planner obtains authority");
    assert_eq!(first.identity(), request.identity());

    assert!(
        block_on(ledger.try_authorize_target(
            request.identity(),
            &expected_head,
            reviewed_value("exact-head.target"),
            None,
        ))
        .expect("stale authorization decision")
        .is_none(),
        "a stale plan must not receive target-entry authority"
    );
    let current = block_on(ledger.effect_view(request.identity()))
        .expect("load effect")
        .expect("bound effect");
    assert_eq!(current.delivery_audit().attempt_count(), 1);
}

#[test]
fn terminal_tombstone_blocks_new_authority_but_late_receipt_appends() {
    let fixture = binding_fixture("late-receipt", false, 8);
    let bounds = bounds(8);
    let store = MemoryExecutorStore::new(&fixture.binding);
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate");
    let contract = reference_contract(&fixture);
    let request = committed(&fixture, 9, "operation.late", "payload");
    let identity = request.identity();
    block_on(ledger.bind_effect(identity)).expect("bind");
    let first =
        block_on(ledger.authorize_target(identity, contract.enqueue_operation().clone(), None))
            .expect("first authority");
    let second =
        block_on(ledger.authorize_target(identity, contract.enqueue_operation().clone(), None))
            .expect("second authority");
    let first = block_on(destination.enqueue(
        first,
        request.request(),
        &contract,
        ReferenceTargetBehavior::Available,
    ))
    .expect("first target");
    let second = block_on(destination.enqueue(
        second,
        request.request(),
        &contract,
        ReferenceTargetBehavior::Available,
    ))
    .expect("second target");

    let first_attempt = first.receipt().attempt_id().clone();
    let first_returned = first
        .receipt()
        .outcome()
        .returned_outcome()
        .cloned()
        .expect("returned");
    let view = block_on(ledger.observe_target(into_receipt(first))).expect("first observation");
    let observation_ref = view
        .delivery_audit()
        .frontiers()
        .iter()
        .flat_map(|frontier| frontier.appended_records())
        .find_map(|record| record.observed_content_ref().ok().flatten())
        .expect("observation ref");
    let proof =
        ReferenceTerminalProof::new(first_attempt, first_returned, observation_ref).expect("proof");
    assert_eq!(
        TerminalTombstone::new("x".repeat(257), "applied", proof.clone(),)
            .expect_err("oversized tombstone identity"),
        ExecutorError::InvalidReferenceEffectIdentifier
    );
    let tombstone = TerminalTombstone::new("operation.late", "applied", proof).expect("tombstone");
    block_on(ledger.append_terminal_tombstone(identity, tombstone)).expect("tombstone append");

    let strengthened = block_on(ledger.observe_target(into_receipt(second))).expect("late receipt");
    strengthened
        .delivery_audit()
        .verify(
            identity,
            &bounds,
            fixture.binding.deployment().evidence_authority_ref(),
        )
        .expect("late-tail audit");
    assert_eq!(
        block_on(ledger.authorize_target(identity, contract.enqueue_operation().clone(), None,))
            .expect_err("terminal authorization"),
        ExecutorError::EffectAlreadyTerminal
    );
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
    let authority = block_on(ledger.authorize_target(
        request.identity(),
        target_operation.clone(),
        Some(policy.binding()),
    ))
    .expect("target authorization");
    let attempt_id = authority.attempt_id().clone();
    drop(authority);
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
    assert_eq!(
        block_on(restored.authorize_target(&loser, reviewed_value("account.submit"), None,))
            .expect_err("missing policy pair"),
        ExecutorError::ResourcePolicyNotRevalidated
    );
    block_on(restored.authorize_target(
        &loser,
        reviewed_value("account.submit"),
        Some(&policy_binding),
    ))
    .expect("exact policy pair");
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
    ) -> ExecutorFuture<'a, mfm_executor::Result<ReferenceDestinationReturn>> {
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
fn all_570_frozen_vectors_are_consumed_by_the_shared_authority() {
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
                    | "mfm.portable-export-manifest.v1"
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

fn payload_value_ref(role: &str) -> ValidatedCanonicalValueV1 {
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

fn into_receipt(value: ReferenceDestinationReturn) -> mfm_executor::TargetOperationReceipt {
    // The destination return intentionally exposes no general raw-parts
    // constructor. This test uses the public affine result by routing it
    // through a one-shot helper method supplied for conformance.
    value.into_target_receipt()
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
