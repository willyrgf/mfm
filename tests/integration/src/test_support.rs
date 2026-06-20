#![warn(missing_docs)]
//! Test support for MFM typed kernel contracts.
//!
//! The crate owns reusable synthetic fixtures for CI gates that need to exercise the typed
//! certified runtime without importing old dynamic authoring or execution APIs.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    ApplySideEffect, CapabilitySpec, ExternalMutationAuthorityRole, ManagedPlatformWrite,
    ManagedPlatformWriteRole, NoCaps, Pure, ReadExternal, ReadExternalRole,
};
use mfm_core::keystore::{Keystore, KeystoreConfig};
use mfm_events::v1 as events;
#[cfg(test)]
use mfm_ids::SemanticTypeId;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, NodeId, RunId, SchemaId, SpecHash,
    StateKind, StateVersion,
};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, IdempotencyKey,
    ManagedWriteState, PublicOutputKey, PureState, ReadState, ResourceClaim, RootBuilder, ScopeKey,
    SeedKey, SideEffectSagaPolicy, SideEffectState, StateKey, StateRegistryBuilder, StateResult,
    StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    CapabilityImplementationId, CertifiedRuntimeSpec, ErasedNodeRunner, ErasedRunCtx,
    ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    MaterializedCellTerminal, MaterializedInputNode, RunLaunchArtifact, RunLaunchEvidence,
    RunLaunchSeedCell, RunnerEventPayload, RuntimeArtifactStageFuture, RuntimeArtifactStager,
    RuntimeError, SchedulerStatus, SerialTypedScheduler, StagedArtifact, StagedRetentionRefs,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_store::v1::{TypedProjectionRead, TypedRunEventStore};
use serde::{Deserialize, Serialize};

/// Calls a JSON-RPC endpoint and returns the response `result`.
pub async fn rpc_call(rpc_url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .expect("send json-rpc request")
        .error_for_status()
        .expect("json-rpc http status");
    let payload: serde_json::Value = response.json().await.expect("json-rpc response json");
    if let Some(error) = payload.get("error") {
        panic!("json-rpc {method} returned error: {error}");
    }
    payload
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("json-rpc {method} response missing result: {payload}"))
}

/// Ephemeral funded keystore wallet used by reth-backed EVM parity tests.
pub struct FundedRethKeystoreWallet {
    temp_dir: tempfile::TempDir,
    keystore_env: String,
    password_file_env: String,
    /// Funded sender address derived from the keystore entry.
    pub from: String,
    /// Stable keystore entry id used by EVM signer config fixtures.
    pub entry_id: String,
    keystore_path: PathBuf,
    password_file_path: PathBuf,
}

impl FundedRethKeystoreWallet {
    /// Returns the typed workflow signer intent for this wallet.
    pub fn signer_json(&self) -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": self.from,
        })
    }

    /// Returns the process-local runtime signer registry JSON for this wallet.
    pub fn runtime_signer_registry_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "signer_ref": "deployer",
                "entry_id": self.entry_id,
                "keystore_env": self.keystore_env,
                "unlock_file_env": self.password_file_env,
            }
        ])
    }

    /// Returns the EVM source registry JSON that routes one local source to this reth node.
    pub fn runtime_source_registry_json(
        &self,
        source_id: &str,
        expected_chain_id: u64,
        endpoint_url: &str,
    ) -> serde_json::Value {
        let mut source = serde_json::json!({
            "id": source_id,
            "expected_chain_id": expected_chain_id,
        });
        let source_object = source.as_object_mut().expect("source object");
        source_object.insert(
            ["rpc", "_url"].concat(),
            serde_json::Value::String(endpoint_url.to_owned()),
        );
        source_object.insert(["author", "ization"].concat(), serde_json::Value::Null);
        serde_json::json!({
            "sources": [
                source
            ],
            "policies": [
                {
                    "id": source_id,
                    "ordered_sources": [source_id]
                }
            ]
        })
    }

    /// Returns true when `rendered` exposes process-local signer registry details.
    pub fn rendered_contains_runtime_signer_config(&self, rendered: &str) -> bool {
        rendered.contains(&self.entry_id)
            || rendered.contains(&self.keystore_env)
            || rendered.contains(&self.password_file_env)
    }

    /// Returns true when `rendered` contains test-local secret-bearing file paths.
    pub fn rendered_contains_secret_path(&self, rendered: &str) -> bool {
        rendered.contains(&self.keystore_path.display().to_string())
            || rendered.contains(&self.password_file_path.display().to_string())
    }
}

impl Drop for FundedRethKeystoreWallet {
    fn drop(&mut self) {
        std::env::remove_var(&self.keystore_env);
        std::env::remove_var(&self.password_file_env);
        let _ = self.temp_dir.path();
    }
}

/// Creates an ephemeral keystore wallet from a reth dev pre-funded account and verifies balance.
pub async fn funded_reth_keystore_wallet(
    rpc_url: &str,
    account_index: u32,
) -> FundedRethKeystoreWallet {
    const TEST_PASSWORD: &str = "reth-parity-test-password-123";
    const RETH_DEV_MNEMONIC: &str = "test test test test test test test test test test test junk";
    assert!(
        account_index < 20,
        "reth --dev prefunds 20 mnemonic accounts"
    );
    let derivation_path = format!("m/44'/60'/0'/0/{account_index}");

    let temp_dir = tempfile::tempdir().expect("reth keystore wallet tempdir");
    let keystore_path = temp_dir.path().join("reth-parity.keystore");
    let password_file_path = temp_dir.path().join("reth-parity.password");
    fs::write(&password_file_path, TEST_PASSWORD).expect("write reth parity password file");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::insecure_integration_test())
            .expect("create reth parity keystore");
    keystore
        .unlock(TEST_PASSWORD)
        .expect("unlock reth parity keystore");
    let entry_id = keystore
        .import_mnemonic(
            Some("reth-parity-funded-signer".to_owned()),
            RETH_DEV_MNEMONIC,
            &derivation_path,
            None,
        )
        .expect("import reth parity key");
    let key_info = keystore
        .list_keys()
        .expect("list reth parity keys")
        .into_iter()
        .find(|key| key.id == entry_id)
        .expect("imported reth parity key info");
    let from = format!("{:?}", key_info.address);

    let suffix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .to_ascii_uppercase();
    let keystore_env = format!("MFM_EVM_PARITY_KEYSTORE_{suffix}");
    let password_file_env = format!("MFM_EVM_PARITY_KEYSTORE_PASSWORD_FILE_{suffix}");
    std::env::set_var(&keystore_env, &keystore_path);
    std::env::set_var(&password_file_env, &password_file_path);

    // Reth dev nodes prefund this deterministic key set, but recent releases do
    // not expose the dev accounts through `eth_accounts`. The balance assertion
    // below is the funding contract these tests actually need.
    let balance = rpc_call(
        rpc_url,
        "eth_getBalance",
        serde_json::json!([from.clone(), "latest"]),
    )
    .await;
    let balance = balance
        .as_str()
        .map(parse_u128_hex_quantity)
        .expect("reth funded balance hex");
    assert!(balance > 0, "reth parity keystore wallet must be funded");

    FundedRethKeystoreWallet {
        temp_dir,
        keystore_env,
        password_file_env,
        from,
        entry_id: entry_id.to_string(),
        keystore_path,
        password_file_path,
    }
}

fn parse_u128_hex_quantity(raw: &str) -> u128 {
    let trimmed = raw
        .strip_prefix("0x")
        .expect("hex quantity must start with 0x");
    u128::from_str_radix(trimmed, 16).expect("hex quantity must parse as u128")
}

type TestRuntimeArtifactMap = BTreeMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>;

#[derive(Clone, Default)]
struct TestRuntimeArtifactStore {
    artifacts: Arc<Mutex<TestRuntimeArtifactMap>>,
}

impl RuntimeArtifactStager for TestRuntimeArtifactStore {
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a> {
        Box::pin(async move {
            let digest = ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(&bytes),
            );
            let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
            if evidence.digest != digest
                || evidence.artifact_id != artifact_id
                || evidence.byte_len != bytes.len() as u64
            {
                return Err(RuntimeError::Store(
                    "staged artifact bytes do not match evidence".to_owned(),
                ));
            }
            let mut artifacts = self.artifacts.lock().map_err(|_| {
                RuntimeError::Store("test artifact store lock was poisoned".to_owned())
            })?;
            if let Some((existing_bytes, existing_evidence)) = artifacts.get(&evidence.artifact_id)
            {
                if existing_bytes != &bytes || existing_evidence != &evidence {
                    return Err(RuntimeError::Store(format!(
                        "conflicting test artifact evidence for {}",
                        evidence.artifact_id
                    )));
                }
                return Ok(());
            }
            artifacts.insert(evidence.artifact_id.clone(), (bytes, evidence));
            Ok(())
        })
    }
}

impl store::RetainedArtifactReadProvider for TestRuntimeArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let (bytes, evidence) = self
                .artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .get(&requirement.artifact_id)
                .cloned()
                .ok_or_else(|| store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                })?;
            store::VerifiedRunArtifactBytes::new(bytes, evidence, requirement)
        })
    }
}

fn test_scheduler(
    registry: ErasedRunnerRegistry,
    artifacts: TestRuntimeArtifactStore,
) -> SerialTypedScheduler {
    SerialTypedScheduler::new(registry, Arc::new(artifacts))
}

/// Coverage produced by the synthetic typed certified slice acceptance fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedCertifiedSliceCoverage {
    typed_spec_hash_persisted: bool,
    run_admitted_v1_present: bool,
    seed_material_persisted: bool,
    cell_events_count: u64,
    side_effect_ledger_complete: bool,
    side_effect_invocation_started_before_submit: bool,
    side_effect_crash_cases_passed: bool,
    side_effect_no_duplicate_submit: bool,
    side_effect_submission_unknown_recovered: bool,
    side_effect_failed_semantics_covered: bool,
    side_effect_logical_key_conflicts_rejected: bool,
    managed_platform_outputs_committed: bool,
    public_output_before_run_completed: bool,
    public_output_event_id: String,
    replay_live_cap_requests_count: u64,
    resume_drift_fixture_count: u64,
    resume_drift_rejected: bool,
    retention_projection_complete: bool,
}

impl TypedCertifiedSliceCoverage {
    fn validate_required_contract(&self) -> Result<(), String> {
        let true_keys = [
            ("typed_spec_hash_persisted", self.typed_spec_hash_persisted),
            ("run_admitted_v1_present", self.run_admitted_v1_present),
            ("seed_material_persisted", self.seed_material_persisted),
            (
                "side_effect_ledger_complete",
                self.side_effect_ledger_complete,
            ),
            (
                "side_effect_invocation_started_before_submit",
                self.side_effect_invocation_started_before_submit,
            ),
            (
                "side_effect_crash_cases_passed",
                self.side_effect_crash_cases_passed,
            ),
            (
                "side_effect_no_duplicate_submit",
                self.side_effect_no_duplicate_submit,
            ),
            (
                "side_effect_submission_unknown_recovered",
                self.side_effect_submission_unknown_recovered,
            ),
            (
                "side_effect_failed_semantics_covered",
                self.side_effect_failed_semantics_covered,
            ),
            (
                "side_effect_logical_key_conflicts_rejected",
                self.side_effect_logical_key_conflicts_rejected,
            ),
            (
                "managed_platform_outputs_committed",
                self.managed_platform_outputs_committed,
            ),
            (
                "public_output_before_run_completed",
                self.public_output_before_run_completed,
            ),
            ("resume_drift_rejected", self.resume_drift_rejected),
            (
                "retention_projection_complete",
                self.retention_projection_complete,
            ),
        ];

        for (key, value) in true_keys {
            if !value {
                return Err(format!(
                    "typed-certified-slice required key is false: {key}"
                ));
            }
        }
        if self.cell_events_count == 0 {
            return Err("typed-certified-slice cell_events_count is zero".to_owned());
        }
        if self.replay_live_cap_requests_count != 0 {
            return Err(
                "typed-certified-slice replay live-cap request count is non-zero".to_owned(),
            );
        }
        if self.resume_drift_fixture_count == 0 {
            return Err("typed-certified-slice resume_drift_fixture_count is zero".to_owned());
        }
        if self.public_output_event_id.is_empty() {
            return Err("typed-certified-slice public_output_event_id is empty".to_owned());
        }
        Ok(())
    }
}

/// Runs the full synthetic typed certified slice and returns its coverage result.
pub async fn typed_certified_slice_coverage() -> Result<TypedCertifiedSliceCoverage, String> {
    let mut run = run_reference_certified_workflow().await?;
    let side_effect_logical_key_conflicts_rejected = duplicate_submit_rejected(&mut run)?;
    let replay_live_cap_requests_count = replay_without_live_capabilities(&run).await?;
    let resume_drift_rejected = resume_drift_is_rejected().await?;
    if !side_effect_ambiguity_degrades_without_public_output().await? {
        return Err(
            "typed-certified-slice ambiguity fixture did not degrade without public output"
                .to_owned(),
        );
    }
    let side_effect_failed_semantics_covered = side_effect_failure_semantics_are_covered().await?;
    let incomplete_retention_rejected = incomplete_retention_projection_is_rejected().await?;

    let stream = run.store.load_run_stream(&run.fixture.run_id);
    let projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(&stream).map_err(display_error)?;
    let side_effect_projection = first_forward_side_effect_projection(&projection)
        .ok_or_else(|| "missing side-effect projection".to_owned())?;
    let side_effect_ledger_complete = matches!(
        side_effect_projection.phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    );
    let public_output = projection
        .public_output(&run.fixture.public_schema)
        .ok_or_else(|| "missing public-output projection".to_owned())?;
    let public_output_event_id = match public_output {
        store::PublicOutputProjection::Produced { event_id, .. } => event_id.as_str().to_owned(),
        store::PublicOutputProjection::RenderFailed { .. } => {
            return Err("public output rendered as failure".to_owned())
        }
    };

    let summary = TypedCertifiedSliceCoverage {
        typed_spec_hash_persisted: typed_spec_hash_persisted(&run, &stream),
        run_admitted_v1_present: stream
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_))),
        seed_material_persisted: seed_material_persisted(&run, &projection),
        cell_events_count: count_payloads(&stream, |payload| {
            matches!(payload, events::KernelEventPayload::CellProduced(_))
        }),
        side_effect_ledger_complete,
        side_effect_invocation_started_before_submit: event_before(
            &stream,
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::SideEffectInvocationStarted(_)
                )
            },
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::SideEffectSubmissionObserved(_)
                )
            },
        ),
        side_effect_crash_cases_passed: event_before(
            &stream,
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::SideEffectClaimTakenOver(_)
                )
            },
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::SideEffectInvocationStarted(_)
                )
            },
        ) && event_before(
            &stream,
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::SideEffectInvocationStarted(_)
                )
            },
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::SideEffectSubmissionUnknown(_)
                )
            },
        ),
        side_effect_no_duplicate_submit: count_payloads(&stream, |payload| {
            matches!(
                payload,
                events::KernelEventPayload::SideEffectSubmissionObserved(_)
            )
        }) == 1,
        side_effect_submission_unknown_recovered: event_before(
            &stream,
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::SideEffectSubmissionUnknown(_)
                )
            },
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::SideEffectSubmissionObserved(_)
                )
            },
        ),
        side_effect_failed_semantics_covered,
        side_effect_logical_key_conflicts_rejected,
        managed_platform_outputs_committed: cell_produced_by_node(
            &stream,
            &run.fixture.managed_cell,
            &node_by_output(&run.fixture, &run.fixture.managed_cell).node_id,
        ),
        public_output_before_run_completed: event_before(
            &stream,
            |payload| matches!(payload, events::KernelEventPayload::PublicOutputProduced(_)),
            |payload| matches!(payload, events::KernelEventPayload::RunCompleted(_)),
        ),
        public_output_event_id,
        replay_live_cap_requests_count,
        resume_drift_fixture_count: 1,
        resume_drift_rejected,
        retention_projection_complete: retention_projection_complete(&run, &stream)?
            && incomplete_retention_rejected,
    };

    summary.validate_required_contract()?;
    Ok(summary)
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn typed_commit_request(
    run_id: RunId,
    expected_next_seq: store::StreamSeq,
    commit_key: store::CommitKey,
    payloads: Vec<events::KernelEventPayload>,
    required_artifacts: Vec<store::ArtifactEvidenceRef>,
    preconditions: store::CommitPreconditions,
) -> Result<store::TypedCommitRequest, String> {
    store::TypedCommitRequest::from_payloads(
        run_id,
        expected_next_seq,
        commit_key,
        payloads,
        required_artifacts,
        preconditions,
    )
    .map_err(display_error)
}

