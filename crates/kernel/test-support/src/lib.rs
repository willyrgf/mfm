#![warn(missing_docs)]
//! Test support for MFM typed kernel contracts.
//!
//! The crate owns reusable synthetic fixtures for CI gates that need to exercise the typed
//! certified runtime without importing old dynamic authoring or execution APIs.

use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{
    CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor, EffectSpec, ManagedPlatformWrite,
};
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EffectKind, EffectVersion, NodeId,
    RunId, SchemaId, ScopeId, SeedId, SemanticTypeId, SpecHash, StateKind, StateVersion,
};
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
        "side-effect",
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
            "read",
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
            "managed-write",
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
    let scope = scope(0x10);
    let seed_id = seed_id(0x11);
    let seed_cell = cell(0x12);
    let pure_node = node(0x13);
    let read_node = node(0x14);
    let managed_node = node(0x15);
    let side_effect_node = node(0x16);
    let render_node = node(0x17);
    let pure_cell = cell(0x18);
    let read_cell = cell(0x19);
    let managed_cell = cell(0x1a);
    let side_effect_cell = cell(0x1b);
    let render_cell = cell(0x1c);
    let pure_descriptor = descriptor(0x1d);
    let read_descriptor = descriptor(0x1e);
    let managed_descriptor = descriptor(0x1f);
    let side_effect_descriptor = descriptor(0x20);
    let render_descriptor = descriptor(0x21);
    let semantic = SemanticTypeId::new(
        "mfm.typed_slice",
        "value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x22),
    )
    .map_err(display_error)?;
    let value_schema = SchemaId::new(
        "mfm.typed_slice.value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x23),
    )
    .map_err(display_error)?;
    let input_schema = SchemaId::new(
        "mfm.typed_slice.input",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x24),
    )
    .map_err(display_error)?;
    let config_schema = SchemaId::new(
        "mfm.typed_slice.config",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x25),
    )
    .map_err(display_error)?;
    let public_schema = SchemaId::new(
        "mfm.typed_slice.public",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x26),
    )
    .map_err(display_error)?;
    let pure_effect = EffectKind::new(
        "mfm.typed_slice",
        "pure",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x27),
    )
    .map_err(display_error)?;
    let read_effect = EffectKind::new(
        "mfm.typed_slice",
        "read",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x28),
    )
    .map_err(display_error)?;
    let managed_effect = EffectKind::new(
        "mfm.typed_slice",
        "managed-write",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x29),
    )
    .map_err(display_error)?;
    let side_effect = EffectKind::new(
        "mfm.typed_slice",
        "side-effect",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x2a),
    )
    .map_err(display_error)?;
    let read_cap_kind = CapabilityKind::new(
        "mfm.typed_slice",
        "read-fixture",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x2b),
    )
    .map_err(display_error)?;
    let read_cap_version =
        CapabilityVersion::new("mfm.typed_slice.cap.read.v1").map_err(display_error)?;
    let managed_cap_kind = CapabilityKind::new(
        "mfm.typed_slice",
        "managed-output",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x2c),
    )
    .map_err(display_error)?;
    let managed_cap_version =
        CapabilityVersion::new("mfm.typed_slice.cap.managed_write.v1").map_err(display_error)?;
    let side_effect_cap_kind = CapabilityKind::new(
        "mfm.typed_slice",
        "external-mutation",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x2d),
    )
    .map_err(display_error)?;
    let side_effect_cap_version =
        CapabilityVersion::new("mfm.typed_slice.cap.external_mutation.v1")
            .map_err(display_error)?;
    let adapter_kind = AdapterKind::new(
        "mfm.typed_slice",
        "deterministic-local",
        DigestAlgorithm::Sha256JcsV1,
        bytes(0x2e),
    )
    .map_err(display_error)?;
    let adapter_version =
        AdapterVersion::new("mfm.typed_slice.adapter.local.v1").map_err(display_error)?;
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).map_err(display_error)?;
    let read_caps = CapabilitySetDescriptor::new(vec![CapabilityDescriptor::new(
        read_cap_kind.clone(),
        read_cap_version.clone(),
        CapabilityRole::ReadExternal,
        "read-fixture",
    )
    .map_err(display_error)?])
    .map_err(display_error)?;
    let managed_caps = CapabilitySetDescriptor::new(vec![CapabilityDescriptor::new(
        managed_cap_kind.clone(),
        managed_cap_version.clone(),
        CapabilityRole::ManagedPlatformWrite,
        "managed-output",
    )
    .map_err(display_error)?])
    .map_err(display_error)?;
    let side_effect_caps = CapabilitySetDescriptor::new(vec![CapabilityDescriptor::new(
        side_effect_cap_kind.clone(),
        side_effect_cap_version.clone(),
        CapabilityRole::ExternalMutationAuthority,
        "external-mutation",
    )
    .map_err(display_error)?])
    .map_err(display_error)?;
    let config_ref = spec::ConfigRef {
        schema_id: config_schema.clone(),
        artifact_id: artifact(0x2f),
        digest: content(0x30),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").map_err(display_error)?,
    };
    let planning = spec::PlanningLineage {
        active_operation_instances: Vec::new(),
        completed_operation_frames: Vec::new(),
        lineage_digest: content(0x31),
    };
    let lineage_seed = lineage(0x32);
    let lineage_pure = lineage(0x33);
    let lineage_read = lineage(0x34);
    let lineage_managed = lineage(0x35);
    let lineage_side_effect = lineage(0x36);
    let lineage_render = lineage(0x37);
    let seed_ref = events::SeedCellRef {
        seed_id: seed_id.clone(),
        cell_id: seed_cell.clone(),
        scope_id: scope.clone(),
        semantic_type_id: semantic.clone(),
        schema_id: value_schema.clone(),
        digest: content(0x38),
        seed_artifact: events::ArtifactEvidenceRef {
            artifact_id: artifact(0x39),
            role: events::ArtifactRole::SeedInput,
            schema_id: value_schema.clone(),
            semantic_type_id: Some(semantic.clone()),
            content_digest: content(0x38),
            byte_len: 11,
            media_type: spec::MediaType::new("application/json").map_err(display_error)?,
        },
    };

    let pure_spec = node_spec(NodeSpecFixture {
        node_id: pure_node.clone(),
        descriptor_id: pure_descriptor.clone(),
        scope_id: scope.clone(),
        state_key: "pure",
        state_kind: state_kind("pure", 0x3a)?,
        state_version: StateVersion::new("mfm.typed_slice.state.pure.v1").map_err(display_error)?,
        effect_kind: pure_effect.clone(),
        config_ref: config_ref.clone(),
        input_schema: input_schema.clone(),
        input_cell: seed_cell.clone(),
        input_schema_for_cell: value_schema.clone(),
        input_lineage: lineage_seed.clone(),
        output_cell: pure_cell.clone(),
        caps: no_caps.clone(),
        predecessors: Vec::new(),
        adapter_bindings: Vec::new(),
        planning: planning.clone(),
    })?;
    let read_spec = node_spec(NodeSpecFixture {
        node_id: read_node.clone(),
        descriptor_id: read_descriptor.clone(),
        scope_id: scope.clone(),
        state_key: "read",
        state_kind: state_kind("read", 0x3b)?,
        state_version: StateVersion::new("mfm.typed_slice.state.read.v1").map_err(display_error)?,
        effect_kind: read_effect.clone(),
        config_ref: config_ref.clone(),
        input_schema: input_schema.clone(),
        input_cell: pure_cell.clone(),
        input_schema_for_cell: value_schema.clone(),
        input_lineage: lineage_pure.clone(),
        output_cell: read_cell.clone(),
        caps: read_caps.clone(),
        predecessors: vec![pure_node.clone()],
        adapter_bindings: vec![spec::AdapterBinding {
            adapter_kind: adapter_kind.clone(),
            adapter_version: adapter_version.clone(),
            binding_digest: None,
        }],
        planning: planning.clone(),
    })?;
    let managed_spec = node_spec(NodeSpecFixture {
        node_id: managed_node.clone(),
        descriptor_id: managed_descriptor.clone(),
        scope_id: scope.clone(),
        state_key: "managed",
        state_kind: state_kind("managed", 0x3c)?,
        state_version: StateVersion::new("mfm.typed_slice.state.managed.v1")
            .map_err(display_error)?,
        effect_kind: managed_effect.clone(),
        config_ref: config_ref.clone(),
        input_schema: input_schema.clone(),
        input_cell: read_cell.clone(),
        input_schema_for_cell: value_schema.clone(),
        input_lineage: lineage_read.clone(),
        output_cell: managed_cell.clone(),
        caps: managed_caps.clone(),
        predecessors: vec![read_node.clone()],
        adapter_bindings: Vec::new(),
        planning: planning.clone(),
    })?;
    let side_effect_contract_digest = content(0x3d);
    let side_effect_spec = {
        let mut node = node_spec(NodeSpecFixture {
            node_id: side_effect_node.clone(),
            descriptor_id: side_effect_descriptor.clone(),
            scope_id: scope.clone(),
            state_key: "side-effect",
            state_kind: state_kind("side_effect", 0x3e)?,
            state_version: StateVersion::new("mfm.typed_slice.state.side_effect.v1")
                .map_err(display_error)?,
            effect_kind: side_effect.clone(),
            config_ref: config_ref.clone(),
            input_schema: input_schema.clone(),
            input_cell: managed_cell.clone(),
            input_schema_for_cell: value_schema.clone(),
            input_lineage: lineage_managed.clone(),
            output_cell: side_effect_cell.clone(),
            caps: side_effect_caps.clone(),
            predecessors: vec![managed_node.clone()],
            adapter_bindings: vec![spec::AdapterBinding {
                adapter_kind: adapter_kind.clone(),
                adapter_version: adapter_version.clone(),
                binding_digest: None,
            }],
            planning: planning.clone(),
        })?;
        node.side_effect = Some(spec::SideEffectContractSpec {
            contract_digest: side_effect_contract_digest.clone(),
        });
        node
    };

    let renderer = spec::RendererDescriptorIdentity {
        descriptor_id: descriptor(0x3f),
        renderer_kind: spec::RendererKind::new("public-output/json").map_err(display_error)?,
        renderer_version: spec::RendererVersion::new("mfm.typed_slice.renderer.v1")
            .map_err(display_error)?,
        public_schema_id: public_schema.clone(),
        canonicalizer_identity: spec::CanonicalizerIdentity::new("sha256-jcs-v1")
            .map_err(display_error)?,
    };
    let public_output_cell = spec::PublicOutputCell {
        public_field_path: spec::PublicFieldPath::new("result").map_err(display_error)?,
        cell_id: side_effect_cell.clone(),
        producer: spec::CellProducer::Node(side_effect_node.clone()),
        scope_id: scope.clone(),
        semantic_type_id: semantic.clone(),
        schema_id: value_schema.clone(),
        value_lineage: lineage_side_effect.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
    };
    let public_outputs = spec::PublicOutputSpec {
        public_schema_id: public_schema.clone(),
        outputs: vec![public_output_cell.clone()],
        renderer_descriptor: renderer.clone(),
    };
    let render_spec = render_node_spec(RenderNodeSpecFixture {
        node_id: render_node.clone(),
        descriptor_id: render_descriptor.clone(),
        scope_id: scope.clone(),
        config_ref: config_ref.clone(),
        public_output_cell: public_output_cell.clone(),
        public_outputs: public_outputs.clone(),
        render_cell: render_cell.clone(),
        planning: planning.clone(),
    })?;

    let typed_spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
        authoring: spec::AuthoringProvenance::StateComposition {
            descriptor: spec::CompositionDescriptor {
                descriptor_id: descriptor(0x40),
                name: "mfm.typed_slice.reference".to_owned(),
                version: "mfm.typed_slice.reference.v1".to_owned(),
            },
            config_hash: content(0x41),
        },
        scopes: vec![spec::ScopeSpec {
            scope_id: scope.clone(),
            parent_scope_id: None,
            stable_key: spec::StableAuthorKey::new("root").map_err(display_error)?,
            planning_lineage: planning.clone(),
        }],
        seeds: vec![spec::SeedSpec {
            seed_id: seed_id.clone(),
            seed_key: spec::StableAuthorKey::new("launch").map_err(display_error)?,
            cell_id: seed_cell.clone(),
            scope_id: scope.clone(),
            semantic_type_id: semantic.clone(),
            schema_id: value_schema.clone(),
            required_digest: Some(content(0x38)),
        }],
        descriptor_identities: vec![
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &pure_spec,
                pure_descriptor.clone(),
                "mfm.typed_slice.state.pure",
                pure_effect,
                no_caps.clone(),
                "pure",
                None,
            )?)),
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &read_spec,
                read_descriptor.clone(),
                "mfm.typed_slice.state.read",
                read_effect,
                read_caps,
                "read",
                None,
            )?)),
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &managed_spec,
                managed_descriptor.clone(),
                "mfm.typed_slice.state.managed",
                managed_effect,
                managed_caps,
                "managed-write",
                None,
            )?)),
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &side_effect_spec,
                side_effect_descriptor.clone(),
                "mfm.typed_slice.state.side_effect",
                side_effect,
                side_effect_caps,
                "side-effect",
                Some(side_effect_contract_digest),
            )?)),
            spec::DescriptorIdentity::State(Box::new(render_descriptor_identity(
                &render_spec,
                render_descriptor.clone(),
                public_schema.clone(),
                config_schema.clone(),
            )?)),
            spec::DescriptorIdentity::Renderer(Box::new(renderer)),
        ],
        config_refs: vec![config_ref.clone()],
        nodes: vec![
            render_spec,
            side_effect_spec,
            managed_spec,
            read_spec,
            pure_spec,
        ],
        cells: vec![
            cell_spec(
                seed_cell,
                spec::CellProducer::Seed(seed_id.clone()),
                &scope,
                &semantic,
                &value_schema,
                lineage_seed.clone(),
                spec::StoragePolicy::ContentAddressed,
            ),
            cell_spec(
                pure_cell.clone(),
                spec::CellProducer::Node(pure_node.clone()),
                &scope,
                &semantic,
                &value_schema,
                lineage_pure.clone(),
                spec::StoragePolicy::ContentAddressed,
            ),
            cell_spec(
                read_cell.clone(),
                spec::CellProducer::Node(read_node.clone()),
                &scope,
                &semantic,
                &value_schema,
                lineage_read.clone(),
                spec::StoragePolicy::ContentAddressed,
            ),
            cell_spec(
                managed_cell.clone(),
                spec::CellProducer::Node(managed_node.clone()),
                &scope,
                &semantic,
                &value_schema,
                lineage_managed.clone(),
                spec::StoragePolicy::ContentAddressed,
            ),
            cell_spec(
                side_effect_cell.clone(),
                spec::CellProducer::Node(side_effect_node.clone()),
                &scope,
                &semantic,
                &value_schema,
                lineage_side_effect.clone(),
                spec::StoragePolicy::ContentAddressed,
            ),
            cell_spec(
                render_cell,
                spec::CellProducer::Node(render_node.clone()),
                &scope,
                &spec::public_output_receipt_semantic_type_id().map_err(display_error)?,
                &spec::public_output_receipt_schema_id().map_err(display_error)?,
                lineage_render.clone(),
                spec::StoragePolicy::PublicOutputArtifact,
            ),
        ],
        value_lineages: vec![
            value_lineage(
                lineage_seed,
                &scope,
                spec::CellProducer::Seed(seed_id),
                Vec::new(),
                None,
                planning.clone(),
                spec::LineageTransformPolicy::Source,
            ),
            value_lineage(
                lineage_pure,
                &scope,
                spec::CellProducer::Node(pure_node),
                vec![cell(0x12)],
                Some(config_ref.digest.clone()),
                planning.clone(),
                spec::LineageTransformPolicy::StateOutput,
            ),
            value_lineage(
                lineage_read,
                &scope,
                spec::CellProducer::Node(read_node),
                vec![pure_cell.clone()],
                Some(config_ref.digest.clone()),
                planning.clone(),
                spec::LineageTransformPolicy::StateOutput,
            ),
            value_lineage(
                lineage_managed,
                &scope,
                spec::CellProducer::Node(managed_node),
                vec![read_cell.clone()],
                Some(config_ref.digest.clone()),
                planning.clone(),
                spec::LineageTransformPolicy::StateOutput,
            ),
            value_lineage(
                lineage_side_effect,
                &scope,
                spec::CellProducer::Node(side_effect_node),
                vec![managed_cell.clone()],
                Some(config_ref.digest.clone()),
                planning.clone(),
                spec::LineageTransformPolicy::StateOutput,
            ),
            value_lineage(
                lineage_render,
                &scope,
                spec::CellProducer::Node(render_node),
                vec![side_effect_cell.clone()],
                Some(config_ref.digest),
                planning,
                spec::LineageTransformPolicy::StateOutput,
            ),
        ],
        planning_lineage: Vec::new(),
        public_outputs,
    })
    .map_err(display_error)?;
    let envelope =
        spec::CertifiedSpecEnvelope::new(typed_spec, spec::TypedExecutionSpecAudit::default())
            .map_err(display_error)?;
    let runtime_spec = CertifiedRuntimeSpec::new(envelope).map_err(display_error)?;
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

