#![warn(missing_docs)]
//! Test support for MFM typed kernel contracts.
//!
//! The crate owns reusable synthetic fixtures for CI gates that need to exercise the typed
//! certified runtime without importing old dynamic authoring or execution APIs.

use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{
    ApplySideEffect, CapabilitySpec, ExternalMutationAuthorityRole, ManagedPlatformWrite,
    ManagedPlatformWriteRole, NoCaps, Pure, ReadExternal, ReadExternalRole,
};
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, NodeId, RunId, SchemaId,
    SemanticTypeId, SpecHash, StateKind, StateVersion,
};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, IdempotencyKey,
    ManagedWriteState, PublicOutputKey, PureState, ReadState, RootBuilder, ScopeKey, SeedKey,
    SideEffectState, StateKey, StateRegistryBuilder, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    build_retention_manifest_artifact, CertifiedRuntimeSpec, ErasedNodeRunner, ErasedRunCtx,
    ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    MaterializedCellTerminal, MaterializedInputNode, RunStartEvidence, RuntimeError,
    SchedulerStatus, SerialTypedScheduler,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_store::v1::{TypedProjectionRead, TypedRunEventStore};
use serde::{Deserialize, Serialize};

/// Summary emitted by the `typed-certified-slice` CI acceptance gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedCertifiedSliceSummary {
    typed_spec_hash_persisted: bool,
    run_started_v1_present: bool,
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