fn reference_adapter_binding() -> mfm_program::Result<AdapterBindingSpec> {
    Ok(AdapterBindingSpec {
        adapter_kind: AdapterKind::new(
            "mfm.typed_slice",
            "deterministic-local",
            DigestAlgorithm::Sha256JcsV1,
            bytes(0x2e),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        adapter_version: AdapterVersion::new("mfm.typed_slice.adapter.local.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
    })
}

#[derive(Clone)]
struct ReferenceRun {
    fixture: ReferenceFixture,
    store: store::InMemoryTypedRunStore,
    artifacts: TestRuntimeArtifactStore,
}

#[cfg(test)]
struct CompensatedReferenceRun {
    fixture: CompensatedReferenceFixture,
    store: store::InMemoryTypedRunStore,
    artifacts: TestRuntimeArtifactStore,
}

#[derive(Clone)]
struct ReferenceFixture {
    runtime_spec: CertifiedRuntimeSpec,
    run_id: RunId,
    seed_ref: events::SeedCellRef,
    seed_bytes: Vec<u8>,
    config_bytes: BTreeMap<String, Vec<u8>>,
    pure_descriptor: DescriptorId,
    read_descriptor: DescriptorId,
    managed_descriptor: DescriptorId,
    side_effect_descriptor: DescriptorId,
    read_cell: CellId,
    managed_cell: CellId,
    public_schema: SchemaId,
    read_cap_kind: CapabilityKind,
    read_cap_version: CapabilityVersion,
    managed_cap_kind: CapabilityKind,
    managed_cap_version: CapabilityVersion,
    side_effect_cap_kind: CapabilityKind,
    side_effect_cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
}

#[cfg(test)]
#[derive(Clone)]
struct CompensatedReferenceFixture {
    runtime_spec: CertifiedRuntimeSpec,
    run_id: RunId,
    seed_ref: events::SeedCellRef,
    seed_bytes: Vec<u8>,
    config_bytes: BTreeMap<String, Vec<u8>>,
    side_effect_descriptors: Vec<DescriptorId>,
    failing_descriptor: DescriptorId,
    remediation_links: BTreeMap<NodeId, NodeId>,
    side_effect_cap_kind: CapabilityKind,
    side_effect_cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.typed_slice",
    name = "value",
    version = "1",
    schema = "mfm.typed_slice.value"
)]
struct ReferenceValue {
    amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct ReferenceConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.typed_slice.public")]
struct ReferencePublicOutputs<'program, 'scope> {
    result: mfm_program::Handle<'program, 'scope, ReferenceValue>,
}

struct ReferenceReadCap;

impl CapabilitySpec for ReferenceReadCap {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.typed_slice",
            "read-fixture",
            DigestAlgorithm::Sha256JcsV1,
            bytes(0x2b),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.typed_slice.cap.read.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "read-fixture"
    }
}

struct ReferenceManagedCap;

impl CapabilitySpec for ReferenceManagedCap {
    type Role = ManagedPlatformWriteRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.typed_slice",
            "managed-output",
            DigestAlgorithm::Sha256JcsV1,
            bytes(0x2c),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.typed_slice.cap.managed_write.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "managed-output"
    }
}

struct ReferenceMutationCap;

impl CapabilitySpec for ReferenceMutationCap {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.typed_slice",
            "external-mutation",
            DigestAlgorithm::Sha256JcsV1,
            bytes(0x2d),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.typed_slice.cap.external_mutation.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "external-mutation"
    }
}

struct ReferencePureState {
    config: ReferenceConfig,
}

impl StateSpec for ReferencePureState {
    type Config = ReferenceConfig;
    type Input = ReferenceValue;
    type Output = ReferenceValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("pure", 0x3a).map_err(mfm_program::PlanError::Key)
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.typed_slice.state.pure.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.typed_slice.state.pure"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for ReferencePureState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(ReferenceValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

struct ReferenceReadState {
    config: ReferenceConfig,
}

impl StateSpec for ReferenceReadState {
    type Config = ReferenceConfig;
    type Input = ReferenceValue;
    type Output = ReferenceValue;
    type Effect = ReadExternal;
    type Caps = (ReferenceReadCap,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("read", 0x3b).map_err(mfm_program::PlanError::Key)
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.typed_slice.state.read.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.typed_slice.state.read"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        Ok(vec![reference_adapter_binding()?])
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for ReferenceReadState {
    type RunFuture<'a> = std::future::Ready<StateResult<Self::Output>>;

    fn run<'a>(&'a self, input: Self::Input, _caps: &'a Self::Caps) -> Self::RunFuture<'a> {
        std::future::ready(Ok(ReferenceValue {
            amount: input.amount + self.config.multiplier,
        }))
    }
}

struct ReferenceManagedState {
    config: ReferenceConfig,
}

impl StateSpec for ReferenceManagedState {
    type Config = ReferenceConfig;
    type Input = ReferenceValue;
    type Output = ReferenceValue;
    type Effect = ManagedPlatformWrite;
    type Caps = (ReferenceManagedCap,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("managed", 0x3c).map_err(mfm_program::PlanError::Key)
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.typed_slice.state.managed.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.typed_slice.state.managed"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ManagedWriteState for ReferenceManagedState {
    type RunFuture<'a> = std::future::Ready<StateResult<Self::Output>>;

    fn run<'a>(&'a self, input: Self::Input, _caps: &'a Self::Caps) -> Self::RunFuture<'a> {
        std::future::ready(Ok(ReferenceValue {
            amount: input.amount + self.config.multiplier,
        }))
    }
}

struct ReferenceSideEffectState {
    config: ReferenceConfig,
}

impl StateSpec for ReferenceSideEffectState {
    type Config = ReferenceConfig;
    type Input = ReferenceValue;
    type Output = ReferenceValue;
    type Effect = ApplySideEffect;
    type Caps = (ReferenceMutationCap,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("side_effect", 0x3e).map_err(mfm_program::PlanError::Key)
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.typed_slice.state.side_effect.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.typed_slice.state.side_effect"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        Ok(vec![reference_adapter_binding()?])
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl SideEffectState for ReferenceSideEffectState {
    type Intent = ReferenceValue;
    type IdempotencyInput = ReferenceValue;
    type Submission = ReferenceValue;
    type Receipt = ReferenceValue;
    type Confirmation = ReferenceValue;
    type SubmitFuture<'a> = std::future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(&self, input: &Self::Input) -> StateResult<Self::Intent> {
        Ok(ReferenceValue {
            amount: input.amount + self.config.multiplier,
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
    ) -> StateResult<Self::IdempotencyInput> {
        Ok(intent.clone())
    }

    fn submit<'a>(
        &'a self,
        intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
    ) -> Self::SubmitFuture<'a> {
        std::future::ready(Ok(intent.clone()))
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
    ) -> StateResult<Self::Output> {
        Ok(confirmation.clone())
    }
}

#[cfg(test)]
struct ReferenceFailingState {
    config: ReferenceConfig,
}

#[cfg(test)]
impl StateSpec for ReferenceFailingState {
    type Config = ReferenceConfig;
    type Input = ReferenceValue;
    type Output = ReferenceValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("tail_failure", 0x4e).map_err(mfm_program::PlanError::Key)
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.typed_slice.state.tail_failure.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.typed_slice.state.tail_failure"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

#[cfg(test)]
impl PureState for ReferenceFailingState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(ReferenceValue {
            amount: input.amount + self.config.multiplier,
        })
    }
}

async fn run_reference_certified_workflow() -> Result<ReferenceRun, String> {
    let fixture = reference_fixture()?;
    let artifacts = TestRuntimeArtifactStore::default();
    let scheduler = test_scheduler(reference_registry(&fixture)?, artifacts.clone());
    let mut store = store::InMemoryTypedRunStore::new();
    start_reference_run(&scheduler, &mut store, &fixture).await?;

    for _ in 0..32 {
        match scheduler_drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .map_err(display_error)?
        {
            SchedulerStatus::Advanced => {}
            SchedulerStatus::PublicOutputProjected => {
                return Ok(ReferenceRun {
                    fixture,
                    store,
                    artifacts,
                });
            }
            SchedulerStatus::Blocked => return Err("reference workflow blocked".to_owned()),
        }
    }

    Err("reference workflow did not reach terminal public-output projection".to_owned())
}

#[cfg(test)]
async fn run_compensated_reference_workflow() -> Result<CompensatedReferenceRun, String> {
    let fixture = compensated_reference_fixture()?;
    let artifacts = TestRuntimeArtifactStore::default();
    let scheduler = test_scheduler(compensated_reference_registry(&fixture)?, artifacts.clone());
    let mut store = store::InMemoryTypedRunStore::new();
    start_compensated_reference_run(&scheduler, &mut store, &fixture).await?;

    for _ in 0..48 {
        if compensated_forward_outputs_complete(&fixture, &store) {
            break;
        }
        match scheduler_drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .map_err(display_error)?
        {
            SchedulerStatus::Advanced => {
                if compensated_forward_outputs_complete(&fixture, &store) {
                    break;
                }
            }
            SchedulerStatus::PublicOutputProjected => {
                return Err(
                    "compensated workflow projected public output before failure".to_owned(),
                )
            }
            SchedulerStatus::Blocked => {
                if compensated_forward_outputs_complete(&fixture, &store) {
                    break;
                }
                return Err("compensated reference workflow blocked before failure".to_owned());
            }
        }
    }
    if !compensated_forward_outputs_complete(&fixture, &store) {
        return Err("compensated reference workflow did not complete forward outputs".to_owned());
    }
    append_compensated_tail_failure(&mut store, &fixture)?;

    for _ in 0..96 {
        match scheduler_drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .map_err(display_error)?
        {
            SchedulerStatus::Advanced => {}
            SchedulerStatus::PublicOutputProjected => {
                return Ok(CompensatedReferenceRun {
                    fixture,
                    store,
                    artifacts,
                });
            }
            SchedulerStatus::Blocked => {
                return Err("compensated reference workflow blocked".to_owned())
            }
        }
    }

    Err("compensated reference workflow did not reach terminal projection".to_owned())
}

fn reference_registry(fixture: &ReferenceFixture) -> Result<ErasedRunnerRegistry, String> {
    reference_registry_with_side_effect(
        fixture,
        "apply_side_effect",
        DeterministicSideEffectRunner::new(fixture),
    )
}

#[cfg(test)]
fn compensated_reference_registry(
    fixture: &CompensatedReferenceFixture,
) -> Result<ErasedRunnerRegistry, String> {
    let mut registry = ErasedRunnerRegistry::new();
    register_reference_capabilities(&mut registry, &fixture.runtime_spec)?;
    for descriptor in &fixture.side_effect_descriptors {
        registry
            .register(binding(
                descriptor.clone(),
                "apply_side_effect",
                DeterministicSideEffectRunner::from_parts(
                    fixture.side_effect_cap_kind.clone(),
                    fixture.side_effect_cap_version.clone(),
                    fixture.adapter_kind.clone(),
                    fixture.adapter_version.clone(),
                )
                .with_remediation_links(fixture.remediation_links.clone()),
            )?)
            .map_err(display_error)?;
    }
    registry
        .register(binding(
            fixture.failing_descriptor.clone(),
            "pure",
            BlockingTailRunner,
        )?)
        .map_err(display_error)?;
    Ok(registry)
}

#[cfg(test)]
fn compensated_forward_outputs_complete(
    fixture: &CompensatedReferenceFixture,
    store: &store::InMemoryTypedRunStore,
) -> bool {
    let spec = fixture.runtime_spec.spec();
    let projection = store.projection_snapshot();
    spec.nodes
        .iter()
        .filter(|node| spec.remediations.contains_key(&node.node_id))
        .all(|node| projection.cell_terminal(&node.output_cell).is_some())
}

#[cfg(test)]
fn compensated_tail_node(fixture: &CompensatedReferenceFixture) -> &spec::NodeSpec {
    fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.failing_descriptor)
        .expect("compensated tail node")
}

#[cfg(test)]
fn append_compensated_tail_failure(
    store: &mut store::InMemoryTypedRunStore,
    fixture: &CompensatedReferenceFixture,
) -> Result<(), String> {
    let node = compensated_tail_node(fixture);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )?;
    let started_request = typed_commit_request(
        fixture.run_id.clone(),
        store.expected_next_seq(&fixture.run_id),
        store::CommitKey::new(format!(
            "compensated-tail-attempt-started:{}:{}",
            node.node_id, attempt_id
        ))
        .map_err(display_error)?,
        vec![events::KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no: 1,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            },
        )],
        Vec::new(),
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    )?;
    let started_commit = store::PreparedCommit::<store::StateAttemptStarted>::new(
        started_request,
        store::CommitArtifactEvidenceSet::empty(),
    )
    .map_err(display_error)?;
    store
        .append_prepared_commit_plan(started_commit.into())
        .map_err(display_error)?;
    let failed_request = typed_commit_request(
        fixture.run_id.clone(),
        store.expected_next_seq(&fixture.run_id),
        store::CommitKey::new(format!(
            "compensated-tail-attempt-failed:{}:{}",
            node.node_id, attempt_id
        ))
        .map_err(display_error)?,
        vec![events::KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                retryable: false,
                error: side_effect_error(false),
            },
        )],
        Vec::new(),
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))
            .map_err(display_error)?],
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    )?;
    let failed_commit = store::PreparedCommit::<store::AttemptTerminal>::new(
        failed_request,
        store::CommitArtifactEvidenceSet::empty(),
    )
    .map_err(display_error)?;
    store
        .append_prepared_commit_plan(failed_commit.into())
        .map_err(display_error)?;
    Ok(())
}

