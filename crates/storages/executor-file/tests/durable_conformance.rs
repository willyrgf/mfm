use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};

use mfm_canonical::{
    sha256_digest_bytes, CanonicalValue, RecoverabilityContractV2, ValidatedCanonicalValueV2,
};
use mfm_executor::{
    verify_ensure_result, AccountSequencePolicy, AccountSequenceRequest, AllocationOutcome,
    CommittedEffectRequest, ContentRef, Ensure, EvidenceBounds, ExecutorBinding,
    ExecutorContractDescriptor, ExecutorDeployment, ExecutorEnsureResultClaim, ExecutorError,
    ExecutorFuture, ExecutorRetainedClosureClaim, ExecutorRetainedClosureContract,
    KeyedExecutorLedger, MemoryConvergentDestination, ReferenceContract, ReferenceDestination,
    ReferenceDestinationReturn, ReferenceExecutor, ReferenceRequest, ReferenceTargetBehavior,
    ReferenceTerminalProof, ResourceOwnership, ResourcePolicyBinding, RetainedValueContract,
    SchemaQualifiedCanonicalValue, TargetEntryAuthority, TerminalTombstone, TypedResourcePolicy,
    VerifiedExecutorBinding,
};
use mfm_ids::{
    DigestAlgorithm, NodeId, RunId, SemanticTypeId, StableId, StoreScopeId, TenantScopeId,
};
use mfm_storage_executor_file::{FileConvergentDestination, FileExecutorStore, FileFaultPoint};
use tempfile::TempDir;

const CORPUS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../contracts/recoverability/v2/corpus.json"
));
const WORKER_ROOT: &str = "MFM_EXECUTOR_FILE_WORKER_ROOT";
const WORKER_COUNT: usize = 8;

#[derive(Clone)]
struct Fixture {
    binding: VerifiedExecutorBinding,
    tenant_scope_id: TenantScopeId,
    generation_ref: ContentRef,
    destination_domain_ref: ContentRef,
}

fn contract() -> &'static RecoverabilityContractV2 {
    RecoverabilityContractV2::embedded().expect("contract")
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
        .expect("value");
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

fn fixture(label: &str, resource_owner: bool) -> Fixture {
    let tenant_scope_id = TenantScopeId::new(format!(
        "mfm.tenant_scope.v1:{:032x}",
        label.len() + usize::from(resource_owner) + 300
    ))
    .expect("tenant");
    let generation_ref = reviewed_ref(&format!("{label}.generation"));
    let destination_domain_ref = reviewed_ref(&format!("{label}.destination"));
    let resource_domain_ref =
        resource_owner.then(|| reviewed_ref(&format!("{label}.resource-domain")));
    let ownership = resource_domain_ref.as_ref().map(|resource_domain_ref| {
        ResourceOwnership::new(
            reviewed_ref(&format!("{label}.coordination")),
            resource_domain_ref.clone(),
            generation_ref.clone(),
            Some(reviewed_ref(&format!("{label}.destination-fence"))),
        )
        .expect("ownership")
    });
    let deployment = ExecutorDeployment::new(
        reviewed_ref(&format!("{label}.namespace")),
        generation_ref.clone(),
        tenant_scope_id.clone(),
        reviewed_ref(&format!("{label}.authority")),
        ownership
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
        reviewed_ref("file.safe-failure"),
        destination_domain_ref.clone(),
        bounds(),
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
    Fixture {
        binding: VerifiedExecutorBinding::verify(
            binding,
            executor_contract,
            deployment,
            ownership,
            &tenant_scope_id,
        )
        .expect("verified binding"),
        tenant_scope_id,
        generation_ref,
        destination_domain_ref,
    }
}

fn bounds() -> EvidenceBounds {
    EvidenceBounds::new(64, 256, 4_000_000, 2, 16_384).expect("bounds")
}

fn reference_contract(fixture: &Fixture) -> ReferenceContract {
    ReferenceContract::new(
        fixture.destination_domain_ref.clone(),
        reviewed_value("file.enqueue"),
        "applied",
        reviewed_ref("file.assurance"),
        reviewed_ref("file.safe-failure"),
    )
    .expect("reference contract")
}

fn committed(
    fixture: &Fixture,
    seed: u8,
    operation: &str,
    payload_role: &str,
) -> CommittedEffectRequest<ReferenceRequest> {
    let scope = StoreScopeId::new(format!(
        "mfm.store_scope.v1:{:032x}",
        u128::from(seed) + 500
    ))
    .expect("scope");
    let run = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[seed, 7]),
    );
    let node = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[seed, 8]),
    );
    CommittedEffectRequest::new(
        fixture.binding.binding_ref().clone(),
        fixture.tenant_scope_id.clone(),
        &scope,
        &run,
        &node,
        ReferenceRequest::new(operation, payload_value_ref(payload_role)).expect("request"),
    )
    .expect("committed")
}