impl TypedCertifiedSliceSummary {
    /// Converts the acceptance summary into the stable CI summary document.
    pub fn to_summary_document(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": "typed-certified-slice-summary",
            "version": 1,
            "payload": {
                "typed_spec_hash_persisted": self.typed_spec_hash_persisted,
                "run_started_v1_present": self.run_started_v1_present,
                "seed_material_persisted": self.seed_material_persisted,
                "cell_events_count": self.cell_events_count,
                "side_effect_ledger_complete": self.side_effect_ledger_complete,
                "side_effect_invocation_started_before_submit": self.side_effect_invocation_started_before_submit,
                "side_effect_crash_cases_passed": self.side_effect_crash_cases_passed,
                "side_effect_no_duplicate_submit": self.side_effect_no_duplicate_submit,
                "side_effect_submission_unknown_recovered": self.side_effect_submission_unknown_recovered,
                "side_effect_failed_semantics_covered": self.side_effect_failed_semantics_covered,
                "side_effect_logical_key_conflicts_rejected": self.side_effect_logical_key_conflicts_rejected,
                "managed_platform_outputs_committed": self.managed_platform_outputs_committed,
                "public_output_before_run_completed": self.public_output_before_run_completed,
                "public_output_event_id": self.public_output_event_id,
                "replay_live_cap_requests_count": self.replay_live_cap_requests_count,
                "resume_drift_fixture_count": self.resume_drift_fixture_count,
                "resume_drift_rejected": self.resume_drift_rejected,
                "retention_projection_complete": self.retention_projection_complete,
            }
        })
    }

    fn validate_required_contract(&self) -> Result<(), String> {
        let true_keys = [
            ("typed_spec_hash_persisted", self.typed_spec_hash_persisted),
            ("run_started_v1_present", self.run_started_v1_present),
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

/// Runs the full synthetic typed certified slice and returns its CI summary.
pub async fn typed_certified_slice_summary() -> Result<TypedCertifiedSliceSummary, String> {
    let mut run = run_reference_certified_workflow().await?;
    let side_effect_logical_key_conflicts_rejected = duplicate_submit_rejected(&mut run)?;
    let replay_live_cap_requests_count = replay_without_live_capabilities(&run)?;
    let resume_drift_rejected = resume_drift_is_rejected().await?;
    if !side_effect_ambiguity_blocks_completion().await? {
        return Err("typed-certified-slice ambiguity fixture did not block completion".to_owned());
    }
    let side_effect_failed_semantics_covered = side_effect_failure_semantics_are_covered().await?;
    let incomplete_retention_rejected = incomplete_retention_projection_is_rejected()?;

    let stream = run.store.load_run_stream(&run.fixture.run_id);
    let projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(&stream).map_err(display_error)?;
    let side_effect_projection = projection
        .side_effect(&side_effect_ledger_key(1))
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

    let summary = TypedCertifiedSliceSummary {
        typed_spec_hash_persisted: typed_spec_hash_persisted(&run, &stream),
        run_started_v1_present: stream
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunStarted(_))),
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
}

#[derive(Clone)]
struct ReferenceFixture {
    runtime_spec: CertifiedRuntimeSpec,
    run_id: RunId,
    seed_ref: events::SeedCellRef,
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

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
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

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
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

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
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

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
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

async fn run_reference_certified_workflow() -> Result<ReferenceRun, String> {
    let fixture = reference_fixture()?;
    let scheduler = SerialTypedScheduler::new(reference_registry(&fixture)?);
    let mut store = store::InMemoryTypedRunStore::new();
    let mut manifest_appended = false;
    scheduler
        .start_run(
            &mut store,
            &fixture.runtime_spec,
            fixture.run_id.clone(),
            run_start_evidence(&fixture, vec![fixture.seed_ref.clone()])?,
        )
        .map_err(display_error)?;

    for _ in 0..32 {
        if !manifest_appended
            && store
                .projection_snapshot()
                .public_output(&fixture.public_schema)
                .is_some()
            && store.projection_snapshot().run_state(&fixture.run_id) == store::RunState::Started
        {
            let manifest = build_retention_manifest_artifact(
                &fixture.runtime_spec,
                &fixture.run_id,
                &store.load_run_stream(&fixture.run_id),
            )
            .map_err(display_error)?;
            scheduler
                .append_retention_manifest_projection(
                    &mut store,
                    &fixture.runtime_spec,
                    &fixture.run_id,
                    manifest,
                )
                .map_err(display_error)?;
            manifest_appended = true;
            continue;
        }

        match scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .map_err(display_error)?
        {
            SchedulerStatus::Advanced => {}
            SchedulerStatus::PublicOutputProjected => {
                return Ok(ReferenceRun { fixture, store });
            }
            SchedulerStatus::Blocked => return Err("reference workflow blocked".to_owned()),
        }
    }

    Err("reference workflow did not reach terminal public-output projection".to_owned())
}

fn reference_registry(fixture: &ReferenceFixture) -> Result<ErasedRunnerRegistry, String> {
    reference_registry_with_side_effect(
        fixture,
        "apply_side_effect",
        DeterministicSideEffectRunner::new(fixture),
    )
}

fn reference_registry_with_side_effect<R: ErasedNodeRunner + 'static>(
    fixture: &ReferenceFixture,
    side_effect_factory: &str,
    side_effect_runner: R,
) -> Result<ErasedRunnerRegistry, String> {
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.pure_descriptor.clone(),
            "pure",
            TerminalRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
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
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
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
                output_artifact: artifact(0xc1),
                output_digest: content(0xc2),
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
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
}

impl ErasedNodeRunner for TerminalRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            assert_cell_input_terminal(&ctx.inputs.root)?;
            for (kind, version) in &self.expected_caps {
                if !ctx.caps.contains(kind, version) {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "missing certified capability {kind}:{version}"
                    )));
                }
            }
            let artifact = state_output_artifact(
                ctx.node,
                ctx.descriptor,
                self.output_artifact.clone(),
                self.output_digest.clone(),
            );
            Ok(ErasedRunnerOutput {
                required_artifacts: vec![artifact],
                staged_retention_refs: Vec::new(),
                payloads: terminal_payloads(
                    &ctx,
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                ),
            })
        })
    }
}

struct ReadRunner {
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
}