fn reference_registry_with_side_effect<R: ErasedNodeRunner + 'static>(
    fixture: &ReferenceFixture,
    side_effect_factory: &str,
    side_effect_runner: R,
) -> Result<ErasedRunnerRegistry, String> {
    let mut registry = ErasedRunnerRegistry::new();
    register_reference_capabilities(&mut registry, &fixture.runtime_spec)?;
    registry
        .register(binding(
            fixture.pure_descriptor.clone(),
            "pure",
            TerminalRunner {
                expected_caps: Vec::new(),
                output_label: "pure-output",
            },
        )?)
        .map_err(display_error)?;
    registry
        .register(binding(
            fixture.read_descriptor.clone(),
            "read_external",
            ReadRunner {
                cap_kind: fixture.read_cap_kind.clone(),
                cap_version: fixture.read_cap_version.clone(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
                output_label: "read-output",
            },
        )?)
        .map_err(display_error)?;
    registry
        .register(binding(
            fixture.managed_descriptor.clone(),
            "managed_platform_write",
            TerminalRunner {
                expected_caps: vec![(
                    fixture.managed_cap_kind.clone(),
                    fixture.managed_cap_version.clone(),
                )],
                output_label: "managed-output",
            },
        )?)
        .map_err(display_error)?;
    registry
        .register(binding(
            fixture.side_effect_descriptor.clone(),
            side_effect_factory,
            side_effect_runner,
        )?)
        .map_err(display_error)?;
    Ok(registry)
}

fn register_reference_capabilities(
    registry: &mut ErasedRunnerRegistry,
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<(), String> {
    let implementation_id =
        CapabilityImplementationId::new("mfm.integration.typed-slice.runtime.v1")
            .map_err(display_error)?;
    for node in runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(runtime_spec.spec().remediations.values())
    {
        registry
            .register_capability_set(&node.capability_bindings, implementation_id.clone())
            .map_err(display_error)?;
    }
    Ok(())
}

fn binding<R: ErasedNodeRunner + 'static>(
    descriptor_id: DescriptorId,
    factory: &str,
    runner: R,
) -> Result<ErasedRunnerBinding, String> {
    let factory_id = events::RunnerFactoryId::new(factory).map_err(display_error)?;
    ErasedRunnerBinding::new(
        descriptor_id,
        factory_id.clone(),
        executable(factory)?,
        Arc::new(runner),
    )
    .map_err(display_error)
}

struct TerminalRunner {
    expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
    output_label: &'static str,
}

impl ErasedNodeRunner for TerminalRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            assert_cell_input_terminal(&ctx.inputs().root)?;
            for (kind, version) in &self.expected_caps {
                if !ctx.caps().contains(kind, version) {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "missing certified capability {kind}:{version}"
                    )));
                }
            }
            let artifact = state_output_artifact(ctx.node(), ctx.descriptor(), self.output_label);
            let staged_artifact = staged_attempt_artifact(&ctx, &artifact)?;
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: retain_artifacts([&artifact.evidence]),
                payloads: terminal_payloads(
                    &ctx,
                    artifact.evidence.artifact_id,
                    artifact.evidence.digest,
                ),
            })
        })
    }
}

#[cfg(test)]
struct BlockingTailRunner;

#[cfg(test)]
impl ErasedNodeRunner for BlockingTailRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            Err(RuntimeError::Blocked(format!(
                "tail node {} is failed by the compensated fixture",
                ctx.node().node_id
            )))
        })
    }
}

struct ReadRunner {
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    output_label: &'static str,
}

impl ErasedNodeRunner for ReadRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            assert_cell_input_terminal(&ctx.inputs().root)?;
            if !ctx.caps().contains(&self.cap_kind, &self.cap_version) {
                return Err(RuntimeError::InvalidRunnerOutput(
                    "read runner missing certified read capability".to_owned(),
                ));
            }
            let fact_key = events::FactKey::new("reference-read").map_err(RuntimeError::from)?;
            let fact_bytes = test_artifact_bytes("reference-read-fact");
            let fact_digest = digest_for_bytes(&fact_bytes);
            let fact_evidence = TestArtifact {
                bytes: fact_bytes.clone(),
                evidence: store::ArtifactEvidenceRef {
                    artifact_id: artifact_id_for_digest(&fact_digest),
                    digest: fact_digest.clone(),
                    byte_len: fact_bytes.len() as u64,
                    media_type: spec::MediaType::new("application/json")?,
                    schema_id: Some(ctx.node().config_ref.schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::FactResponse,
                },
            };
            let output = state_output_artifact(ctx.node(), ctx.descriptor(), self.output_label);
            let staged_fact = staged_attempt_artifact(&ctx, &fact_evidence)?;
            let staged_output = staged_attempt_artifact(&ctx, &output)?;
            let mut payloads = vec![RunnerEventPayload::FactRecorded(events::FactRecorded {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                capability_kind: self.cap_kind.clone(),
                capability_version: self.cap_version.clone(),
                adapter_kind: self.adapter_kind.clone(),
                adapter_version: self.adapter_version.clone(),
                request_schema_id: ctx.node().config_ref.schema_id.clone(),
                request_hash: content(0xb5),
                response_schema_id: ctx.node().config_ref.schema_id.clone(),
                response_hash: fact_digest,
                fact_key,
                artifact_id: fact_evidence.evidence.artifact_id.clone(),
            })];
            payloads.extend(terminal_payloads(
                &ctx,
                output.evidence.artifact_id.clone(),
                output.evidence.digest.clone(),
            ));
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_fact, staged_output],
                staged_retention_refs: retain_artifacts([
                    &fact_evidence.evidence,
                    &output.evidence,
                ]),
                payloads,
            })
        })
    }
}

struct DeterministicSideEffectRunner {
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    remediation_links: BTreeMap<NodeId, NodeId>,
    output_label: &'static str,
}

impl DeterministicSideEffectRunner {
    fn new(fixture: &ReferenceFixture) -> Self {
        Self::from_parts(
            fixture.side_effect_cap_kind.clone(),
            fixture.side_effect_cap_version.clone(),
            fixture.adapter_kind.clone(),
            fixture.adapter_version.clone(),
        )
    }

    fn from_parts(
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    ) -> Self {
        Self {
            cap_kind,
            cap_version,
            adapter_kind,
            adapter_version,
            remediation_links: BTreeMap::new(),
            output_label: "side-effect-output",
        }
    }

    #[cfg(test)]
    fn with_remediation_links(mut self, remediation_links: BTreeMap<NodeId, NodeId>) -> Self {
        self.remediation_links = remediation_links;
        self
    }
}

fn reference_fixture() -> Result<ReferenceFixture, String> {
    let seed = CanonicalSeed::from_value(&ReferenceValue { amount: 1 }).map_err(display_error)?;
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ReferencePureState>()
        .map_err(display_error)?;
    states
        .register::<ReferenceReadState>()
        .map_err(display_error)?;
    states
        .register::<ReferenceManagedState>()
        .map_err(display_error)?;
    states
        .register::<ReferenceSideEffectState>()
        .map_err(display_error)?;
    let draft = build_root_with_registries(
        ScopeKey::new("root").map_err(display_error)?,
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let seed = root.seed(SeedKey::new("launch")?, seed.clone())?;
            let pure = root.scope().state::<ReferencePureState, _>(
                StateKey::new("pure")?,
                ReferenceConfig { multiplier: 2 },
                seed,
            )?;
            let read = root.scope().state::<ReferenceReadState, _>(
                StateKey::new("read")?,
                ReferenceConfig { multiplier: 3 },
                pure,
            )?;
            let managed = root.scope().state::<ReferenceManagedState, _>(
                StateKey::new("managed")?,
                ReferenceConfig { multiplier: 4 },
                read,
            )?;
            let side_effect = root.scope().side_effect::<ReferenceSideEffectState, _>(
                StateKey::new("side-effect")?,
                ReferenceConfig { multiplier: 5 },
                managed,
                ResourceClaim::manual_only(),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("public-output")?,
                &ReferencePublicOutputs {
                    result: side_effect.into_handle(),
                },
            )
        },
    )
    .map_err(display_error)?;
    let certified = mfm_certify::certify_program_draft(&draft).map_err(display_error)?;
    let runtime_spec = CertifiedRuntimeSpec::new(certified).map_err(display_error)?;
    let spec = runtime_spec.spec();
    let config_bytes = config_bytes_for_draft_and_spec(&draft, spec)?;
    let seed_bytes = seed.canonical_json().to_vec();
    let node_by_key = |key: &str| -> Result<&spec::NodeSpec, String> {
        spec.nodes
            .iter()
            .find(|node| node.stable_key.as_str() == key)
            .ok_or_else(|| format!("missing node with key {key}"))
    };
    let pure_node = node_by_key("pure")?;
    let read_node = node_by_key("read")?;
    let managed_node = node_by_key("managed")?;
    let side_effect_node = node_by_key("side-effect")?;
    let seed_spec = spec
        .seeds
        .first()
        .ok_or_else(|| "missing reference seed".to_owned())?;
    let seed_digest = seed_spec
        .required_digest
        .clone()
        .ok_or_else(|| "reference seed must require a digest".to_owned())?;
    let seed_ref = events::SeedCellRef {
        seed_id: seed_spec.seed_id.clone(),
        cell_id: seed_spec.cell_id.clone(),
        scope_id: seed_spec.scope_id.clone(),
        semantic_type_id: seed_spec.semantic_type_id.clone(),
        schema_id: seed_spec.schema_id.clone(),
        digest: seed_digest.clone(),
        seed_artifact: events::ArtifactEvidenceRef {
            artifact_id: artifact_id_for_digest(&seed_digest),
            role: events::ArtifactRole::SeedInput,
            schema_id: seed_spec.schema_id.clone(),
            semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
            content_digest: seed_digest,
            byte_len: seed.canonical_json().as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json").map_err(display_error)?,
        },
    };
    let first_cap = |node: &spec::NodeSpec| -> Result<(CapabilityKind, CapabilityVersion), String> {
        let capability = node
            .capability_bindings
            .capabilities
            .first()
            .ok_or_else(|| format!("node {} has no capability", node.node_id))?;
        Ok((capability.kind.clone(), capability.version.clone()))
    };
    let (read_cap_kind, read_cap_version) = first_cap(read_node)?;
    let (managed_cap_kind, managed_cap_version) = first_cap(managed_node)?;
    let (side_effect_cap_kind, side_effect_cap_version) = first_cap(side_effect_node)?;
    let adapter = read_node
        .adapter_bindings
        .first()
        .ok_or_else(|| "reference read node missing adapter binding".to_owned())?;
    let pure_descriptor = pure_node.descriptor_id.clone();
    let read_descriptor = read_node.descriptor_id.clone();
    let managed_descriptor = managed_node.descriptor_id.clone();
    let side_effect_descriptor = side_effect_node.descriptor_id.clone();
    let read_cell = read_node.output_cell.clone();
    let managed_cell = managed_node.output_cell.clone();
    let public_schema = spec.public_outputs.public_schema_id.clone();
    let adapter_kind = adapter.adapter_kind.clone();
    let adapter_version = adapter.adapter_version.clone();
    Ok(ReferenceFixture {
        runtime_spec,
        run_id: run_id(0x42),
        seed_ref,
        seed_bytes,
        config_bytes,
        pure_descriptor,
        read_descriptor,
        managed_descriptor,
        side_effect_descriptor,
        read_cell,
        managed_cell,
        public_schema,
        read_cap_kind,
        read_cap_version,
        managed_cap_kind,
        managed_cap_version,
        side_effect_cap_kind,
        side_effect_cap_version,
        adapter_kind,
        adapter_version,
    })
}