fn open_executor(
    root: &Path,
    fixture: &Fixture,
) -> (
    KeyedExecutorLedger<FileExecutorStore>,
    FileConvergentDestination,
    ReferenceExecutor<FileExecutorStore, FileConvergentDestination>,
) {
    let store = block_on(FileExecutorStore::open(
        root.join("ledger"),
        fixture.binding.clone(),
    ))
    .expect("store");
    let ledger = KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("ledger");
    let destination =
        FileConvergentDestination::open(root.join("destination")).expect("destination");
    destination
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate");
    let executor = ReferenceExecutor::new(
        ledger.clone(),
        destination.clone(),
        reference_contract(fixture),
    );
    (ledger, destination, executor)
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
    ledger: &KeyedExecutorLedger<FileExecutorStore>,
    fixture: &Fixture,
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
fn restart_and_every_executor_crash_boundary_converge() {
    let temp = TempDir::new().expect("temp");
    let fixture = fixture("restart", false);
    let (_, destination, executor) = open_executor(temp.path(), &fixture);
    for (seed, point) in [
        (1, mfm_executor::ReferenceCrashPoint::BeforeTargetEntry),
        (
            2,
            mfm_executor::ReferenceCrashPoint::AfterTargetMutationBeforeObservation,
        ),
        (
            3,
            mfm_executor::ReferenceCrashPoint::AfterObservationBeforeTombstone,
        ),
        (
            4,
            mfm_executor::ReferenceCrashPoint::AfterTombstoneBeforeReturn,
        ),
    ] {
        let request = committed(&fixture, seed, &format!("file.crash.{seed}"), "payload");
        assert_eq!(
            block_on(executor.drive_conformance(
                &request,
                ReferenceTargetBehavior::Available,
                Some(point),
            ))
            .expect("fault drive"),
            mfm_executor::ReferenceDriveOutcome::Crashed(point)
        );
        let (_, reopened_destination, reopened) = open_executor(temp.path(), &fixture);
        let terminal = block_on(reopened.drive(&request)).expect("recovery");
        let Ensure::Terminal { evidence } = terminal.outcome() else {
            panic!("expected terminal");
        };
        let reverified = verify_ensure_result(
            terminal.identity().clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::terminal(evidence.claim().clone()),
            ExecutorRetainedClosureClaim::new(terminal.retained_closure().members().cloned())
                .expect("closure claim"),
        )
        .expect("callback-free retained restart verification");
        assert!(matches!(reverified.outcome(), Ensure::Terminal { .. }));
        assert_eq!(
            reopened_destination
                .semantic_mutation_count()
                .expect("mutations"),
            u64::from(seed),
            "each distinct semantic key mutates once"
        );
    }
    assert_eq!(destination.semantic_mutation_count().expect("mutations"), 4);
}

#[test]
fn publication_failures_never_return_authority_or_receipt() {
    let temp = TempDir::new().expect("temp");
    let fixture = fixture("publication", false);
    let (ledger, destination, executor) = open_executor(temp.path(), &fixture);
    let request = committed(&fixture, 5, "file.publication", "payload");
    block_on(ledger.bind_effect(request.identity())).expect("bind before fault");

    ledger
        .store()
        .inject_next_fault(FileFaultPoint::BeforeSnapshotPublish);
    assert_eq!(
        block_on(ledger.authorize_target(
            request.identity(),
            reference_contract(&fixture).enqueue_operation().clone(),
            None,
        ))
        .expect_err("pre-publish fault"),
        ExecutorError::DurableBackendUnavailable
    );
    assert_eq!(
        block_on(ledger.effect_view(request.identity()))
            .expect("view")
            .expect("effect")
            .delivery_audit()
            .attempt_count(),
        0
    );

    ledger
        .store()
        .inject_next_fault(FileFaultPoint::AfterSnapshotPublishBeforeAck);
    assert_eq!(
        block_on(ledger.authorize_target(
            request.identity(),
            reference_contract(&fixture).enqueue_operation().clone(),
            None,
        ))
        .expect_err("lost append ack"),
        ExecutorError::DurableAppendOutcomeUnknown
    );
    assert_eq!(
        block_on(ledger.effect_view(request.identity()))
            .expect("view")
            .expect("effect")
            .delivery_audit()
            .attempt_count(),
        1,
        "ambiguous append is recovered as history, never authority"
    );
    assert_eq!(destination.target_entry_count().expect("entries"), 0);

    let authority = block_on(ledger.authorize_target(
        request.identity(),
        reference_contract(&fixture).enqueue_operation().clone(),
        None,
    ))
    .expect("fresh authority");
    destination.inject_next_fault(FileFaultPoint::AfterSnapshotPublishBeforeAck);
    assert_eq!(
        block_on(destination.enqueue(
            authority,
            request.request(),
            &reference_contract(&fixture),
            ReferenceTargetBehavior::Available,
        ))
        .expect_err("lost destination ack"),
        ExecutorError::DurableBackendUnavailable
    );
    assert_eq!(
        destination
            .semantic_mutation_count()
            .expect("durable mutation"),
        1
    );
    let terminal = block_on(executor.drive(&request)).expect("retry");
    assert!(matches!(terminal.outcome(), Ensure::Terminal { .. }));
    assert_eq!(destination.semantic_mutation_count().expect("mutations"), 1);
}

#[test]
fn independent_file_handles_serialize_resource_cas() {
    let temp = TempDir::new().expect("temp");
    let fixture = fixture("resource-cas", true);
    let first_store = block_on(FileExecutorStore::open(
        temp.path().join("ledger"),
        fixture.binding.clone(),
    ))
    .expect("first store");
    let second_store = block_on(FileExecutorStore::open(
        temp.path().join("ledger"),
        fixture.binding.clone(),
    ))
    .expect("second store");
    let first =
        KeyedExecutorLedger::new(first_store, fixture.binding.clone()).expect("first ledger");
    let second =
        KeyedExecutorLedger::new(second_store, fixture.binding.clone()).expect("second ledger");
    let policy = AccountSequencePolicy::new(
        ResourcePolicyBinding::new(
            reviewed_ref("file.account-policy"),
            reviewed_ref("file.account-configuration"),
        ),
        20,
        None,
    );
    let policy_request = AccountSequenceRequest::new("sender", "chain").expect("request");
    let requests = [
        committed(&fixture, 6, "file.resource.1", "payload"),
        committed(&fixture, 7, "file.resource.2", "payload"),
    ];
    let barrier = Arc::new(Barrier::new(2));
    let handles = [first.clone(), second.clone()]
        .into_iter()
        .zip(requests.iter().cloned())
        .map(|(ledger, request)| {
            let policy = policy.clone();
            let policy_request = policy_request.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let result = block_on(ledger.try_bind_and_allocate(
                    request.identity(),
                    None,
                    &policy,
                    &policy_request,
                ));
                (request, result)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("worker"))
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
        .map(|(request, _)| request.clone())
        .expect("loser");
    let winner = results
        .iter()
        .find(|(_, result)| matches!(result, Ok(AllocationOutcome::Allocated { .. })))
        .map(|(request, _)| request.clone())
        .expect("winner");
    let resource_key = policy.resource_key(&policy_request).expect("key");
    let head = block_on(first.resource_head(&resource_key))
        .expect("head")
        .expect("head");
    assert_eq!(
        block_on(second.try_bind_and_allocate(
            loser.identity(),
            Some(&head),
            &policy,
            &policy_request,
        ))
        .expect_err("pending predecessor"),
        ExecutorError::ResourcePolicy(mfm_executor::PolicyError::PriorSequencePending)
    );
    terminalize_allocated_effect(&first, &fixture, &winner, policy.binding());
    let allocated = block_on(second.try_bind_and_allocate(
        loser.identity(),
        Some(&head),
        &policy,
        &policy_request,
    ))
    .expect("retry");
    let AllocationOutcome::Allocated { allocation, .. } = allocated else {
        panic!("expected allocation");
    };
    assert_eq!(allocation.sequence(), 21);

    let reopened_store = block_on(FileExecutorStore::open(
        temp.path().join("ledger"),
        fixture.binding.clone(),
    ))
    .expect("reopen");
    let reopened =
        KeyedExecutorLedger::new(reopened_store, fixture.binding.clone()).expect("reopened ledger");
    assert!(matches!(
        block_on(reopened.try_bind_and_allocate(loser.identity(), None, &policy, &policy_request,))
            .expect("refold existing"),
        AllocationOutcome::Existing { .. }
    ));
}

#[test]
fn greatest_corrupt_snapshot_fails_without_fallback() {
    let temp = TempDir::new().expect("temp");
    let fixture = fixture("corrupt-greatest", false);
    let (ledger, _, _) = open_executor(temp.path(), &fixture);
    let request = committed(&fixture, 8, "file.corrupt", "payload");
    block_on(ledger.bind_effect(request.identity())).expect("bind");
    let sequence = ledger.store().snapshot_sequence().expect("sequence");
    assert!(sequence > 0);
    let path = temp
        .path()
        .join("ledger")
        .join(format!("ledger-{sequence:020}.snap"));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("snapshot");
    let mut byte = [0_u8; 1];
    file.seek(SeekFrom::Start(12)).expect("seek");
    file.read_exact(&mut byte).expect("read");
    byte[0] ^= 0x40;
    file.seek(SeekFrom::Start(12)).expect("seek");
    file.write_all(&byte).expect("write");
    file.sync_all().expect("sync");

    assert_eq!(
        block_on(FileExecutorStore::open(
            temp.path().join("ledger"),
            fixture.binding,
        ))
        .expect_err("greatest snapshot is corrupt"),
        ExecutorError::InvalidDurableSnapshot
    );
}

#[cfg(unix)]
#[test]
fn symlink_and_nonregular_storage_targets_are_rejected() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().expect("temp");
    let fixture = fixture("symlink", false);
    let real = temp.path().join("real");
    fs::create_dir(&real).expect("real");
    let alias = temp.path().join("alias");
    symlink(&real, &alias).expect("symlink");
    assert_eq!(
        block_on(FileExecutorStore::open(&alias, fixture.binding.clone()))
            .expect_err("directory symlink"),
        ExecutorError::DurableBackendUnavailable
    );

    let malformed = temp.path().join("malformed");
    fs::create_dir(&malformed).expect("malformed");
    fs::create_dir(malformed.join("ledger-00000000000000000000.snap"))
        .expect("nonregular snapshot");
    assert_eq!(
        block_on(FileExecutorStore::open(&malformed, fixture.binding.clone(),))
            .expect_err("nonregular snapshot"),
        ExecutorError::InvalidDurableSnapshot
    );

    let lock_target = temp.path().join("lock-target");
    fs::write(&lock_target, b"lock").expect("lock target");
    let lock_dir = temp.path().join("lock-dir");
    fs::create_dir(&lock_dir).expect("lock dir");
    symlink(&lock_target, lock_dir.join("ledger.lock")).expect("lock symlink");
    assert_eq!(
        block_on(FileExecutorStore::open(&lock_dir, fixture.binding)).expect_err("lock symlink"),
        ExecutorError::DurableBackendUnavailable
    );
}

#[test]
fn cross_process_worker() {
    let Some(root) = std::env::var_os(WORKER_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let fixture = fixture("cross-process", false);
    let (_, _, executor) = open_executor(&root, &fixture);
    let request = committed(&fixture, 9, "file.cross-process", "payload");
    let result = block_on(executor.drive(&request)).expect("worker drive");
    assert!(matches!(result.outcome(), Ensure::Terminal { .. }));
}

#[test]
fn cross_process_drives_share_one_immutable_history() {
    if std::env::var_os(WORKER_ROOT).is_some() {
        return;
    }
    let temp = TempDir::new().expect("temp");
    let fixture = fixture("cross-process", false);
    let _ = open_executor(temp.path(), &fixture);
    let executable = std::env::current_exe().expect("test binary");
    let mut children = Vec::new();
    for _ in 0..WORKER_COUNT {
        children.push(
            Command::new(&executable)
                .arg("--exact")
                .arg("cross_process_worker")
                .arg("--nocapture")
                .env(WORKER_ROOT, temp.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn worker"),
        );
    }
    for child in children {
        let output = child.wait_with_output().expect("worker output");
        assert!(
            output.status.success(),
            "worker failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let (ledger, destination, executor) = open_executor(temp.path(), &fixture);
    let request = committed(&fixture, 9, "file.cross-process", "payload");
    let terminal = block_on(executor.drive(&request)).expect("terminal");
    assert!(matches!(terminal.outcome(), Ensure::Terminal { .. }));
    assert_eq!(destination.semantic_mutation_count().expect("mutations"), 1);
    block_on(ledger.effect_view(request.identity()))
        .expect("view")
        .expect("effect")
        .delivery_audit()
        .verify(
            request.identity(),
            &bounds(),
            fixture.binding.deployment().evidence_authority_ref(),
        )
        .expect("audit");
}

#[derive(Clone)]
struct SecretFileDestination {
    inner: FileConvergentDestination,
    credential: Arc<String>,
}

impl ReferenceDestination for SecretFileDestination {
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
fn file_snapshots_never_retain_below_boundary_credentials() {
    let temp = TempDir::new().expect("temp");
    let fixture = fixture("file-secret", false);
    let store = block_on(FileExecutorStore::open(
        temp.path().join("ledger"),
        fixture.binding.clone(),
    ))
    .expect("store");
    let ledger =
        KeyedExecutorLedger::new(store, fixture.binding.clone()).expect("keyed executor ledger");
    let inner =
        FileConvergentDestination::open(temp.path().join("destination")).expect("destination");
    inner
        .activate_generation(fixture.generation_ref.clone())
        .expect("activate");
    let secret = "file bearer credential super secret".to_owned();
    let executor = ReferenceExecutor::new(
        ledger,
        SecretFileDestination {
            inner,
            credential: Arc::new(secret.clone()),
        },
        reference_contract(&fixture),
    );
    let request = committed(&fixture, 10, "file.secret", "payload");
    block_on(executor.drive(&request)).expect("drive");

    let fingerprint = sha256_digest_bytes(secret.as_bytes()).to_string();
    for directory in ["ledger", "destination"] {
        for entry in fs::read_dir(temp.path().join(directory)).expect("directory") {
            let entry = entry.expect("entry");
            assert!(!entry.file_name().to_string_lossy().contains(&secret));
            if entry.file_type().expect("type").is_file() {
                let bytes = fs::read(entry.path()).expect("bytes");
                assert!(!contains(&bytes, secret.as_bytes()));
                assert!(!contains(&bytes, fingerprint.as_bytes()));
            }
        }
    }
}

fn payload_value_ref(role: &str) -> ValidatedCanonicalValueV2 {
    let corpus: serde_json::Value = serde_json::from_str(CORPUS).expect("corpus");
    let vector = corpus["positive_vectors"]
        .as_array()
        .expect("vectors")
        .iter()
        .find(|vector| vector["schema_contract"].as_str() == Some("mfm.value-ref.v1"))
        .expect("value ref");
    let bytes = decode_hex(vector["input_hex"].as_str().expect("hex"));
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("value");
    value["role"] = serde_json::Value::String(role.to_owned());
    let bytes = serde_json::to_vec(&value).expect("bytes");
    contract()
        .strict_decode("mfm.value-ref.v1", &bytes)
        .expect("value ref")
}

fn decode_hex(value: &str) -> Vec<u8> {
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
        _ => panic!("invalid hex"),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}