impl ErasedNodeRunner for ReadRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            assert_cell_input_terminal(&ctx.inputs.root)?;
            if !ctx.caps.contains(&self.cap_kind, &self.cap_version) {
                return Err(RuntimeError::InvalidRunnerOutput(
                    "read runner missing certified read capability".to_owned(),
                ));
            }
            let fact_key = events::FactKey::new("reference-read").map_err(RuntimeError::from)?;
            let fact_artifact = artifact(0xb3);
            let fact_digest = content(0xb4);
            let fact_evidence = store::ArtifactEvidenceRef {
                artifact_id: fact_artifact.clone(),
                digest: fact_digest.clone(),
                byte_len: 23,
                media_type: spec::MediaType::new("application/json")?,
                schema_id: Some(ctx.node.config_ref.schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(ctx.node.node_id.clone()),
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::FactResponse,
            };
            let output = state_output_artifact(
                ctx.node,
                ctx.descriptor,
                self.output_artifact.clone(),
                self.output_digest.clone(),
            );
            let mut payloads = vec![events::KernelEventPayload::FactRecorded(
                events::FactRecorded {
                    spec_hash: ctx.spec_hash.clone(),
                    node_id: ctx.node.node_id.clone(),
                    attempt_id: ctx.attempt_id.clone(),
                    capability_kind: self.cap_kind.clone(),
                    capability_version: self.cap_version.clone(),
                    adapter_kind: self.adapter_kind.clone(),
                    adapter_version: self.adapter_version.clone(),
                    request_schema_id: ctx.node.config_ref.schema_id.clone(),
                    request_hash: content(0xb5),
                    response_schema_id: ctx.node.config_ref.schema_id.clone(),
                    response_hash: fact_digest,
                    fact_key,
                    artifact_id: fact_artifact,
                },
            )];
            payloads.extend(terminal_payloads(
                &ctx,
                self.output_artifact.clone(),
                self.output_digest.clone(),
            ));
            Ok(ErasedRunnerOutput {
                required_artifacts: vec![fact_evidence, output],
                staged_retention_refs: Vec::new(),
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
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
}

impl DeterministicSideEffectRunner {
    fn new(fixture: &ReferenceFixture) -> Self {
        Self {
            cap_kind: fixture.side_effect_cap_kind.clone(),
            cap_version: fixture.side_effect_cap_version.clone(),
            adapter_kind: fixture.adapter_kind.clone(),
            adapter_version: fixture.adapter_version.clone(),
            output_artifact: artifact(0xd1),
            output_digest: content(0xd2),
        }
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
            let side_effect = root.scope().state::<ReferenceSideEffectState, _>(
                StateKey::new("side-effect")?,
                ReferenceConfig { multiplier: 5 },
                managed,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("public-output")?,
                &ReferencePublicOutputs {
                    result: side_effect,
                },
            )
        },
    )
    .map_err(display_error)?;
    let certified = mfm_certify::certify_program_draft(&draft).map_err(display_error)?;
    let runtime_spec = CertifiedRuntimeSpec::new(certified).map_err(display_error)?;
    let spec = runtime_spec.spec();
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

fn typed_spec_hash_persisted(run: &ReferenceRun, stream: &[store::KernelEventEnvelope]) -> bool {
    stream.iter().any(|event| match event.payload() {
        events::KernelEventPayload::RunStarted(payload) => {
            payload.spec_hash == *run.fixture.runtime_spec.spec_hash()
                && stream
                    .iter()
                    .any(|retention_event| match retention_event.payload() {
                        events::KernelEventPayload::RetentionRefsAppended(retention) => {
                            retention.refs.iter().any(|retention_ref| {
                                retention_ref.role == events::ArtifactRole::TypedExecutionSpec
                            })
                        }
                        _ => false,
                    })
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
    let verified = store::VerifiedRetentionProjectionSet::from_run_streams([(
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

fn incomplete_retention_projection_is_rejected() -> Result<bool, String> {
    let fixture = reference_fixture()?;
    let scheduler = SerialTypedScheduler::new(reference_registry(&fixture)?);
    let mut store = store::InMemoryTypedRunStore::new();
    scheduler
        .start_run(
            &mut store,
            &fixture.runtime_spec,
            fixture.run_id.clone(),
            run_start_evidence(&fixture, vec![fixture.seed_ref.clone()])?,
        )
        .map_err(display_error)?;
    let stream = store.load_run_stream(&fixture.run_id);
    Ok(
        store::VerifiedRetentionProjectionSet::from_run_streams(std::iter::empty::<(
            RunId,
            &[store::KernelEventEnvelope],
        )>())
        .is_err()
            && !retention_projection_complete(&ReferenceRun { fixture, store }, &stream)?,
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
    run.store
        .record_artifact_evidence(evidence.clone())
        .map_err(display_error)?;
    let result = run
        .store
        .append_typed_run_commit(store::TypedCommitRequest {
            run_id: run.fixture.run_id.clone(),
            expected_next_seq: run.store.expected_next_seq(&run.fixture.run_id),
            commit_key: store::CommitKey::new("duplicate-submit-conflict")
                .map_err(display_error)?,
            payloads: vec![events::KernelEventPayload::SideEffectSubmissionObserved(
                events::side_effect::SubmissionObserved {
                    spec_hash: run.fixture.runtime_spec.spec_hash().clone(),
                    node_id: submission.node_id,
                    attempt_id: submission.attempt_id,
                    ledger_key: submission.ledger_key,
                    invocation_epoch: submission.invocation_epoch,
                    submission_schema_id: submission.submission_schema_id,
                    submission_hash: duplicate_digest,
                    submission_artifact_id: duplicate_artifact,
                },
            )],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::Any,
                ..store::CommitPreconditions::default()
            },
        });
    Ok(matches!(
        result,
        Err(store::StoreError::LogicalKeyConflict { .. })
            | Err(store::StoreError::DuplicateLogicalKey { .. })
    ))
}

fn replay_without_live_capabilities(run: &ReferenceRun) -> Result<u64, String> {
    let stream = run.store.load_run_stream(&run.fixture.run_id);
    let run_started = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload.clone()),
            _ => None,
        })
        .ok_or_else(|| "missing RunStarted for replay fixture".to_owned())?;
    let artifacts = replay_artifacts(&run.fixture, &stream)?;
    let authority = replay::ReplayAuthority::from_certified_spec(
        run.fixture.runtime_spec.envelope(),
        run_started.runner_executables,
        run_started.adapter_executables,
        artifacts,
    );
    let broker = replay::ReplayBroker::from_run_stream(
        run.fixture.runtime_spec.envelope().clone(),
        &stream,
        authority,
    )
    .map_err(display_error)?;
    let live_cap_rejected = matches!(
        broker.reject_live_capability_request(),
        Err(replay::ReplayError {
            kind: replay::ReplayErrorKind::LiveCapabilityRequest,
            ..
        })
    );
    if live_cap_rejected {
        Ok(0)
    } else {
        Err("replay broker did not reject a live capability request".to_owned())
    }
}

async fn resume_drift_is_rejected() -> Result<bool, String> {
    let fixture = reference_fixture()?;
    let scheduler = SerialTypedScheduler::new(reference_registry(&fixture)?);
    let mut store = store::InMemoryTypedRunStore::new();
    scheduler
        .start_run(
            &mut store,
            &fixture.runtime_spec,
            fixture.run_id.clone(),
            run_start_evidence(&fixture, vec![fixture.seed_ref.clone()])?,
        )
        .map_err(display_error)?;
    let read_node = node_by_output(&fixture, &fixture.read_cell).clone();
    append_attempt_started(&mut store, &fixture, &read_node, 1)?;
    Ok(matches!(
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
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
    let scheduler = SerialTypedScheduler::new(registry);
    let mut store = store::InMemoryTypedRunStore::new();
    scheduler
        .start_run(
            &mut store,
            &fixture.runtime_spec,
            fixture.run_id.clone(),
            run_start_evidence(&fixture, vec![fixture.seed_ref.clone()])?,
        )
        .map_err(display_error)?;
    for _ in 0..4 {
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .map_err(display_error)?;
    }
    let projection = store.projection_snapshot();
    let side_effect = projection
        .side_effect(&side_effect_ledger_key(1))
        .ok_or_else(|| "missing failed side-effect projection".to_owned())?;
    Ok(matches!(
        side_effect.phase,
        store::SideEffectPhase::Failed { .. }
    ))
}

async fn side_effect_ambiguity_blocks_completion() -> Result<bool, String> {
    let fixture = reference_fixture()?;
    let registry = reference_registry_with_side_effect(
        &fixture,
        "apply_side_effect",
        AmbiguousSideEffectRunner::new(&fixture),
    )?;
    let scheduler = SerialTypedScheduler::new(registry);
    let mut store = store::InMemoryTypedRunStore::new();
    scheduler
        .start_run(
            &mut store,
            &fixture.runtime_spec,
            fixture.run_id.clone(),
            run_start_evidence(&fixture, vec![fixture.seed_ref.clone()])?,
        )
        .map_err(display_error)?;

    for _ in 0..16 {
        match scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .map_err(display_error)?
        {
            SchedulerStatus::Advanced => {}
            SchedulerStatus::PublicOutputProjected => return Ok(false),
            SchedulerStatus::Blocked => {
                let stream = store.load_run_stream(&fixture.run_id);
                let ambiguous = stream.iter().any(|event| {
                    matches!(
                        event.payload(),
                        events::KernelEventPayload::SideEffectAmbiguous(_)
                    )
                });
                let completed = stream.iter().any(|event| {
                    matches!(
                        event.payload(),
                        events::KernelEventPayload::RunCompleted(_)
                            | events::KernelEventPayload::PublicOutputProduced(_)
                    )
                });
                return Ok(ambiguous && !completed);
            }
        }
    }

    Err("ambiguous side-effect fixture did not reach a blocked scheduler state".to_owned())
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
            let ledger = side_effect_ledger_key(ctx.attempt_no);
            let intent_artifact_id = artifact(0xf1);
            let intent_hash = content(0xf2);
            Ok(ErasedRunnerOutput {
                required_artifacts: vec![side_effect_artifact(
                    &ctx,
                    intent_artifact_id.clone(),
                    intent_hash.clone(),
                    events::ArtifactRole::SideEffectIntent,
                )],
                staged_retention_refs: Vec::new(),
                payloads: vec![
                    events::KernelEventPayload::SideEffectIntentPersisted(
                        events::side_effect::IntentPersisted {
                            spec_hash: ctx.spec_hash.clone(),
                            node_id: ctx.node.node_id.clone(),
                            scope_id: ctx.node.scope_id.clone(),
                            attempt_id: ctx.attempt_id.clone(),
                            ledger_key: ledger.clone(),
                            invocation_epoch: 1,
                            intent_schema_id: ctx.node.config_ref.schema_id.clone(),
                            intent_hash,
                            intent_artifact_id,
                            idempotency_input_schema_id: ctx.node.config_ref.schema_id.clone(),
                            idempotency_input_hash: content(0xf3),
                            idempotency_key: events::IdempotencyKeyRef::new("failing-idem")
                                .map_err(RuntimeError::from)?,
                            capability_kind: self.cap_kind.clone(),
                            capability_version: self.cap_version.clone(),
                            adapter_kind: self.adapter_kind.clone(),
                            adapter_version: self.adapter_version.clone(),
                        },
                    ),
                    events::KernelEventPayload::SideEffectFailed(events::side_effect::Failed {
                        spec_hash: ctx.spec_hash.clone(),
                        node_id: ctx.node.node_id.clone(),
                        attempt_id: ctx.attempt_id.clone(),
                        ledger_key: ledger,
                        invocation_epoch: 1,
                        failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                        retryable: false,
                        error: side_effect_error(false),
                    }),
                    events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                        spec_hash: ctx.spec_hash.clone(),
                        node_id: ctx.node.node_id.clone(),
                        attempt_id: ctx.attempt_id.clone(),
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
            let ledger = side_effect_ledger_key(ctx.attempt_no);
            let phase = ctx.projections.side_effect(&ledger);
            if matches!(
                phase.map(|projection| &projection.phase),
                Some(store::SideEffectPhase::InvocationStarted { .. })
            ) {
                let artifact_id = artifact(0xe1);
                let digest = content(0xe2);
                Ok(ErasedRunnerOutput {
                    required_artifacts: vec![side_effect_artifact(
                        &ctx,
                        artifact_id.clone(),
                        digest.clone(),
                        events::ArtifactRole::AmbiguityEvidence,
                    )],
                    staged_retention_refs: Vec::new(),
                    payloads: vec![events::KernelEventPayload::SideEffectAmbiguous(
                        events::side_effect::Ambiguous {
                            spec_hash: ctx.spec_hash.clone(),
                            node_id: ctx.node.node_id.clone(),
                            attempt_id: ctx.attempt_id.clone(),
                            ledger_key: ledger,
                            invocation_epoch: 1,
                            ambiguity_code: events::AmbiguityCode::new("unknown_submission")
                                .map_err(RuntimeError::from)?,
                            evidence_schema_id: ctx.node.config_ref.schema_id.clone(),
                            evidence_hash: digest,
                            evidence_artifact_id: artifact_id,
                        },
                    )],
                })
            } else {
                self.inner.run_erased(ctx).await
            }
        })
    }
}

fn replay_artifacts(
    fixture: &ReferenceFixture,
    stream: &[store::KernelEventEnvelope],
) -> Result<Vec<store::ArtifactEvidenceRef>, String> {
    let mut artifacts = BTreeMap::<ArtifactId, store::ArtifactEvidenceRef>::new();
    insert_artifact(&mut artifacts, spec_artifact(&fixture.runtime_spec)?);
    insert_artifact(&mut artifacts, certificate_artifact(&fixture.runtime_spec)?);
    for config in &fixture.runtime_spec.spec().config_refs {
        insert_artifact(&mut artifacts, config_artifact(config));
    }
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => {
                for seed in &payload.seed_cells {
                    insert_artifact(&mut artifacts, seed_artifact(seed));
                }
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.artifact_id.clone(),
                        payload.response_hash.clone(),
                        Some(payload.response_schema_id.clone()),
                        None,
                        Some(payload.node_id.clone()),
                        events::ArtifactRole::FactResponse,
                    )?,
                );
            }
            events::KernelEventPayload::CellProduced(payload) => {
                insert_artifact(
                    &mut artifacts,
                    event_artifact(
                        payload.artifact_id.clone(),
                        payload.content_digest.clone(),
                        Some(payload.schema_id.clone()),
                        Some(payload.semantic_type_id.clone()),
                        Some(payload.node_id.clone()),
                        events::ArtifactRole::StateOutput,
                    )?,
                );
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
            events::KernelEventPayload::ArtifactReferenced(_)
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
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_) => {}
        }
    }
    Ok(artifacts.into_values().collect())
}

fn insert_artifact(
    artifacts: &mut BTreeMap<ArtifactId, store::ArtifactEvidenceRef>,
    artifact: store::ArtifactEvidenceRef,
) {
    artifacts
        .entry(artifact.artifact_id.clone())
        .or_insert(artifact);
}

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
    store
        .append_typed_run_commit(store::TypedCommitRequest {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "drift-attempt-start:{}:{}",
                node.node_id, attempt_id
            ))
            .map_err(display_error)?,
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .map_err(display_error)?;
    Ok(attempt_id)
}

fn run_start_evidence(
    fixture: &ReferenceFixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<RunStartEvidence, String> {
    Ok(RunStartEvidence {
        spec_artifact: spec_artifact(&fixture.runtime_spec)?,
        certificate_artifact: certificate_artifact(&fixture.runtime_spec)?,
        config_artifacts: fixture
            .runtime_spec
            .spec()
            .config_refs
            .iter()
            .map(config_artifact)
            .collect(),
        framework_version: events::FrameworkVersion::new("mfm.typed_slice.framework.v1")
            .map_err(display_error)?,
        source_revision: events::SourceRevision::new("typed-certified-slice")
            .map_err(display_error)?,
        adapter_executables: vec![executable("deterministic-local-adapter")?],
        seed_cells,
    })
}

fn spec_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<store::ArtifactEvidenceRef, String> {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(display_error)?;
    let digest = canonical.content_digest();
    Ok(store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: canonical.as_bytes().len() as u64,
        media_type: runtime_spec.spec().media_type.clone(),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedExecutionSpec,
    })
}

fn certificate_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<store::ArtifactEvidenceRef, String> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(display_error)?;
    let digest = canonical.content_digest();
    let media_type =
        spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).map_err(display_error)?;
    Ok(store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: canonical.as_bytes().len() as u64,
        media_type,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedSpecCertificate,
    })
}