#[cfg(test)]
fn compensated_reference_fixture() -> Result<CompensatedReferenceFixture, String> {
    let seed = CanonicalSeed::from_value(&ReferenceValue { amount: 1 }).map_err(display_error)?;
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ReferenceSideEffectState>()
        .map_err(display_error)?;
    states
        .register::<ReferenceFailingState>()
        .map_err(display_error)?;
    let draft = build_root_with_registries(
        ScopeKey::new("root").map_err(display_error)?,
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SideEffectSagaPolicy::CompensateCompleted {
                on_remediation_unresolved: mfm_program::RemediationUnresolved::FailWithoutAcdcClaim,
            })?;
            let seed = root.seed(SeedKey::new("launch")?, seed.clone())?;
            let (forward_a, _) = root.scope().side_effect_with_compensation::<
                ReferenceSideEffectState,
                ReferenceSideEffectState,
                _,
                _,
                _,
            >(
                mfm_program::SideEffectNodeParams {
                    key: StateKey::new("forward-a")?,
                    config: ReferenceConfig { multiplier: 2 },
                    input: seed,
                    resource_claim: ResourceClaim::manual_only(),
                },
                mfm_program::RemediationNodeParams {
                    key: StateKey::new("remediate-a")?,
                    config: ReferenceConfig { multiplier: 11 },
                    resource_claim: ResourceClaim::manual_only(),
                },
                |forward| Ok(forward.clone()),
            )?;
            let (forward_b, _) = root.scope().side_effect_with_compensation::<
                ReferenceSideEffectState,
                ReferenceSideEffectState,
                _,
                _,
                _,
            >(
                mfm_program::SideEffectNodeParams {
                    key: StateKey::new("forward-b")?,
                    config: ReferenceConfig { multiplier: 3 },
                    input: forward_a,
                    resource_claim: ResourceClaim::manual_only(),
                },
                mfm_program::RemediationNodeParams {
                    key: StateKey::new("remediate-b")?,
                    config: ReferenceConfig { multiplier: 13 },
                    resource_claim: ResourceClaim::manual_only(),
                },
                |forward| Ok(forward.clone()),
            )?;
            let tail = root.scope().state::<ReferenceFailingState, _>(
                StateKey::new("tail-failure")?,
                ReferenceConfig { multiplier: 5 },
                forward_b.into_handle(),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("public-output")?,
                &ReferencePublicOutputs { result: tail },
            )
        },
    )
    .map_err(display_error)?;
    let certified = mfm_certify::certify_program_draft(&draft).map_err(display_error)?;
    let runtime_spec = CertifiedRuntimeSpec::new(certified).map_err(display_error)?;
    let spec = runtime_spec.spec();
    let config_bytes = config_bytes_for_draft_and_spec(&draft, spec)?;
    let seed_bytes = seed.canonical_json().to_vec();
    let side_effect_node = spec
        .nodes
        .iter()
        .find(|node| node.stable_key.as_str() == "forward-a")
        .ok_or_else(|| "missing forward-a node".to_owned())?;
    let failing_node = spec
        .nodes
        .iter()
        .find(|node| node.stable_key.as_str() == "tail-failure")
        .ok_or_else(|| "missing tail-failure node".to_owned())?;
    let seed_spec = spec
        .seeds
        .first()
        .ok_or_else(|| "missing compensated reference seed".to_owned())?;
    let seed_digest = seed_spec
        .required_digest
        .clone()
        .ok_or_else(|| "compensated reference seed must require a digest".to_owned())?;
    let seed_ref = events::SeedCellRef {
        seed_id: seed_spec.seed_id.clone(),
        cell_id: seed_spec.cell_id.clone(),
        scope_id: seed_spec.scope_id.clone(),
        semantic_type_id: seed_spec.semantic_type_id.clone(),
        schema_id: seed_spec.schema_id.clone(),
        digest: seed_digest.clone(),
        seed_artifact: events::ArtifactEvidenceRef {
            artifact_id: artifact_id_for_digest(&seed_digest),
            role: events::ArtifactRole::SeedInput,
            schema_id: seed_spec.schema_id.clone(),
            semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
            content_digest: seed_digest,
            byte_len: seed.canonical_json().as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json").map_err(display_error)?,
        },
    };
    let capability = side_effect_node
        .capability_bindings
        .capabilities
        .first()
        .ok_or_else(|| "compensated side-effect node has no capability".to_owned())?;
    let adapter = side_effect_node
        .adapter_bindings
        .first()
        .ok_or_else(|| "compensated side-effect node missing adapter binding".to_owned())?;
    let mut side_effect_descriptors = spec
        .nodes
        .iter()
        .chain(spec.remediations.values())
        .filter(|node| node.side_effect.is_some())
        .map(|node| node.descriptor_id.clone())
        .collect::<Vec<_>>();
    side_effect_descriptors.sort();
    side_effect_descriptors.dedup();
    let failing_descriptor = failing_node.descriptor_id.clone();
    let side_effect_cap_kind = capability.kind.clone();
    let side_effect_cap_version = capability.version.clone();
    let adapter_kind = adapter.adapter_kind.clone();
    let adapter_version = adapter.adapter_version.clone();
    let remediation_links = spec
        .remediations
        .iter()
        .map(|(forward_node_id, remediation)| {
            (remediation.node_id.clone(), forward_node_id.clone())
        })
        .collect();
    Ok(CompensatedReferenceFixture {
        runtime_spec,
        run_id: run_id(0x58),
        seed_ref,
        seed_bytes,
        config_bytes,
        side_effect_descriptors,
        failing_descriptor,
        remediation_links,
        side_effect_cap_kind,
        side_effect_cap_version,
        adapter_kind,
        adapter_version,
    })
}

fn typed_spec_hash_persisted(run: &ReferenceRun, stream: &[store::KernelEventEnvelope]) -> bool {
    stream.iter().any(|event| match event.payload() {
        events::KernelEventPayload::RunAdmitted(payload) => {
            let spec_hash = SpecHash::from_digest(
                payload.spec_artifact.content_digest.algorithm(),
                *payload.spec_artifact.content_digest.digest(),
            );
            payload.spec_hash == *run.fixture.runtime_spec.spec_hash()
                && payload.spec_hash == spec_hash
                && payload.spec_artifact.role == events::ArtifactRole::TypedExecutionSpec
        }
        _ => false,
    })
}

fn seed_material_persisted(run: &ReferenceRun, projection: &store::ProjectionSnapshot) -> bool {
    let Some(retention) = projection.retention(&run.fixture.run_id) else {
        return false;
    };
    retention.refs.values().any(|retention_ref| {
        retention_ref.artifact_id == run.fixture.seed_ref.seed_artifact.artifact_id
            && retention_ref.content_digest == run.fixture.seed_ref.seed_artifact.content_digest
            && retention_ref.role == events::ArtifactRole::SeedInput
    })
}

fn count_payloads(
    stream: &[store::KernelEventEnvelope],
    matches_payload: impl Fn(&events::KernelEventPayload) -> bool,
) -> u64 {
    stream
        .iter()
        .filter(|event| matches_payload(event.payload()))
        .count() as u64
}

fn event_before(
    stream: &[store::KernelEventEnvelope],
    first: impl Fn(&events::KernelEventPayload) -> bool,
    second: impl Fn(&events::KernelEventPayload) -> bool,
) -> bool {
    let first_position = stream.iter().position(|event| first(event.payload()));
    let second_position = stream.iter().position(|event| second(event.payload()));
    matches!((first_position, second_position), (Some(a), Some(b)) if a < b)
}

fn cell_produced_by_node(
    stream: &[store::KernelEventEnvelope],
    cell_id: &CellId,
    node_id: &NodeId,
) -> bool {
    stream.iter().any(|event| match event.payload() {
        events::KernelEventPayload::CellProduced(payload) => {
            &payload.cell_id == cell_id && &payload.node_id == node_id
        }
        _ => false,
    })
}

fn retention_projection_complete(
    run: &ReferenceRun,
    stream: &[store::KernelEventEnvelope],
) -> Result<bool, String> {
    let verified = store::VerifiedRetentionProjectionSet::from_synthetic_run_streams([(
        run.fixture.run_id.clone(),
        stream,
    )])
    .map_err(display_error)?;
    let Some((_, projection)) = verified.projections().next() else {
        return Ok(false);
    };
    Ok(projection.manifest.is_some()
        && !projection.manifests.is_empty()
        && !projection.refs.is_empty())
}

async fn incomplete_retention_projection_is_rejected() -> Result<bool, String> {
    let fixture = reference_fixture()?;
    let artifacts = TestRuntimeArtifactStore::default();
    let scheduler = test_scheduler(reference_registry(&fixture)?, artifacts.clone());
    let mut store = store::InMemoryTypedRunStore::new();
    start_reference_run(&scheduler, &mut store, &fixture).await?;
    let stream = store.load_run_stream(&fixture.run_id);
    Ok(
        store::VerifiedRetentionProjectionSet::from_synthetic_run_streams(std::iter::empty::<(
            RunId,
            &[store::KernelEventEnvelope],
        )>())
        .is_err()
            && !retention_projection_complete(
                &ReferenceRun {
                    fixture,
                    store,
                    artifacts,
                },
                &stream,
            )?,
    )
}

fn duplicate_submit_rejected(run: &mut ReferenceRun) -> Result<bool, String> {
    let stream = run.store.load_run_stream(&run.fixture.run_id);
    let Some(submission) = stream.iter().find_map(|event| match event.payload() {
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => Some(payload.clone()),
        _ => None,
    }) else {
        return Err("missing submission event for duplicate-submit fixture".to_owned());
    };
    let duplicate_artifact = artifact(0xee);
    let duplicate_digest = content(0xef);
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: duplicate_artifact.clone(),
        digest: duplicate_digest.clone(),
        byte_len: 29,
        media_type: spec::MediaType::new("application/json").map_err(display_error)?,
        schema_id: Some(submission.submission_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(submission.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::Submission,
    };
    let request = typed_commit_request(
        run.fixture.run_id.clone(),
        run.store.expected_next_seq(&run.fixture.run_id),
        store::CommitKey::new("duplicate-submit-conflict").map_err(display_error)?,
        vec![events::KernelEventPayload::SideEffectSubmissionObserved(
            events::side_effect::SubmissionObserved {
                spec_hash: run.fixture.runtime_spec.spec_hash().clone(),
                node_id: submission.node_id,
                attempt_id: submission.attempt_id,
                ledger_key: submission.ledger_key,
                ledger_purpose: submission.ledger_purpose,
                invocation_epoch: submission.invocation_epoch,
                submission_schema_id: submission.submission_schema_id,
                submission_hash: duplicate_digest,
                submission_artifact_id: duplicate_artifact,
            },
        )],
        vec![evidence.clone()],
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::Any,
            ..store::CommitPreconditions::default()
        },
    )?;
    let artifacts = store::CommitArtifactEvidenceSet::new(
        request.required_artifacts().to_vec(),
        vec![evidence],
    )
    .map_err(display_error)?;
    let result = store::PreparedCommit::<store::SideEffectTerminal>::new(request, artifacts)
        .and_then(|commit| run.store.append_prepared_commit_plan(commit.into()));
    Ok(matches!(
        result,
        Err(store::StoreError::LogicalKeyConflict { .. })
            | Err(store::StoreError::DuplicateLogicalKey { .. })
    ))
}

async fn replay_without_live_capabilities(run: &ReferenceRun) -> Result<u64, String> {
    let stream = run.store.load_run_stream(&run.fixture.run_id);
    replay_without_live_capabilities_for(
        &run.fixture.runtime_spec,
        &run.store,
        &run.artifacts,
        &run.fixture.run_id,
    )
    .await
    .map(|()| 0)
    .map_err(|error| {
        if stream.is_empty() {
            "missing run stream for replay fixture".to_owned()
        } else {
            error
        }
    })
}

async fn replay_without_live_capabilities_for(
    runtime_spec: &CertifiedRuntimeSpec,
    store: &store::InMemoryTypedRunStore,
    artifacts: &TestRuntimeArtifactStore,
    run_id: &RunId,
) -> Result<(), String> {
    let stream = store.load_run_stream(run_id);
    let run_admitted = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(payload.clone()),
            _ => None,
        })
        .ok_or_else(|| "missing RunAdmitted for replay fixture".to_owned())?;
    let committed = store::CommittedRunStream::from_events(run_admitted.run_id.clone(), stream)
        .map_err(display_error)?;
    let retained_artifacts =
        store::VerifiedRunArtifactStore::from_committed_stream(&committed, artifacts)
            .await
            .map_err(display_error)?;
    let verified_history = mfm_runtime::VerifiedRunHistory::from_committed_stream(
        runtime_spec,
        committed,
        retained_artifacts,
    )
    .map_err(display_error)?;
    let authority =
        replay::ReplayReadAuthority::from_verified_run_history(runtime_spec, &verified_history)
            .map_err(display_error)?;
    let broker = replay::ReplayBroker::from_read_authority(authority).map_err(display_error)?;
    let live_cap_rejected = matches!(
        broker.reject_live_capability_request(),
        Err(replay::ReplayError {
            kind: replay::ReplayErrorKind::LiveCapabilityRequest,
            ..
        })
    );
    if live_cap_rejected {
        Ok(())
    } else {
        Err("replay broker did not reject a live capability request".to_owned())
    }
}

async fn resume_drift_is_rejected() -> Result<bool, String> {
    let fixture = reference_fixture()?;
    let scheduler = test_scheduler(
        reference_registry(&fixture)?,
        TestRuntimeArtifactStore::default(),
    );
    let mut store = store::InMemoryTypedRunStore::new();
    start_reference_run(&scheduler, &mut store, &fixture).await?;
    let read_node = node_by_output(&fixture, &fixture.read_cell).clone();
    append_attempt_started(&mut store, &fixture, &read_node, 1)?;
    Ok(matches!(
        scheduler_drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await,
        Err(RuntimeError::InvalidRunStream(_))
    ))
}

async fn side_effect_failure_semantics_are_covered() -> Result<bool, String> {
    let fixture = reference_fixture()?;
    let registry = reference_registry_with_side_effect(
        &fixture,
        "apply_side_effect",
        FailingSideEffectRunner::new(&fixture),
    )?;
    let scheduler = test_scheduler(registry, TestRuntimeArtifactStore::default());
    let mut store = store::InMemoryTypedRunStore::new();
    start_reference_run(&scheduler, &mut store, &fixture).await?;
    for _ in 0..4 {
        scheduler_drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .map_err(display_error)?;
    }
    let projection = store.projection_snapshot();
    let side_effect = first_forward_side_effect_projection(projection)
        .ok_or_else(|| "missing failed side-effect projection".to_owned())?;
    Ok(matches!(
        side_effect.phase,
        store::SideEffectPhase::Failed { .. }
    ))
}

async fn side_effect_ambiguity_degrades_without_public_output() -> Result<bool, String> {
    let fixture = reference_fixture()?;
    let registry = reference_registry_with_side_effect(
        &fixture,
        "apply_side_effect",
        AmbiguousSideEffectRunner::new(&fixture),
    )?;
    let scheduler = test_scheduler(registry, TestRuntimeArtifactStore::default());
    let mut store = store::InMemoryTypedRunStore::new();
    start_reference_run(&scheduler, &mut store, &fixture).await?;

    for _ in 0..16 {
        match scheduler_drive_once(
            &scheduler,
            &mut store,
            &fixture.runtime_spec,
            &fixture.run_id,
        )
        .await
        .map_err(display_error)?
        {
            SchedulerStatus::Advanced => {}
            SchedulerStatus::PublicOutputProjected => {
                let stream = store.load_run_stream(&fixture.run_id);
                let ambiguous = stream.iter().any(|event| {
                    matches!(
                        event.payload(),
                        events::KernelEventPayload::SideEffectAmbiguous(_)
                    )
                });
                let failed_without_claim = stream.iter().any(|event| {
                    matches!(
                        event.payload(),
                        events::KernelEventPayload::RunCompleted(events::RunCompleted {
                            outcome: events::RunCompletionOutcome::FailedWithoutAcdcClaim,
                            ..
                        })
                    )
                });
                let public_output = stream.iter().any(|event| {
                    matches!(
                        event.payload(),
                        events::KernelEventPayload::PublicOutputProduced(_)
                    )
                });
                return Ok(ambiguous && failed_without_claim && !public_output);
            }
            SchedulerStatus::Blocked => return Ok(false),
        }
    }

    Err("ambiguous side-effect fixture did not reach a terminal scheduler state".to_owned())
}

struct FailingSideEffectRunner {
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
}

impl FailingSideEffectRunner {
    fn new(fixture: &ReferenceFixture) -> Self {
        Self {
            cap_kind: fixture.side_effect_cap_kind.clone(),
            cap_version: fixture.side_effect_cap_version.clone(),
            adapter_kind: fixture.adapter_kind.clone(),
            adapter_version: fixture.adapter_version.clone(),
        }
    }
}