struct NodeSpecFixture {
    node_id: NodeId,
    descriptor_id: DescriptorId,
    scope_id: ScopeId,
    state_key: &'static str,
    state_kind: StateKind,
    state_version: StateVersion,
    effect_kind: EffectKind,
    config_ref: spec::ConfigRef,
    input_schema: SchemaId,
    input_cell: CellId,
    input_schema_for_cell: SchemaId,
    input_lineage: spec::ValueLineageRef,
    output_cell: CellId,
    caps: CapabilitySetDescriptor,
    predecessors: Vec<NodeId>,
    adapter_bindings: Vec<spec::AdapterBinding>,
    planning: spec::PlanningLineage,
}

fn node_spec(fixture: NodeSpecFixture) -> Result<spec::NodeSpec, String> {
    Ok(spec::NodeSpec {
        node_id: fixture.node_id,
        stable_key: spec::StableAuthorKey::new(fixture.state_key).map_err(display_error)?,
        scope_id: fixture.scope_id,
        state_kind: fixture.state_kind,
        state_version: fixture.state_version,
        descriptor_id: fixture.descriptor_id,
        config_ref: fixture.config_ref,
        input_bindings: spec::InputBindingSpec {
            input_schema_id: fixture.input_schema,
            input_descriptor_id: descriptor(0x90),
            root: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                field_path: spec::PublicFieldPath::new("input").map_err(display_error)?,
                cell_id: fixture.input_cell,
                semantic_type_id: SemanticTypeId::new(
                    "mfm.typed_slice",
                    "value",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    bytes(0x22),
                )
                .map_err(display_error)?,
                schema_id: fixture.input_schema_for_cell,
                required_terminal: spec::RequiredTerminal::ProducedOnly,
                value_lineage: fixture.input_lineage,
            })),
            digest: content(0x91),
        },
        output_cell: fixture.output_cell,
        effect_kind: fixture.effect_kind,
        capability_bindings: fixture.caps,
        adapter_bindings: fixture.adapter_bindings,
        side_effect: None,
        framework: None,
        planning_lineage: fixture.planning,
        deterministic_predecessors: fixture.predecessors,
    })
}