fn config_artifact(config: &spec::ConfigRef) -> store::ArtifactEvidenceRef {
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
) -> Vec<events::KernelEventPayload> {
    vec![
        events::KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            cell_id: ctx.node.output_cell.clone(),
            scope_id: ctx.node.scope_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            semantic_type_id: ctx.descriptor.output_semantic_type_id.clone(),
            schema_id: ctx.descriptor.output_schema_id.clone(),
            value_lineage: ctx.output_cell.value_lineage.clone(),
            artifact_id: output_artifact,
            content_digest: output_digest,
            producer_state_kind: Some(ctx.node.state_kind.clone()),
            producer_state_version: Some(ctx.node.state_version.clone()),
        }),
        events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            output_cell_id: ctx.node.output_cell.clone(),
        }),
    ]
}

fn state_output_artifact(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("valid media"),
        schema_id: Some(descriptor.output_schema_id.clone()),
        semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    }
}

fn side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("valid media"),
        schema_id: Some(ctx.node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(ctx.node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn side_effect_claimed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> events::KernelEventPayload {
    events::KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
        spec_hash: ctx.spec_hash.clone(),
        node_id: ctx.node.node_id.clone(),
        attempt_id: ctx.attempt_id.clone(),
        ledger_key: ledger,
        claim_owner: side_effect_claim_owner(ctx.attempt_no, claim_generation),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no, claim_generation),
    })
}