impl ErasedNodeRunner for FailingSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let ledger = side_effect_forward_ledger_key_for_ctx(&ctx);
            let artifact = side_effect_artifact(
                &ctx,
                "failing-intent",
                events::ArtifactRole::SideEffectIntent,
            );
            let staged_artifact = staged_side_effect_artifact(&ctx, &artifact, ledger.clone(), 1)?;
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: retain_artifacts([&artifact.evidence]),
                payloads: vec![
                    RunnerEventPayload::SideEffectIntentPersisted(
                        events::side_effect::IntentPersisted {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            scope_id: ctx.node().scope_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            ledger_key: ledger.clone(),
                            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                            invocation_epoch: 1,
                            intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                            intent_hash: artifact.evidence.digest.clone(),
                            intent_artifact_id: artifact.evidence.artifact_id.clone(),
                            idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
                            idempotency_input_hash: content(0xf3),
                            idempotency_key: events::IdempotencyKeyRef::new(format!(
                                "failing-idem-{}-{}",
                                ctx.node().node_id,
                                ctx.attempt_no()
                            ))
                            .map_err(RuntimeError::from)?,
                            capability_kind: self.cap_kind.clone(),
                            capability_version: self.cap_version.clone(),
                            adapter_kind: self.adapter_kind.clone(),
                            adapter_version: self.adapter_version.clone(),
                        },
                    ),
                    RunnerEventPayload::SideEffectFailed(events::side_effect::Failed {
                        spec_hash: ctx.spec_hash().clone(),
                        node_id: ctx.node().node_id.clone(),
                        attempt_id: ctx.attempt_id().clone(),
                        ledger_key: ledger,
                        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                        invocation_epoch: 1,
                        failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                        retryable: false,
                        error: side_effect_error(false),
                    }),
                ],
            })
        })
    }
}

struct AmbiguousSideEffectRunner {
    inner: DeterministicSideEffectRunner,
}

impl AmbiguousSideEffectRunner {
    fn new(fixture: &ReferenceFixture) -> Self {
        Self {
            inner: DeterministicSideEffectRunner::new(fixture),
        }
    }
}

impl ErasedNodeRunner for AmbiguousSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let ledger = side_effect_ledger_key_for_ctx(&ctx, &self.inner.remediation_links);
            let phase = ctx.projections().side_effect(&ledger);
            if matches!(
                phase.map(|projection| &projection.phase),
                Some(store::SideEffectPhase::InvocationStarted { .. })
            ) {
                let artifact = side_effect_artifact(
                    &ctx,
                    "ambiguous-evidence",
                    events::ArtifactRole::AmbiguityEvidence,
                );
                let staged_artifact =
                    staged_side_effect_artifact(&ctx, &artifact, ledger.clone(), 1)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: retain_artifacts([&artifact.evidence]),
                    payloads: vec![RunnerEventPayload::SideEffectAmbiguous(
                        events::side_effect::Ambiguous {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            ledger_key: ledger,
                            ledger_purpose: side_effect_ledger_purpose_for_ctx(
                                &ctx,
                                &self.inner.remediation_links,
                            ),
                            invocation_epoch: 1,
                            ambiguity_code: events::AmbiguityCode::new("unknown_submission")
                                .map_err(RuntimeError::from)?,
                            evidence_schema_id: ctx.node().config_ref.schema_id.clone(),
                            evidence_hash: artifact.evidence.digest,
                            evidence_artifact_id: artifact.evidence.artifact_id,
                        },
                    )],
                })
            } else {
                self.inner.run_erased(ctx).await
            }
        })
    }
}

#[cfg(test)]
fn replay_artifacts(
    fixture: &ReferenceFixture,
    stream: &[store::KernelEventEnvelope],
) -> Result<Vec<store::ArtifactEvidenceRef>, String> {
    replay_artifacts_for(&fixture.runtime_spec, stream)
}

#[cfg(test)]
fn replay_artifacts_for(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<Vec<store::ArtifactEvidenceRef>, String> {
    let mut artifacts = BTreeMap::<ArtifactId, store::ArtifactEvidenceRef>::new();
    insert_artifact(&mut artifacts, spec_artifact(runtime_spec)?.evidence);
    insert_artifact(&mut artifacts, certificate_artifact(runtime_spec)?.evidence);
    for config in &runtime_spec.spec().config_refs {
        insert_artifact(&mut artifacts, config_artifact_evidence(config));
    }
    for artifact in referenced_artifacts_from_stream(runtime_spec, stream)? {
        insert_artifact(&mut artifacts, artifact);
    }
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => {
                for seed in &payload.seed_cells {
                    insert_artifact(&mut artifacts, seed_artifact(seed));
                }
            }
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.intent_artifact_id.clone(),
                        payload.intent_hash.clone(),
                        Some(payload.intent_schema_id.clone()),
                        None,
                        Some(payload.node_id.clone()),
                        events::ArtifactRole::SideEffectIntent,
                    )?,
                );
            }
            events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.evidence_artifact_id.clone(),
                        payload.evidence_hash.clone(),
                        Some(payload.evidence_schema_id.clone()),
                        None,
                        Some(payload.node_id.clone()),
                        events::ArtifactRole::SubmissionUnknownEvidence,
                    )?,
                );
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.submission_artifact_id.clone(),
                        payload.submission_hash.clone(),
                        Some(payload.submission_schema_id.clone()),
                        None,
                        Some(payload.node_id.clone()),
                        events::ArtifactRole::Submission,
                    )?,
                );
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.receipt_artifact_id.clone(),
                        payload.receipt_hash.clone(),
                        Some(payload.receipt_schema_id.clone()),
                        None,
                        Some(payload.node_id.clone()),
                        events::ArtifactRole::Receipt,
                    )?,
                );
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.confirmation_artifact_id.clone(),
                        payload.confirmation_hash.clone(),
                        Some(payload.confirmation_schema_id.clone()),
                        None,
                        Some(payload.node_id.clone()),
                        events::ArtifactRole::Confirmation,
                    )?,
                );
            }
            events::KernelEventPayload::RetentionManifestProjected(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.manifest_artifact_id.clone(),
                        payload.manifest_digest.clone(),
                        None,
                        None,
                        None,
                        events::ArtifactRole::RetentionManifest,
                    )?,
                );
            }
            events::KernelEventPayload::ManualResolutionRecorded(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.evidence_artifact_id.clone(),
                        payload.evidence_hash.clone(),
                        Some(payload.evidence_schema_id.clone()),
                        None,
                        None,
                        events::ArtifactRole::ManualResolutionEvidence,
                    )?,
                );
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.authorization_artifact_id.clone(),
                        payload.authorization_hash.clone(),
                        Some(payload.authorization_schema_id.clone()),
                        None,
                        None,
                        events::ArtifactRole::ManualResolutionAuthorization,
                    )?,
                );
            }
            events::KernelEventPayload::ArtifactReferenced(_)
            | events::KernelEventPayload::FactRecorded(_)
            | events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::StateAttemptStarted(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_)
            | events::KernelEventPayload::PublicOutputProduced(_)
            | events::KernelEventPayload::PublicOutputRenderFailed(_)
            | events::KernelEventPayload::StateAttemptCompleted(_)
            | events::KernelEventPayload::StateAttemptFailed(_)
            | events::KernelEventPayload::StateAttemptInterrupted(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_) => {}
        }
    }
    Ok(artifacts.into_values().collect())
}

#[cfg(test)]
fn referenced_artifacts_from_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<Vec<store::ArtifactEvidenceRef>, String> {
    let (start_seq, start_commit_key) = stream
        .iter()
        .find(|event| matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_)))
        .map(|event| (event.seq(), event.commit_key().clone()))
        .ok_or_else(|| "missing RunAdmitted event".to_owned())?;
    let mut artifacts = Vec::new();
    let mut config_artifacts = Vec::new();
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        for event in commit {
            let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
                continue;
            };
            match payload.artifact_ref.role {
                events::ArtifactRole::TypedConfig => {
                    if payload.node_id.is_some() || payload.attempt_id.is_some() {
                        return Err(
                            "typed config artifact reference was scoped to an attempt".to_owned()
                        );
                    }
                    if event.seq() != start_seq || event.commit_key() != &start_commit_key {
                        return Err(
                            "typed config artifact reference was outside RunAdmitted commit"
                                .to_owned(),
                        );
                    }
                    config_artifacts.push(referenced_artifact(payload));
                }
                events::ArtifactRole::StateOutput
                | events::ArtifactRole::FactResponse
                | events::ArtifactRole::PublicOutput
                | events::ArtifactRole::RedactedDiagnostic
                | events::ArtifactRole::ManualResolutionEvidence
                | events::ArtifactRole::ManualResolutionAuthorization => {
                    if !reference_matches_same_commit_payload(commit, payload) {
                        return Err(format!(
                            "artifact reference {} was not bound to a same-commit typed payload",
                            payload.artifact_ref.artifact_id
                        ));
                    }
                    artifacts.push(referenced_artifact(payload));
                }
                events::ArtifactRole::TypedExecutionSpec
                | events::ArtifactRole::TypedSpecCertificate
                | events::ArtifactRole::SeedInput
                | events::ArtifactRole::SideEffectIntent
                | events::ArtifactRole::PreparedInvocation
                | events::ArtifactRole::NotSubmittedProof
                | events::ArtifactRole::Submission
                | events::ArtifactRole::SubmissionUnknownEvidence
                | events::ArtifactRole::Receipt
                | events::ArtifactRole::Confirmation
                | events::ArtifactRole::AmbiguityEvidence
                | events::ArtifactRole::RetentionManifest => {
                    return Err(format!(
                        "unsupported artifact reference role {:?} for {}",
                        payload.artifact_ref.role, payload.artifact_ref.artifact_id
                    ));
                }
            }
        }
        for event in commit {
            if !payload_has_required_reference(commit, event.payload()) {
                return Err(format!(
                    "typed payload in commit {} lacks a committed artifact reference",
                    seq
                ));
            }
        }
        index = end;
    }
    artifacts.extend(validate_referenced_config_artifacts(
        runtime_spec,
        config_artifacts,
    )?);
    Ok(artifacts)
}

#[cfg(test)]
fn validate_referenced_config_artifacts(
    runtime_spec: &CertifiedRuntimeSpec,
    artifacts: Vec<store::ArtifactEvidenceRef>,
) -> Result<Vec<store::ArtifactEvidenceRef>, String> {
    let mut expected = runtime_spec
        .spec()
        .config_refs
        .iter()
        .map(|config| (config_ref_key(config), config_artifact_evidence(config)))
        .collect::<BTreeMap<_, _>>();
    let mut validated = BTreeMap::new();
    for artifact in artifacts {
        if artifact.artifact_role != events::ArtifactRole::TypedConfig
            || artifact.semantic_type_id.is_some()
            || artifact.producer_node_id.is_some()
            || artifact.producer_seed_id.is_some()
        {
            return Err(format!(
                "typed config artifact reference {} carried non-config evidence",
                artifact.artifact_id
            ));
        }
        let key = config_artifact_key(&artifact)?;
        let expected_artifact = expected.remove(&key).ok_or_else(|| {
            format!(
                "typed config artifact reference {} is not certified by the spec",
                artifact.artifact_id
            )
        })?;
        if artifact != expected_artifact {
            return Err(format!(
                "typed config artifact reference {} does not match certified config evidence",
                artifact.artifact_id
            ));
        }
        if validated.insert(key, artifact).is_some() {
            return Err("duplicate typed config artifact reference".to_owned());
        }
    }
    if !expected.is_empty() {
        return Err("missing typed config artifact reference".to_owned());
    }
    Ok(validated.into_values().collect())
}

#[cfg(test)]
fn payload_has_required_reference(
    commit: &[store::KernelEventEnvelope],
    payload: &events::KernelEventPayload,
) -> bool {
    match payload {
        events::KernelEventPayload::CellProduced(payload) => commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ArtifactReferenced(reference)
                    if reference.artifact_ref.role == events::ArtifactRole::StateOutput
                        && reference.node_id.as_ref() == Some(&payload.node_id)
                        && reference.attempt_id.as_ref() == Some(&payload.attempt_id)
                        && reference.artifact_ref.artifact_id == payload.artifact_id
                        && reference.artifact_ref.content_digest == payload.content_digest
                        && reference.artifact_ref.schema_id == payload.schema_id
                        && reference.artifact_ref.semantic_type_id.as_ref()
                            == Some(&payload.semantic_type_id)
            )
        }),
        events::KernelEventPayload::FactRecorded(payload) => commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ArtifactReferenced(reference)
                    if reference.artifact_ref.role == events::ArtifactRole::FactResponse
                        && reference.node_id.as_ref() == Some(&payload.node_id)
                        && reference.attempt_id.as_ref() == Some(&payload.attempt_id)
                        && reference.artifact_ref.artifact_id == payload.artifact_id
                        && reference.artifact_ref.content_digest == payload.response_hash
                        && reference.artifact_ref.schema_id == payload.response_schema_id
            )
        }),
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            let Some(artifact_id) = &payload.rendered_artifact_id else {
                return true;
            };
            commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::ArtifactReferenced(reference)
                        if reference.artifact_ref.role == events::ArtifactRole::PublicOutput
                            && reference.node_id.as_ref() == Some(&payload.node_id)
                            && reference.attempt_id.as_ref() == Some(&payload.attempt_id)
                            && &reference.artifact_ref.artifact_id == artifact_id
                            && reference.artifact_ref.content_digest == payload.rendered_digest
                            && reference.artifact_ref.schema_id == payload.public_schema_id
                )
            })
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            let Some(diagnostic) = &payload.error.diagnostic_ref else {
                return true;
            };
            commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::ArtifactReferenced(reference)
                        if reference.node_id.as_ref() == Some(&payload.node_id)
                            && reference.attempt_id.as_ref() == Some(&payload.attempt_id)
                            && event_artifact_refs_match(diagnostic, reference)
                )
            })
        }
        events::KernelEventPayload::StateAttemptFailed(payload) => {
            let Some(diagnostic) = &payload.error.diagnostic_ref else {
                return true;
            };
            commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::ArtifactReferenced(reference)
                        if reference.node_id.as_ref() == Some(&payload.node_id)
                            && reference.attempt_id.as_ref() == Some(&payload.attempt_id)
                    && event_artifact_refs_match(diagnostic, reference)
                )
            })
        }
        events::KernelEventPayload::ManualResolutionRecorded(payload) => {
            commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::ArtifactReferenced(reference)
                        if manual_resolution_artifact_ref_matches(
                            payload,
                            reference,
                            events::ArtifactRole::ManualResolutionEvidence
                        )
                )
            }) && commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::ArtifactReferenced(reference)
                        if manual_resolution_artifact_ref_matches(
                            payload,
                            reference,
                            events::ArtifactRole::ManualResolutionAuthorization
                        )
                )
            })
        }
        _ => true,
    }
}