struct RenderNodeSpecFixture {
    node_id: NodeId,
    descriptor_id: DescriptorId,
    scope_id: ScopeId,
    config_ref: spec::ConfigRef,
    public_output_cell: spec::PublicOutputCell,
    public_outputs: spec::PublicOutputSpec,
    render_cell: CellId,
    planning: spec::PlanningLineage,
}

fn render_node_spec(fixture: RenderNodeSpecFixture) -> Result<spec::NodeSpec, String> {
    let render_input_root = spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
        field_path: fixture.public_output_cell.public_field_path.clone(),
        node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: fixture.public_output_cell.public_field_path.clone(),
            cell_id: fixture.public_output_cell.cell_id.clone(),
            semantic_type_id: fixture.public_output_cell.semantic_type_id.clone(),
            schema_id: fixture.public_output_cell.schema_id.clone(),
            required_terminal: fixture.public_output_cell.required_terminal,
            value_lineage: fixture.public_output_cell.value_lineage.clone(),
        })),
    }]);
    let managed_effect = ManagedPlatformWrite::descriptor().map_err(display_error)?;
    let output_spec_digest = fixture.public_outputs.digest().map_err(display_error)?;
    let public_schema_id = fixture.public_outputs.public_schema_id.clone();
    let renderer_descriptor = fixture.public_outputs.renderer_descriptor.clone();
    let required_cells = fixture.public_outputs.outputs.clone();
    Ok(spec::NodeSpec {
        node_id: fixture.node_id.clone(),
        stable_key: spec::StableAuthorKey::new("public-output").map_err(display_error)?,
        scope_id: fixture.scope_id,
        state_kind: StateKind::new(
            "mfm.framework.state",
            "render_public_outputs",
            DigestAlgorithm::Sha256JcsV1,
            bytes(0x92),
        )
        .map_err(display_error)?,
        state_version: StateVersion::new("mfm.framework.state.render_public_outputs.v1")
            .map_err(display_error)?,
        descriptor_id: fixture.descriptor_id,
        config_ref: fixture.config_ref,
        input_bindings: spec::InputBindingSpec {
            input_schema_id: fixture.public_outputs.public_schema_id.clone(),
            input_descriptor_id: descriptor(0x93),
            digest: content_digest_json(input_node_json(&render_input_root))?,
            root: render_input_root,
        },
        output_cell: fixture.render_cell,
        effect_kind: managed_effect.kind,
        capability_bindings: CapabilitySetDescriptor::new(Vec::new()).map_err(display_error)?,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
            spec::PublicOutputRenderNodeSpec {
                public_schema_id,
                output_spec_digest,
                renderer_descriptor,
                required_cells,
            },
        )),
        planning_lineage: fixture.planning,
        deterministic_predecessors: vec![match fixture.public_output_cell.producer {
            spec::CellProducer::Node(node_id) => node_id,
            spec::CellProducer::Seed(_) => {
                return Err("public output render requires node-produced cell".to_owned())
            }
        }],
    })
}