fn side_effect_claim_taken_over(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    previous: &store::SideEffectClaimProjection,
    claim_generation: u32,
) -> events::KernelEventPayload {
    events::KernelEventPayload::SideEffectClaimTakenOver(events::side_effect::ClaimTakenOver {
        spec_hash: ctx.spec_hash.clone(),
        node_id: ctx.node.node_id.clone(),
        attempt_id: ctx.attempt_id.clone(),
        ledger_key: ledger,
        previous_claim_owner: previous.claim_owner.clone(),
        new_claim_owner: side_effect_claim_owner(ctx.attempt_no, claim_generation),
        invocation_epoch: previous.invocation_epoch,
        previous_claim_generation: previous.claim_generation,
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no, claim_generation),
    })
}

fn side_effect_prepared(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> events::KernelEventPayload {
    events::KernelEventPayload::SideEffectInvocationPrepared(
        events::side_effect::InvocationPrepared {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            ledger_key: ledger,
            invocation_epoch,
            claim_generation,
            claim_fencing_token: side_effect_fencing_token(ctx.attempt_no, claim_generation),
            prepared_artifact_id: None,
            prepared_hash: None,
        },
    )
}

fn side_effect_invocation_started(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> events::KernelEventPayload {
    events::KernelEventPayload::SideEffectInvocationStarted(
        events::side_effect::InvocationStarted {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            ledger_key: ledger,
            invocation_epoch,
            claim_owner: side_effect_claim_owner(ctx.attempt_no, claim_generation),
            claim_generation,
            claim_fencing_token: side_effect_fencing_token(ctx.attempt_no, claim_generation),
        },
    )
}

fn side_effect_submission_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> events::KernelEventPayload {
    events::KernelEventPayload::SideEffectSubmissionObserved(
        events::side_effect::SubmissionObserved {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            ledger_key: ledger,
            invocation_epoch,
            submission_schema_id: ctx.node.config_ref.schema_id.clone(),
            submission_hash: digest,
            submission_artifact_id: artifact_id,
        },
    )
}

fn side_effect_receipt_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> events::KernelEventPayload {
    events::KernelEventPayload::SideEffectReceiptObserved(events::side_effect::ReceiptObserved {
        spec_hash: ctx.spec_hash.clone(),
        node_id: ctx.node.node_id.clone(),
        attempt_id: ctx.attempt_id.clone(),
        ledger_key: ledger,
        invocation_epoch,
        receipt_schema_id: ctx.node.config_ref.schema_id.clone(),
        receipt_hash: digest,
        receipt_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("typed-slice-verifier")
            .expect("valid verifier"),
    })
}

fn side_effect_confirmation_observed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> events::KernelEventPayload {
    events::KernelEventPayload::SideEffectConfirmationObserved(
        events::side_effect::ConfirmationObserved {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            ledger_key: ledger,
            invocation_epoch,
            confirmation_schema_id: ctx.node.config_ref.schema_id.clone(),
            confirmation_hash: digest,
            confirmation_artifact_id: artifact_id,
            replay_verifier_id: events::ReplayVerifierId::new("typed-slice-verifier")
                .expect("valid verifier"),
        },
    )
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
        cargo_package_name: events::PackageName::new("mfm-kernel-test-support")
            .map_err(display_error)?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))
            .map_err(display_error)?,
        cargo_package_digest: content(0xf8),
        binary_digest: content_digest_json(serde_json::json!({
            "factory": factory,
            "package": "mfm-kernel-test-support",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

fn side_effect_ledger_key(attempt_no: u32) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!("typed-slice-ledger-{attempt_no}"))
        .expect("valid ledger key")
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
            let ledger = side_effect_ledger_key(ctx.attempt_no);
            let phase = ctx.projections.side_effect(&ledger);
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
                        side_effect_claim_taken_over(&ctx, ledger.clone(), claim, 2),
                        side_effect_prepared(&ctx, ledger.clone(), 1, 2),
                        side_effect_invocation_started(&ctx, ledger, 1, 2),
                    ]))
                }
                Some(store::SideEffectProjection {
                    phase:
                        store::SideEffectPhase::InvocationStarted {
                            invocation_epoch, ..
                        },
                    ..
                }) => {
                    let artifact_id = artifact(0xd5);
                    let digest = content(0xd6);
                    Ok(ErasedRunnerOutput {
                        required_artifacts: vec![side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::SubmissionUnknownEvidence,
                        )],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![events::KernelEventPayload::SideEffectSubmissionUnknown(
                            events::side_effect::SubmissionUnknown {
                                spec_hash: ctx.spec_hash.clone(),
                                node_id: ctx.node.node_id.clone(),
                                attempt_id: ctx.attempt_id.clone(),
                                ledger_key: ledger,
                                invocation_epoch: *invocation_epoch,
                                evidence_schema_id: ctx.node.config_ref.schema_id.clone(),
                                evidence_hash: digest,
                                evidence_artifact_id: artifact_id,
                            },
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::SubmissionUnknown { invocation_epoch },
                    ..
                }) => {
                    let artifact_id = artifact(0xd7);
                    let digest = content(0xd8);
                    Ok(ErasedRunnerOutput {
                        required_artifacts: vec![side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::Submission,
                        )],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![side_effect_submission_observed(
                            &ctx,
                            ledger,
                            *invocation_epoch,
                            artifact_id,
                            digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::SubmissionObserved { invocation_epoch },
                    ..
                }) => {
                    let artifact_id = artifact(0xd9);
                    let digest = content(0xda);
                    Ok(ErasedRunnerOutput {
                        required_artifacts: vec![side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::Receipt,
                        )],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![side_effect_receipt_observed(
                            &ctx,
                            ledger,
                            *invocation_epoch,
                            artifact_id,
                            digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::ReceiptObserved { invocation_epoch },
                    ..
                }) => {
                    let artifact_id = artifact(0xdb);
                    let digest = content(0xdc);
                    Ok(ErasedRunnerOutput {
                        required_artifacts: vec![side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::Confirmation,
                        )],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![side_effect_confirmation_observed(
                            &ctx,
                            ledger,
                            *invocation_epoch,
                            artifact_id,
                            digest,
                        )],
                    })
                }
                Some(store::SideEffectProjection {
                    phase: store::SideEffectPhase::ConfirmationObserved { .. },
                    ..
                }) => {
                    let artifact = state_output_artifact(
                        ctx.node,
                        ctx.descriptor,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    );
                    Ok(ErasedRunnerOutput {
                        required_artifacts: vec![artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: terminal_payloads(
                            &ctx,
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
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
        if !ctx.caps.contains(&self.cap_kind, &self.cap_version) {
            return Err(RuntimeError::InvalidRunnerOutput(
                "side-effect runner missing certified mutation capability".to_owned(),
            ));
        }
        let intent_artifact_id = artifact(0xd3);
        let intent_hash = content(0xd4);
        Ok(ErasedRunnerOutput {
            required_artifacts: vec![side_effect_artifact(
                &ctx,
                intent_artifact_id.clone(),
                intent_hash.clone(),
                events::ArtifactRole::SideEffectIntent,
            )],
            staged_retention_refs: Vec::new(),
            payloads: vec![
                events::KernelEventPayload::SideEffectIntentPersisted(
                    events::side_effect::IntentPersisted {
                        spec_hash: ctx.spec_hash.clone(),
                        node_id: ctx.node.node_id.clone(),
                        scope_id: ctx.node.scope_id.clone(),
                        attempt_id: ctx.attempt_id.clone(),
                        ledger_key: ledger.clone(),
                        invocation_epoch: 1,
                        intent_schema_id: ctx.node.config_ref.schema_id.clone(),
                        intent_hash,
                        intent_artifact_id,
                        idempotency_input_schema_id: ctx.node.config_ref.schema_id.clone(),
                        idempotency_input_hash: content(0xdd),
                        idempotency_key: events::IdempotencyKeyRef::new("reference-idem")
                            .map_err(RuntimeError::from)?,
                        capability_kind: self.cap_kind.clone(),
                        capability_version: self.cap_version.clone(),
                        adapter_kind: self.adapter_kind.clone(),
                        adapter_version: self.adapter_version.clone(),
                    },
                ),
                side_effect_claimed(&ctx, ledger.clone(), 1, 1),
                side_effect_prepared(&ctx, ledger, 1, 1),
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn typed_certified_slice_acceptance_summary_passes_required_contract() {
        let summary = typed_certified_slice_summary()
            .await
            .expect("typed certified slice summary");
        summary
            .validate_required_contract()
            .expect("required contract");
    }
}