#[cfg(test)]
fn config_artifact_key(artifact: &store::ArtifactEvidenceRef) -> Result<String, String> {
    let Some(schema_id) = &artifact.schema_id else {
        return Err(format!(
            "typed config artifact reference {} missing schema id",
            artifact.artifact_id
        ));
    };
    Ok(format!("{}:{}", schema_id, artifact.digest))
}

#[cfg(test)]
fn reference_matches_same_commit_payload(
    commit: &[store::KernelEventEnvelope],
    reference: &events::ArtifactReferenced,
) -> bool {
    if matches!(
        reference.artifact_ref.role,
        events::ArtifactRole::ManualResolutionEvidence
            | events::ArtifactRole::ManualResolutionAuthorization
    ) {
        return commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ManualResolutionRecorded(payload)
                    if manual_resolution_artifact_ref_matches(
                        payload,
                        reference,
                        reference.artifact_ref.role
                    )
            )
        });
    }

    let (Some(reference_node_id), Some(reference_attempt_id)) =
        (&reference.node_id, &reference.attempt_id)
    else {
        return false;
    };
    commit.iter().any(|event| match event.payload() {
        events::KernelEventPayload::CellProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::StateOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.artifact_id == reference.artifact_ref.artifact_id
                && payload.content_digest == reference.artifact_ref.content_digest
                && payload.schema_id == reference.artifact_ref.schema_id
                && reference.artifact_ref.semantic_type_id.as_ref()
                    == Some(&payload.semantic_type_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::FactResponse
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.artifact_id == reference.artifact_ref.artifact_id
                && payload.response_hash == reference.artifact_ref.content_digest
                && payload.response_schema_id == reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::PublicOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.rendered_artifact_id.as_ref()
                    == Some(&reference.artifact_ref.artifact_id)
                && payload.rendered_digest == reference.artifact_ref.content_digest
                && payload.public_schema_id == reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::RedactedDiagnostic
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload
                    .error
                    .diagnostic_ref
                    .as_ref()
                    .is_some_and(|diagnostic| event_artifact_refs_match(diagnostic, reference))
        }
        events::KernelEventPayload::StateAttemptFailed(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::RedactedDiagnostic
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload
                    .error
                    .diagnostic_ref
                    .as_ref()
                    .is_some_and(|diagnostic| event_artifact_refs_match(diagnostic, reference))
        }
        _ => false,
    })
}

#[cfg(test)]
fn manual_resolution_artifact_ref_matches(
    payload: &events::ManualResolutionRecorded,
    reference: &events::ArtifactReferenced,
    role: events::ArtifactRole,
) -> bool {
    if reference.node_id.is_some()
        || reference.attempt_id.is_some()
        || reference.artifact_ref.semantic_type_id.is_some()
        || reference.artifact_ref.role != role
    {
        return false;
    }

    match role {
        events::ArtifactRole::ManualResolutionEvidence => {
            reference.artifact_ref.artifact_id == payload.evidence_artifact_id
                && reference.artifact_ref.content_digest == payload.evidence_hash
                && reference.artifact_ref.schema_id == payload.evidence_schema_id
        }
        events::ArtifactRole::ManualResolutionAuthorization => {
            reference.artifact_ref.artifact_id == payload.authorization_artifact_id
                && reference.artifact_ref.content_digest == payload.authorization_hash
                && reference.artifact_ref.schema_id == payload.authorization_schema_id
        }
        _ => false,
    }
}

#[cfg(test)]
fn event_artifact_refs_match(
    diagnostic: &events::ArtifactEvidenceRef,
    reference: &events::ArtifactReferenced,
) -> bool {
    diagnostic.artifact_id == reference.artifact_ref.artifact_id
        && diagnostic.role == reference.artifact_ref.role
        && diagnostic.schema_id == reference.artifact_ref.schema_id
        && diagnostic.semantic_type_id == reference.artifact_ref.semantic_type_id
        && diagnostic.content_digest == reference.artifact_ref.content_digest
        && diagnostic.byte_len == reference.artifact_ref.byte_len
        && diagnostic.media_type == reference.artifact_ref.media_type
}

#[cfg(test)]
fn referenced_artifact(payload: &events::ArtifactReferenced) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: payload.artifact_ref.artifact_id.clone(),
        digest: payload.artifact_ref.content_digest.clone(),
        byte_len: payload.artifact_ref.byte_len,
        media_type: payload.artifact_ref.media_type.clone(),
        schema_id: Some(payload.artifact_ref.schema_id.clone()),
        semantic_type_id: payload.artifact_ref.semantic_type_id.clone(),
        producer_node_id: payload.node_id.clone(),
        producer_seed_id: None,
        artifact_role: payload.artifact_ref.role,
    }
}

#[cfg(test)]
fn insert_artifact(
    artifacts: &mut BTreeMap<ArtifactId, store::ArtifactEvidenceRef>,
    artifact: store::ArtifactEvidenceRef,
) {
    artifacts
        .entry(artifact.artifact_id.clone())
        .or_insert(artifact);
}

#[cfg(test)]
fn event_artifact(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<SemanticTypeId>,
    producer_node_id: Option<NodeId>,
    role: events::ArtifactRole,
) -> Result<store::ArtifactEvidenceRef, String> {
    Ok(store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 128,
        media_type: spec::MediaType::new("application/json").map_err(display_error)?,
        schema_id,
        semantic_type_id,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    })
}

fn append_attempt_started(
    store: &mut store::InMemoryTypedRunStore,
    fixture: &ReferenceFixture,
    node: &spec::NodeSpec,
    attempt_no: u32,
) -> Result<AttemptId, String> {
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        attempt_no,
    )?;
    let request = typed_commit_request(
        fixture.run_id.clone(),
        store.expected_next_seq(&fixture.run_id),
        store::CommitKey::new(format!(
            "drift-attempt-start:{}:{}",
            node.node_id, attempt_id
        ))
        .map_err(display_error)?,
        vec![events::KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            },
        )],
        Vec::new(),
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    )?;
    let commit = store::PreparedCommit::<store::StateAttemptStarted>::new(
        request,
        store::CommitArtifactEvidenceSet::empty(),
    )
    .map_err(display_error)?;
    store
        .append_prepared_commit_plan(commit.into())
        .map_err(display_error)?;
    Ok(attempt_id)
}

async fn start_reference_run(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
    fixture: &ReferenceFixture,
) -> Result<(), String> {
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture.run_id.clone(),
            run_start_evidence(fixture, vec![fixture.seed_ref.clone()])?,
            store.expected_next_seq(&fixture.run_id),
        )
        .map_err(display_error)?;
    scheduler_start_run(scheduler, store, launch)
        .await
        .map_err(display_error)?;
    Ok(())
}

async fn scheduler_start_run(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
    launch: mfm_runtime::PreparedRunLaunch,
) -> mfm_runtime::Result<()> {
    let async_store = store::AsyncInMemoryTypedRunStore::from_store(std::mem::take(store));
    let result = scheduler.start_run(&async_store, launch).await.map(|_| ());
    restore_in_memory_store(store, &async_store)?;
    result
}

async fn scheduler_drive_once(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> mfm_runtime::Result<SchedulerStatus> {
    let async_store = store::AsyncInMemoryTypedRunStore::from_store(std::mem::take(store));
    let result = scheduler
        .drive_once(&async_store, runtime_spec, run_id)
        .await;
    restore_in_memory_store(store, &async_store)?;
    result
}

fn restore_in_memory_store(
    store: &mut store::InMemoryTypedRunStore,
    async_store: &store::AsyncInMemoryTypedRunStore,
) -> mfm_runtime::Result<()> {
    *store = async_store
        .with_inner(Clone::clone)
        .map_err(RuntimeError::from)?;
    Ok(())
}

#[cfg(test)]
async fn start_compensated_reference_run(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
    fixture: &CompensatedReferenceFixture,
) -> Result<(), String> {
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture.run_id.clone(),
            run_start_evidence_for(
                &fixture.runtime_spec,
                &fixture.config_bytes,
                &fixture.seed_bytes,
                vec![fixture.seed_ref.clone()],
            )?,
            store.expected_next_seq(&fixture.run_id),
        )
        .map_err(display_error)?;
    scheduler_start_run(scheduler, store, launch)
        .await
        .map_err(display_error)?;
    Ok(())
}

fn run_start_evidence(
    fixture: &ReferenceFixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<RunLaunchEvidence, String> {
    run_start_evidence_for(
        &fixture.runtime_spec,
        &fixture.config_bytes,
        &fixture.seed_bytes,
        seed_cells,
    )
}

fn run_start_evidence_for(
    runtime_spec: &CertifiedRuntimeSpec,
    config_bytes: &BTreeMap<String, Vec<u8>>,
    seed_bytes: &[u8],
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<RunLaunchEvidence, String> {
    Ok(RunLaunchEvidence {
        spec_artifact: spec_artifact(runtime_spec)?,
        certificate_artifact: certificate_artifact(runtime_spec)?,
        config_artifacts: runtime_spec
            .spec()
            .config_refs
            .iter()
            .map(|config| config_artifact_from(config_bytes, config))
            .collect::<Result<Vec<_>, _>>()?,
        framework_version: events::FrameworkVersion::new("mfm.typed_slice.framework.v1")
            .map_err(display_error)?,
        source_revision: events::SourceRevision::new("typed-certified-slice")
            .map_err(display_error)?,
        launched_at_unix_ms: 1_700_000_000_000,
        adapter_executables: vec![executable("deterministic-local-adapter")?],
        seed_cells: seed_cells
            .into_iter()
            .map(|cell| RunLaunchSeedCell {
                bytes: seed_bytes.to_vec(),
                cell,
            })
            .collect(),
    })
}

fn spec_artifact(runtime_spec: &CertifiedRuntimeSpec) -> Result<RunLaunchArtifact, String> {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(display_error)?;
    let digest = canonical.content_digest();
    Ok(RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: runtime_spec.spec().media_type.clone(),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedExecutionSpec,
        },
    })
}

fn certificate_artifact(runtime_spec: &CertifiedRuntimeSpec) -> Result<RunLaunchArtifact, String> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(display_error)?;
    let digest = canonical.content_digest();
    let media_type =
        spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).map_err(display_error)?;
    Ok(RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type,
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedSpecCertificate,
        },
    })
}

fn config_artifact_from(
    config_bytes: &BTreeMap<String, Vec<u8>>,
    config: &spec::ConfigRef,
) -> Result<RunLaunchArtifact, String> {
    let bytes = config_bytes
        .get(&config_ref_key(config))
        .ok_or_else(|| format!("missing config bytes for {}", config.artifact_id))?
        .clone();
    Ok(RunLaunchArtifact {
        bytes,
        evidence: config_artifact_evidence(config),
    })
}

fn config_artifact_evidence(config: &spec::ConfigRef) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: config.artifact_id.clone(),
        digest: config.digest.clone(),
        byte_len: config.byte_len,
        media_type: config.media_type.clone(),
        schema_id: Some(config.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    }
}

fn config_ref_key(config: &spec::ConfigRef) -> String {
    format!("{}:{}", config.schema_id, config.digest)
}

fn config_bytes_for_draft_and_spec(
    draft: &mfm_program::TypedProgramDraft,
    typed_spec: &spec::TypedExecutionSpec,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut configs = BTreeMap::new();
    for config in draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.remediation_nodes().values().map(|node| &node.config))
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
    {
        let digest = config.canonical_json.content_digest();
        configs.insert(
            format!("{}:{}", config.schema_id, digest),
            config.canonical_json.to_vec(),
        );
    }
    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes = spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
            .map_err(display_error)?;
        configs.insert(config_ref_key(&node.config_ref), bytes.to_vec());
    }
    Ok(configs)
}

#[cfg(test)]
fn seed_artifact(seed: &events::SeedCellRef) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: seed.seed_artifact.artifact_id.clone(),
        digest: seed.seed_artifact.content_digest.clone(),
        byte_len: seed.seed_artifact.byte_len,
        media_type: seed.seed_artifact.media_type.clone(),
        schema_id: Some(seed.seed_artifact.schema_id.clone()),
        semantic_type_id: seed.seed_artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: Some(seed.seed_id.clone()),
        artifact_role: seed.seed_artifact.role,
    }
}

fn terminal_payloads(
    ctx: &ErasedRunCtx<'_>,
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
) -> Vec<RunnerEventPayload> {
    vec![RunnerEventPayload::CellProduced(events::CellProduced {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        cell_id: ctx.node().output_cell.clone(),
        scope_id: ctx.node().scope_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
        schema_id: ctx.descriptor().output_schema_id.clone(),
        value_lineage: ctx.output_cell().value_lineage.clone(),
        artifact_id: output_artifact,
        content_digest: output_digest,
        producer_state_kind: Some(ctx.node().state_kind.clone()),
        producer_state_version: Some(ctx.node().state_version.clone()),
    })]
}

#[derive(Debug, Clone)]
struct TestArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn test_artifact_bytes(label: &str) -> Vec<u8> {
    canonical_json(serde_json::json!({ "typed_slice_artifact": label }))
        .expect("canonical test artifact")
        .to_vec()
}

fn digest_for_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn state_output_artifact(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    label: &str,
) -> TestArtifact {
    let bytes = test_artifact_bytes(&format!("{label}:{}", node.node_id));
    let digest = digest_for_bytes(&bytes);
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id_for_digest(&digest),
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("valid media"),
        schema_id: Some(descriptor.output_schema_id.clone()),
        semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    TestArtifact { bytes, evidence }
}

fn side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    label: &str,
    role: events::ArtifactRole,
) -> TestArtifact {
    let bytes = test_artifact_bytes(&format!(
        "{label}:{}:{}",
        ctx.node().node_id,
        ctx.attempt_id()
    ));
    let digest = digest_for_bytes(&bytes);
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id_for_digest(&digest),
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("valid media"),
        schema_id: Some(ctx.node().config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    };
    TestArtifact { bytes, evidence }
}

fn staged_attempt_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &TestArtifact,
) -> mfm_runtime::Result<StagedArtifact> {
    StagedArtifact::inline_attempt_artifact(ctx, artifact.bytes.clone(), artifact.evidence.clone())
}

fn staged_side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &TestArtifact,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> mfm_runtime::Result<StagedArtifact> {
    StagedArtifact::inline_side_effect_artifact(
        ctx,
        artifact.bytes.clone(),
        artifact.evidence.clone(),
        ledger_key,
        invocation_epoch,
    )
}