fn state_descriptor(
    node: &spec::NodeSpec,
    descriptor_id: DescriptorId,
    name: &str,
    effect_kind: EffectKind,
    capabilities: CapabilitySetDescriptor,
    runner: &str,
    side_effect_contract_digest: Option<ContentDigest>,
) -> Result<spec::StateDescriptorIdentity, String> {
    Ok(spec::StateDescriptorIdentity {
        descriptor_id,
        name: name.to_owned(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        config_schema_id: node.config_ref.schema_id.clone(),
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        output_schema_id: SchemaId::new(
            "mfm.typed_slice.value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            bytes(0x23),
        )
        .map_err(display_error)?,
        output_semantic_type_id: SemanticTypeId::new(
            "mfm.typed_slice",
            "value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            bytes(0x22),
        )
        .map_err(display_error)?,
        effect_kind,
        effect_class: runner.to_owned(),
        effect_name: runner.to_owned(),
        effect_version: EffectVersion::new("mfm.typed_slice.effect.v1").map_err(display_error)?,
        capabilities,
        runner: runner.to_owned(),
        side_effect_contract_digest,
    })
}

fn render_descriptor_identity(
    node: &spec::NodeSpec,
    descriptor_id: DescriptorId,
    public_schema: SchemaId,
    config_schema: SchemaId,
) -> Result<spec::StateDescriptorIdentity, String> {
    let managed_effect = ManagedPlatformWrite::descriptor().map_err(display_error)?;
    Ok(spec::StateDescriptorIdentity {
        descriptor_id,
        name: "mfm.framework.render_public_outputs".to_owned(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        config_schema_id: config_schema,
        input_schema_id: public_schema,
        output_schema_id: spec::public_output_receipt_schema_id().map_err(display_error)?,
        output_semantic_type_id: spec::public_output_receipt_semantic_type_id()
            .map_err(display_error)?,
        effect_kind: managed_effect.kind,
        effect_class: managed_effect.class.as_str().to_owned(),
        effect_name: managed_effect.name.to_owned(),
        effect_version: managed_effect.version,
        capabilities: CapabilitySetDescriptor::new(Vec::new()).map_err(display_error)?,
        runner: "managed_platform_write".to_owned(),
        side_effect_contract_digest: None,
    })
}

fn cell_spec(
    cell_id: CellId,
    producer: spec::CellProducer,
    scope_id: &ScopeId,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
    value_lineage: spec::ValueLineageRef,
    storage_policy: spec::StoragePolicy,
) -> spec::CellSpec {
    spec::CellSpec {
        cell_id,
        producer,
        scope_id: scope_id.clone(),
        semantic_type_id: semantic_type_id.clone(),
        schema_id: schema_id.clone(),
        value_lineage,
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy,
        redaction_policy: spec::RedactionPolicy::Public,
    }
}

fn value_lineage(
    lineage_ref: spec::ValueLineageRef,
    scope_id: &ScopeId,
    producer: spec::CellProducer,
    input_cells: Vec<CellId>,
    config_ref_digest: Option<ContentDigest>,
    planning_lineage: spec::PlanningLineage,
    transform_policy: spec::LineageTransformPolicy,
) -> spec::ValueLineage {
    spec::ValueLineage {
        lineage_ref,
        scope_id: scope_id.clone(),
        producer,
        input_cells,
        config_ref_digest,
        planning_lineage,
        domain_keys: Vec::new(),
        transform_policy,
    }
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
        "side-effect",
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
        "side-effect",
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

fn input_node_json(node: &spec::InputBindingNodeSpec) -> serde_json::Value {
    match node {
        spec::InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
        spec::InputBindingNodeSpec::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": match cell.required_terminal {
                spec::RequiredTerminal::ProducedOnly => "produced_only",
                spec::RequiredTerminal::MaybeSkipped => "maybe_skipped",
            },
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.lineage_digest.as_str(),
        }),
        spec::InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        spec::InputBindingNodeSpec::Struct(fields) => serde_json::json!({
            "fields": fields.iter().map(|field| {
                serde_json::json!({
                    "field_path": field.field_path.as_str(),
                    "node": input_node_json(&field.node),
                })
            }).collect::<Vec<_>>(),
            "kind": "struct",
        }),
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering_json(*ordering),
        }),
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering_json(*ordering),
        }),
    }
}

fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
        spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
    }
}

fn stable_domain_key_ref_json(key: &spec::StableDomainKeyRef) -> serde_json::Value {
    serde_json::json!({
        "content_digest": key.content_digest.as_str(),
        "schema_id": key.schema_id.as_str(),
    })
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

fn descriptor(byte: u8) -> DescriptorId {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn node(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn cell(byte: u8) -> CellId {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn scope(byte: u8) -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn seed_id(byte: u8) -> SeedId {
    SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
}

fn lineage(byte: u8) -> spec::ValueLineageRef {
    spec::ValueLineageRef {
        lineage_digest: content(byte),
    }
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