fn retain_artifacts<'a>(
    artifacts: impl IntoIterator<Item = &'a store::ArtifactEvidenceRef>,
) -> Vec<StagedRetentionRefs> {
    vec![StagedRetentionRefs::runtime_evidence(
        artifacts
            .into_iter()
            .map(|artifact| events::RetentionRef {
                artifact_id: artifact.artifact_id.clone(),
                role: artifact.artifact_role,
                content_digest: artifact.digest.clone(),
            })
            .collect(),
    )]
}

fn side_effect_claimed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    remediation_links: &BTreeMap<NodeId, NodeId>,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectClaimed(events::side_effect::Claimed {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx, remediation_links),
        claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
    })
}

fn side_effect_claim_taken_over(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    remediation_links: &BTreeMap<NodeId, NodeId>,
    previous: &store::SideEffectClaimProjection,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectClaimTakenOver(events::side_effect::ClaimTakenOver {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx, remediation_links),
        previous_claim_owner: previous.claim_owner.clone(),
        new_claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
        invocation_epoch: previous.invocation_epoch,
        previous_claim_generation: previous.claim_generation,
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
    })
}

fn side_effect_prepared(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    remediation_links: &BTreeMap<NodeId, NodeId>,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectInvocationPrepared(events::side_effect::InvocationPrepared {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx, remediation_links),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
        prepared_artifact_id: None,
        prepared_hash: None,
        resource_key: None,
    })
}

fn side_effect_invocation_started(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    remediation_links: &BTreeMap<NodeId, NodeId>,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectInvocationStarted(events::side_effect::InvocationStarted {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx, remediation_links),
        invocation_epoch,
        claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
    })
}

fn side_effect_submission_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    remediation_links: &BTreeMap<NodeId, NodeId>,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectSubmissionObserved(events::side_effect::SubmissionObserved {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx, remediation_links),
        invocation_epoch,
        submission_schema_id: ctx.node().config_ref.schema_id.clone(),
        submission_hash: digest,
        submission_artifact_id: artifact_id,
    })
}

fn side_effect_receipt_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    remediation_links: &BTreeMap<NodeId, NodeId>,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectReceiptObserved(events::side_effect::ReceiptObserved {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx, remediation_links),
        invocation_epoch,
        receipt_schema_id: ctx.node().config_ref.schema_id.clone(),
        receipt_hash: digest,
        receipt_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("typed-slice-verifier")
            .expect("valid verifier"),
        resource_touched_set: None,
    })
}

fn side_effect_confirmation_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    remediation_links: &BTreeMap<NodeId, NodeId>,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectConfirmationObserved(events::side_effect::ConfirmationObserved {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: ledger,
        ledger_purpose: side_effect_ledger_purpose_for_ctx(ctx, remediation_links),
        invocation_epoch,
        confirmation_schema_id: ctx.node().config_ref.schema_id.clone(),
        confirmation_hash: digest,
        confirmation_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("typed-slice-verifier")
            .expect("valid verifier"),
        resource_touched_set: None,
    })
}

fn assert_cell_input_terminal(input: &MaterializedInputNode) -> mfm_runtime::Result<()> {
    let MaterializedInputNode::Cell(cell) = input else {
        return Err(RuntimeError::InvalidRunnerOutput(
            "reference runner expected a cell input".to_owned(),
        ));
    };
    match cell.terminal {
        MaterializedCellTerminal::Seed { .. } | MaterializedCellTerminal::Produced { .. } => Ok(()),
        MaterializedCellTerminal::Skipped { .. } => Err(RuntimeError::InvalidRunnerOutput(
            "reference runner requires produced input".to_owned(),
        )),
    }
}

fn node_by_output<'a>(fixture: &'a ReferenceFixture, cell_id: &CellId) -> &'a spec::NodeSpec {
    fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| &node.output_cell == cell_id)
        .expect("node by output")
}

fn attempt_id(
    run_id: &RunId,
    spec_hash: &SpecHash,
    node_id: &NodeId,
    attempt_no: u32,
) -> Result<AttemptId, String> {
    let canonical = canonical_json(serde_json::json!({
        "attempt_no": attempt_no,
        "node_id": node_id.as_str(),
        "run_id": run_id.as_str(),
        "spec_hash": spec_hash.as_str(),
    }))?;
    Ok(AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *canonical.content_digest().digest(),
    ))
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes, String> {
    let json = serde_json::to_string(&value).map_err(display_error)?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(display_error)
}

fn content_digest_json(value: serde_json::Value) -> Result<ContentDigest, String> {
    Ok(canonical_json(value)?.content_digest())
}

fn executable(factory: &str) -> Result<events::ExecutableIdentity, String> {
    Ok(events::ExecutableIdentity {
        factory_id: events::RunnerFactoryId::new(factory).map_err(display_error)?,
        source_revision: events::SourceRevision::new("typed-certified-slice")
            .map_err(display_error)?,
        cargo_package_name: events::PackageName::new("mfm-integration-tests")
            .map_err(display_error)?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))
            .map_err(display_error)?,
        cargo_package_digest: content(0xf8),
        binary_digest: content_digest_json(serde_json::json!({
            "factory": factory,
            "package": "mfm-integration-tests",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

fn side_effect_ledger_key_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    remediation_links: &BTreeMap<NodeId, NodeId>,
) -> events::SideEffectLedgerKey {
    if let Some(forward_ledger_key) = linked_forward_ledger_for_remediation(ctx, remediation_links)
    {
        events::SideEffectLedgerKey::new(format!(
            "typed-slice-remediation-{}-{}",
            forward_ledger_key,
            ctx.attempt_no()
        ))
        .expect("valid remediation ledger key")
    } else {
        side_effect_forward_ledger_key_for_ctx(ctx)
    }
}

fn side_effect_forward_ledger_key_for_ctx(ctx: &ErasedRunCtx<'_>) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!(
        "typed-slice-forward-{}-{}-{}",
        ctx.run_id(),
        ctx.node().node_id,
        ctx.attempt_no()
    ))
    .expect("valid forward ledger key")
}

fn side_effect_ledger_purpose_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    remediation_links: &BTreeMap<NodeId, NodeId>,
) -> events::SideEffectLedgerPurpose {
    linked_forward_ledger_for_remediation(ctx, remediation_links)
        .map(
            |forward_ledger_key| events::SideEffectLedgerPurpose::Remediation {
                forward_ledger_key,
            },
        )
        .unwrap_or(events::SideEffectLedgerPurpose::Forward)
}

fn linked_forward_ledger_for_remediation(
    ctx: &ErasedRunCtx<'_>,
    remediation_links: &BTreeMap<NodeId, NodeId>,
) -> Option<events::SideEffectLedgerKey> {
    let forward_node_id = remediation_links.get(&ctx.node().node_id)?;
    ctx.projections()
        .side_effects()
        .find_map(|(_, projection)| {
            (projection.intent.node_id == *forward_node_id
                && matches!(
                    &projection.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                )
                && matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                ))
            .then(|| projection.ledger_key.clone())
        })
}

fn first_forward_side_effect_projection(
    projection: &store::ProjectionSnapshot,
) -> Option<&store::SideEffectProjection> {
    projection.side_effects().find_map(|(_, side_effect)| {
        matches!(
            side_effect.ledger_purpose,
            events::SideEffectLedgerPurpose::Forward
        )
        .then_some(side_effect)
    })
}

fn side_effect_claim_owner(attempt_no: u32, generation: u32) -> events::RunnerInvocationId {
    events::RunnerInvocationId::new(format!("typed-slice-owner-{attempt_no}-{generation}"))
        .expect("valid owner")
}

fn side_effect_fencing_token(
    attempt_no: u32,
    generation: u32,
) -> events::side_effect::ClaimFencingToken {
    events::side_effect::ClaimFencingToken::new(format!(
        "typed-slice-token-{attempt_no}-{generation}"
    ))
    .expect("valid fencing token")
}

fn side_effect_error(retryable: bool) -> events::MfmErrorInfo {
    events::MfmErrorInfo {
        code: events::ErrorCode::new("typed_slice_side_effect_failed").expect("valid error code"),
        category: events::ErrorCategory::SideEffect,
        retryable,
        safe_message: "typed certified slice side-effect failed".to_owned(),
        public_details: None,
        diagnostic_ref: None,
    }
}

fn bytes(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn artifact(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn artifact_id_for_digest(digest: &ContentDigest) -> ArtifactId {
    ArtifactId::from_digest(digest.algorithm(), *digest.digest())
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn state_kind(name: &str, byte: u8) -> Result<StateKind, String> {
    StateKind::new(
        "mfm.typed_slice",
        name,
        DigestAlgorithm::Sha256JcsV1,
        bytes(byte),
    )
    .map_err(display_error)
}

impl ErasedNodeRunner for DeterministicSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let ledger = side_effect_ledger_key_for_ctx(&ctx, &self.remediation_links);
            let phase = ctx.projections().side_effect(&ledger);
            match phase {
                None => self.persist_intent_and_claim(ctx, ledger),
                Some(projection)
                    if matches!(
                        projection.phase,
                        store::SideEffectPhase::Claimed { .. }
                            | store::SideEffectPhase::InvocationPrepared { .. }
                    ) =>
                {
                    let claim = projection.claim.as_ref().ok_or_else(|| {
                        RuntimeError::InvalidRunnerOutput(
                            "claimed side-effect projection has no active claim".to_owned(),
                        )
                    })?;
                    Ok(ErasedRunnerOutput::new(vec![
                        side_effect_claim_taken_over(
                            &ctx,
                            ledger.clone(),
                            &self.remediation_links,
                            claim,
                            2,
                        ),
                        side_effect_prepared(&ctx, ledger.clone(), &self.remediation_links, 1, 2),
                        side_effect_invocation_started(&ctx, ledger, &self.remediation_links, 1, 2),
                    ]))
                }
                Some(store::SideEffectProjection {
                    phase:
                        store::SideEffectPhase::InvocationStarted {
                            invocation_epoch, ..
                        },
                    ..
                }) => {
                    let artifact = side_effect_artifact(
                        &ctx,
                        "submission-unknown",
                        events::ArtifactRole::SubmissionUnknownEvidence,
                    );
                    let staged_artifact = staged_side_effect_artifact(
                        &ctx,
                        &artifact,
                        ledger.clone(),
                        *invocation_epoch,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: retain_artifacts([&artifact.evidence]),
                        payloads: vec![RunnerEventPayload::SideEffectSubmissionUnknown(
                            events::side_effect::SubmissionUnknown {
                                spec_hash: ctx.spec_hash().clone(),
                                node_id: ctx.node().node_id.clone(),
                                attempt_id: ctx.attempt_id().clone(),
                                ledger_key: ledger,
                                ledger_purpose: side_effect_ledger_purpose_for_ctx(
                                    &ctx,
                                    &self.remediation_links,
                                ),
                                invocation_epoch: *invocation_epoch,
                                evidence_schema_id: ctx.node().config_ref.schema_id.clone(),
                                evidence_hash: artifact.evidence.digest,
                                evidence_artifact_id: artifact.evidence.artifact_id,
                            },
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::SubmissionUnknown { invocation_epoch },
                    ..
                }) => {
                    let artifact =
                        side_effect_artifact(&ctx, "submission", events::ArtifactRole::Submission);
                    let staged_artifact = staged_side_effect_artifact(
                        &ctx,
                        &artifact,
                        ledger.clone(),
                        *invocation_epoch,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: retain_artifacts([&artifact.evidence]),
                        payloads: vec![side_effect_submission_observed(
                            &ctx,
                            ledger,
                            &self.remediation_links,
                            *invocation_epoch,
                            artifact.evidence.artifact_id,
                            artifact.evidence.digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::SubmissionObserved { invocation_epoch },
                    ..
                }) => {
                    let artifact =
                        side_effect_artifact(&ctx, "receipt", events::ArtifactRole::Receipt);
                    let staged_artifact = staged_side_effect_artifact(
                        &ctx,
                        &artifact,
                        ledger.clone(),
                        *invocation_epoch,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: retain_artifacts([&artifact.evidence]),
                        payloads: vec![side_effect_receipt_observed(
                            &ctx,
                            ledger,
                            &self.remediation_links,
                            *invocation_epoch,
                            artifact.evidence.artifact_id,
                            artifact.evidence.digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::ReceiptObserved { invocation_epoch },
                    ..
                }) => {
                    let artifact = side_effect_artifact(
                        &ctx,
                        "confirmation",
                        events::ArtifactRole::Confirmation,
                    );
                    let staged_artifact = staged_side_effect_artifact(
                        &ctx,
                        &artifact,
                        ledger.clone(),
                        *invocation_epoch,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: retain_artifacts([&artifact.evidence]),
                        payloads: vec![side_effect_confirmation_observed(
                            &ctx,
                            ledger,
                            &self.remediation_links,
                            *invocation_epoch,
                            artifact.evidence.artifact_id,
                            artifact.evidence.digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::ConfirmationObserved { .. },
                    ..
                }) => {
                    let artifact =
                        state_output_artifact(ctx.node(), ctx.descriptor(), self.output_label);
                    let staged_artifact = staged_attempt_artifact(&ctx, &artifact)?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: retain_artifacts([&artifact.evidence]),
                        payloads: terminal_payloads(
                            &ctx,
                            artifact.evidence.artifact_id,
                            artifact.evidence.digest,
                        ),
                    })
                }
                Some(_) => Err(RuntimeError::Blocked(
                    "side-effect reference runner blocked".to_owned(),
                )),
            }
        })
    }
}

impl DeterministicSideEffectRunner {
    fn persist_intent_and_claim(
        &self,
        ctx: ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
    ) -> mfm_runtime::Result<ErasedRunnerOutput> {
        if !ctx.caps().contains(&self.cap_kind, &self.cap_version) {
            return Err(RuntimeError::InvalidRunnerOutput(
                "side-effect runner missing certified mutation capability".to_owned(),
            ));
        }
        let artifact = side_effect_artifact(
            &ctx,
            "reference-intent",
            events::ArtifactRole::SideEffectIntent,
        );
        let staged_artifact = staged_side_effect_artifact(&ctx, &artifact, ledger.clone(), 1)?;
        Ok(ErasedRunnerOutput {
            staged_artifacts: vec![staged_artifact],
            staged_retention_refs: retain_artifacts([&artifact.evidence]),
            payloads: vec![
                RunnerEventPayload::SideEffectIntentPersisted(
                    events::side_effect::IntentPersisted {
                        spec_hash: ctx.spec_hash().clone(),
                        node_id: ctx.node().node_id.clone(),
                        scope_id: ctx.node().scope_id.clone(),
                        attempt_id: ctx.attempt_id().clone(),
                        ledger_key: ledger.clone(),
                        ledger_purpose: side_effect_ledger_purpose_for_ctx(
                            &ctx,
                            &self.remediation_links,
                        ),
                        invocation_epoch: 1,
                        intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                        intent_hash: artifact.evidence.digest.clone(),
                        intent_artifact_id: artifact.evidence.artifact_id.clone(),
                        idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
                        idempotency_input_hash: content(0xdd),
                        idempotency_key: events::IdempotencyKeyRef::new(format!(
                            "reference-idem-{}-{}",
                            ctx.node().node_id,
                            ctx.attempt_no()
                        ))
                        .map_err(RuntimeError::from)?,
                        capability_kind: self.cap_kind.clone(),
                        capability_version: self.cap_version.clone(),
                        adapter_kind: self.adapter_kind.clone(),
                        adapter_version: self.adapter_version.clone(),
                    },
                ),
                side_effect_claimed(&ctx, ledger.clone(), &self.remediation_links, 1, 1),
                side_effect_prepared(&ctx, ledger, &self.remediation_links, 1, 1),
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_kernel_scenario_data::replay_artifacts;

    #[tokio::test]
    async fn typed_certified_slice_acceptance_passes_required_contract() {
        let summary = typed_certified_slice_coverage()
            .await
            .expect("typed certified slice summary");
        summary
            .validate_required_contract()
            .expect("required contract");
    }

    #[tokio::test]
    async fn compensated_certified_slice_replays_and_reports_public_status() {
        let run = run_compensated_reference_workflow()
            .await
            .expect("compensated reference workflow");
        let stream = run.store.load_run_stream(&run.fixture.run_id);
        let projection =
            store::ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("projection");
        let saga = projection
            .derive_saga_projection(&run.fixture.run_id, &run.fixture.runtime_spec.spec().saga);

        assert_eq!(saga.run_mode, store::RunMode::Compensated);
        assert_eq!(saga.obligations.len(), 2);
        assert!(saga.obligations.values().all(|obligation| {
            obligation.classification == store::ForwardLedgerClassification::Owed
                && obligation
                    .remediation
                    .as_ref()
                    .is_some_and(|remediation| remediation.closed)
        }));
        assert!(matches!(
            projection
                .run_completion(&run.fixture.run_id)
                .expect("run completion")
                .outcome,
            events::RunCompletionOutcome::Compensated
        ));

        let forward_confirmation_order = stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::SideEffectConfirmationObserved(payload)
                    if matches!(
                        payload.ledger_purpose,
                        events::SideEffectLedgerPurpose::Forward
                    ) =>
                {
                    Some(payload.ledger_key.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(forward_confirmation_order.len(), 2);
        let remediation_order = stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                    match &payload.ledger_purpose {
                        events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } => {
                            Some(forward_ledger_key.clone())
                        }
                        events::SideEffectLedgerPurpose::Forward => None,
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            remediation_order,
            forward_confirmation_order
                .iter()
                .rev()
                .cloned()
                .collect::<Vec<_>>()
        );

        replay_without_live_capabilities_for(
            &run.fixture.runtime_spec,
            &run.store,
            &run.artifacts,
            &run.fixture.run_id,
        )
        .await
        .expect("replay without live capabilities");

        assert_eq!(
            mfm_app::TypedRunMode::from(saga.run_mode),
            mfm_app::TypedRunMode::Compensated
        );
        assert!(saga.obligations.values().all(|obligation| {
            obligation.classification == store::ForwardLedgerClassification::Owed
                && obligation
                    .remediation
                    .as_ref()
                    .is_some_and(|remediation| remediation.closed)
        }));
    }

    #[tokio::test]
    async fn replay_artifacts_reject_extra_config_reference_in_start_commit() {
        let fixture = reference_fixture().expect("fixture");
        let scheduler = test_scheduler(
            reference_registry(&fixture).expect("registry"),
            TestRuntimeArtifactStore::default(),
        );
        let mut store = store::InMemoryTypedRunStore::new();
        start_reference_run(&scheduler, &mut store, &fixture)
            .await
            .expect("start run");

        let stream = store.load_run_stream(&fixture.run_id);
        let config = fixture
            .runtime_spec
            .spec()
            .config_refs
            .first()
            .expect("config ref");
        let corrupt_stream = append_payload_to_start_commit(
            &stream,
            &fixture.run_id,
            events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: None,
                attempt_id: None,
                artifact_ref: events::ArtifactEvidenceRef {
                    artifact_id: artifact(0xc1),
                    role: events::ArtifactRole::TypedConfig,
                    schema_id: config.schema_id.clone(),
                    semantic_type_id: None,
                    content_digest: content(0xc2),
                    byte_len: config.byte_len + 1,
                    media_type: config.media_type.clone(),
                },
            }),
        );

        assert!(matches!(
            replay_artifacts(&fixture, &corrupt_stream),
            Err(message) if message.contains("not certified")
        ));
    }

    #[tokio::test]
    async fn replay_artifact_negative_cases_match_scenario_data() {
        let run = run_reference_certified_workflow()
            .await
            .expect("reference workflow");

        for case in replay_artifacts::NEGATIVE_CASE_SCENARIO.corruption_cases {
            let actual = replay_artifact_negative_case_kind(&run, case.name)
                .await
                .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
            assert_eq!(
                actual, case.expected_error_kind,
                "unexpected replay artifact negative kind for {}",
                case.name
            );
        }
    }

    async fn replay_artifact_negative_case_kind(
        run: &ReferenceRun,
        case_name: &str,
    ) -> Result<&'static str, String> {
        let stream = run.store.load_run_stream(&run.fixture.run_id);
        match case_name {
            "wrong_role" => {
                let (corrupt_stream, replaced) = rewrite_first_artifact_reference(
                    &stream,
                    events::ArtifactRole::StateOutput,
                    |payload| {
                        payload.artifact_ref.role = events::ArtifactRole::PublicOutput;
                    },
                );
                if !replaced {
                    return Err("missing state output reference for wrong-role case".to_owned());
                }
                replay_artifacts(&run.fixture, &corrupt_stream)
                    .expect_err("wrong-role artifact reference rejects");
                Ok("invalid_run_stream")
            }
            "wrong_producer" => {
                let wrong_node = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(0xf1));
                let (corrupt_stream, replaced) = rewrite_first_artifact_reference(
                    &stream,
                    events::ArtifactRole::StateOutput,
                    |payload| {
                        payload.node_id = Some(wrong_node.clone());
                    },
                );
                if !replaced {
                    return Err("missing state output reference for wrong-producer case".to_owned());
                }
                replay_artifacts(&run.fixture, &corrupt_stream)
                    .expect_err("wrong-producer artifact reference rejects");
                Ok("invalid_run_stream")
            }
            "missing_committed_artifact_reference" => {
                let (corrupt_stream, removed) =
                    remove_first_artifact_reference(&stream, events::ArtifactRole::StateOutput);
                if !removed {
                    return Err(
                        "reference workflow emitted no state output reference to remove".to_owned(),
                    );
                }
                let error = replay_artifacts(&run.fixture, &corrupt_stream)
                    .expect_err("missing state output reference rejects");
                if !error.contains("lacks a committed artifact reference") {
                    return Err(format!("unexpected missing-reference error: {error}"));
                }
                Ok("artifact_missing")
            }
            "tampered_spec" => {
                let corrupt_stream = rewrite_run_admitted(&stream, |payload| {
                    payload.spec_artifact.artifact_id = artifact(0xe1);
                    payload.spec_artifact.content_digest = content(0xe1);
                })?;
                replay_artifact_stream_error_kind(run, corrupt_stream).await
            }
            "tampered_certificate" => {
                let corrupt_stream = rewrite_run_admitted(&stream, |payload| {
                    payload.certificate_artifact.artifact_id = artifact(0xe2);
                    payload.certificate_artifact.content_digest = content(0xe2);
                })?;
                replay_artifact_stream_error_kind(run, corrupt_stream).await
            }
            "replay_with_live_capability" => replay_artifact_stream_error_kind(run, stream).await,
            other => Err(format!("unknown replay artifact negative case {other}")),
        }
    }

    async fn replay_artifact_stream_error_kind(
        run: &ReferenceRun,
        stream: Vec<store::KernelEventEnvelope>,
    ) -> Result<&'static str, String> {
        let error = replay_stream_without_live_capabilities(
            &run.fixture.runtime_spec,
            stream,
            &run.artifacts,
        )
        .await
        .expect_err("replay artifact negative case rejects");
        Ok(replay_error_kind_label(error.kind))
    }

    fn replay_error_kind_label(kind: replay::ReplayErrorKind) -> &'static str {
        match kind {
            replay::ReplayErrorKind::InvalidRunStream => "invalid_run_stream",
            replay::ReplayErrorKind::ArtifactMissing => "artifact_missing",
            replay::ReplayErrorKind::LiveCapabilityRequest => "live_capability_request",
            _ => "other",
        }
    }

    async fn replay_stream_without_live_capabilities(
        runtime_spec: &CertifiedRuntimeSpec,
        stream: Vec<store::KernelEventEnvelope>,
        artifacts: &TestRuntimeArtifactStore,
    ) -> replay::Result<()> {
        let run_admitted = stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RunAdmitted(payload) => Some(payload.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                replay::ReplayError::new(
                    replay::ReplayErrorKind::RunAdmittedMissing,
                    "missing RunAdmitted for replay artifact case",
                )
            })?;
        let committed = store::CommittedRunStream::from_events(run_admitted.run_id.clone(), stream)
            .map_err(|error| {
                replay::ReplayError::new(
                    replay::ReplayErrorKind::InvalidRunStream,
                    error.to_string(),
                )
            })?;
        let retained_artifacts =
            store::VerifiedRunArtifactStore::from_committed_stream(&committed, artifacts)
                .await
                .map_err(|error| {
                    replay::ReplayError::new(
                        replay::ReplayErrorKind::ArtifactMissing,
                        error.to_string(),
                    )
                })?;
        let verified_history = mfm_runtime::VerifiedRunHistory::from_committed_stream(
            runtime_spec,
            committed,
            retained_artifacts,
        )
        .map_err(|error| {
            replay::ReplayError::new(replay::ReplayErrorKind::InvalidRunStream, error.to_string())
        })?;
        let authority = replay::ReplayReadAuthority::from_verified_run_history(
            runtime_spec,
            &verified_history,
        )?;
        let broker = replay::ReplayBroker::from_read_authority(authority)?;
        broker.reject_live_capability_request()
    }

    fn rewrite_first_artifact_reference(
        stream: &[store::KernelEventEnvelope],
        role: events::ArtifactRole,
        mutate: impl FnOnce(&mut events::ArtifactReferenced),
    ) -> (Vec<store::KernelEventEnvelope>, bool) {
        let mut mutate = Some(mutate);
        rewrite_commits(stream, |payloads| {
            if mutate.is_none() {
                return false;
            }
            for payload in payloads {
                let events::KernelEventPayload::ArtifactReferenced(reference) = payload else {
                    continue;
                };
                if reference.artifact_ref.role == role {
                    let mutate = mutate.take().expect("mutation is used once");
                    mutate(reference);
                    return true;
                }
            }
            false
        })
    }

    fn remove_first_artifact_reference(
        stream: &[store::KernelEventEnvelope],
        role: events::ArtifactRole,
    ) -> (Vec<store::KernelEventEnvelope>, bool) {
        let mut removed = false;
        let (rewritten, changed) = rewrite_commits(stream, |payloads| {
            if removed {
                return false;
            }
            let Some(index) = payloads.iter().position(|payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::ArtifactReferenced(reference)
                        if reference.artifact_ref.role == role
                )
            }) else {
                return false;
            };
            payloads.remove(index);
            removed = true;
            true
        });
        (rewritten, changed)
    }

    fn rewrite_run_admitted(
        stream: &[store::KernelEventEnvelope],
        mutate: impl FnOnce(&mut events::RunAdmitted),
    ) -> Result<Vec<store::KernelEventEnvelope>, String> {
        let mut mutate = Some(mutate);
        let (rewritten, changed) = rewrite_commits(stream, |payloads| {
            if mutate.is_none() {
                return false;
            }
            for payload in payloads {
                let events::KernelEventPayload::RunAdmitted(run_admitted) = payload else {
                    continue;
                };
                let mutate = mutate.take().expect("mutation is used once");
                mutate(run_admitted);
                return true;
            }
            false
        });
        if changed {
            Ok(rewritten)
        } else {
            Err("missing RunAdmitted payload".to_owned())
        }
    }

    fn rewrite_commits(
        stream: &[store::KernelEventEnvelope],
        mut mutate_payloads: impl FnMut(&mut Vec<events::KernelEventPayload>) -> bool,
    ) -> (Vec<store::KernelEventEnvelope>, bool) {
        let mut rewritten = Vec::with_capacity(stream.len());
        let mut changed = false;
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let mut payloads = stream[index..end]
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            if mutate_payloads(&mut payloads) {
                let request = typed_commit_request(
                    first.run_id().clone(),
                    seq,
                    commit_key,
                    payloads,
                    Vec::new(),
                    store::CommitPreconditions::default(),
                )
                .expect("rewritten replay artifact commit request");
                let batch = store::build_committed_batch(&request, seq)
                    .expect("rewritten replay artifact commit batch");
                rewritten.extend(batch.events().iter().cloned());
                changed = true;
            } else {
                rewritten.extend(stream[index..end].iter().cloned());
            }
            index = end;
        }
        (rewritten, changed)
    }

    fn append_payload_to_start_commit(
        stream: &[store::KernelEventEnvelope],
        run_id: &RunId,
        payload: events::KernelEventPayload,
    ) -> Vec<store::KernelEventEnvelope> {
        let start = stream
            .iter()
            .position(|event| matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_)))
            .expect("RunAdmitted event");
        let seq = stream[start].seq();
        let commit_key = stream[start].commit_key().clone();
        let mut end = start;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let mut payloads = stream[start..end]
            .iter()
            .map(|event| event.payload().clone())
            .collect::<Vec<_>>();
        payloads.push(payload);
        let request = typed_commit_request(
            run_id.clone(),
            seq,
            commit_key,
            payloads,
            Vec::new(),
            store::CommitPreconditions::default(),
        )
        .expect("corrupt start commit request");
        let batch =
            store::build_committed_batch(&request, seq).expect("corrupt start commit batch");
        let mut rewritten = Vec::with_capacity(stream.len() + 1);
        rewritten.extend(stream[..start].iter().cloned());
        rewritten.extend(batch.events().iter().cloned());
        rewritten.extend(stream[end..].iter().cloned());
        rewritten
    }
}
