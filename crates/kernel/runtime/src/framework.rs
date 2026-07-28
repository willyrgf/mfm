use std::collections::BTreeSet;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::{verify_artifact_bytes, StagedArtifact, StagedRetentionRefs};
use crate::history::committed_input_artifact;
use crate::invocation::ErasedRunCtx;
use crate::runners::{
    ErasedNodeRunner, ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput,
    RunnerEventPayload,
};
use crate::spec_authority::CurrentSpecRead;
use crate::{
    canonical_json, content_digest_json, executable_identity_json, ExecutableIdentityTemplate,
    Result, RuntimeError,
};

pub(crate) fn framework_public_output_binding(
    executable_identities: &ExecutableIdentityTemplate,
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a public-output render node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.render_public_outputs" {
        return Err(RuntimeError::RunnerBinding(format!(
            "public-output render node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        executable_identities.executable(factory_id),
        Arc::new(FrameworkPublicOutputRunner),
    )
}

pub(crate) fn framework_bridge_binding(
    executable_identities: &ExecutableIdentityTemplate,
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::Bridge(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a same-value bridge node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.bridge_same_value" {
        return Err(RuntimeError::RunnerBinding(format!(
            "bridge node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        executable_identities.executable(factory_id),
        Arc::new(FrameworkBridgeRunner),
    )
}

struct FrameworkBridgeRunner;

impl ErasedNodeRunner for FrameworkBridgeRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { bridge_same_value(ctx) })
    }
}

fn bridge_same_value(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::Bridge(bridge)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a same-value bridge node",
            ctx.node().node_id
        )));
    };
    if bridge.target_cell_id != ctx.node().output_cell {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "bridge node {} output cell does not match its framework metadata",
            ctx.node().node_id
        )));
    }
    let input = match &ctx.inputs().root {
        crate::MaterializedInputNode::Cell(cell) => cell,
        _ => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "bridge node {} requires one materialized source cell",
                ctx.node().node_id
            )))
        }
    };
    if input.cell_id != bridge.source_cell_id {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "bridge node {} input cell does not match its source cell",
            ctx.node().node_id
        )));
    }
    let (artifact_id, content_digest, evidence_hash) = match &input.terminal {
        crate::MaterializedCellTerminal::Seed {
            artifact_id,
            content_digest,
            evidence_hash,
            ..
        }
        | crate::MaterializedCellTerminal::Produced {
            artifact_id,
            content_digest,
            evidence_hash,
            ..
        } => (artifact_id, content_digest, evidence_hash),
        crate::MaterializedCellTerminal::Skipped { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(
                "same-value bridge cannot copy a skipped input cell".to_owned(),
            ))
        }
    };
    let role = match &input.terminal {
        crate::MaterializedCellTerminal::Seed { .. } => events::ArtifactRole::SeedInput,
        crate::MaterializedCellTerminal::Produced { .. } => events::ArtifactRole::StateOutput,
        crate::MaterializedCellTerminal::Skipped { .. } => unreachable!("rejected above"),
    };
    let source = committed_input_artifact(
        ctx.lifecycle(),
        &input.cell_id,
        artifact_id,
        evidence_hash,
        role,
    )?;
    let bytes = source.bytes();
    let source_evidence = source.evidence();
    if source_evidence.digest != *content_digest
        || source_evidence.schema_id.as_ref() != Some(&input.schema_id)
        || source_evidence.semantic_type_id.as_ref() != Some(&input.semantic_type_id)
        || source_evidence.evidence_hash()? != *evidence_hash
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "bridge input artifact {} evidence does not match its materialized cell",
            artifact_id
        )));
    }
    verify_artifact_bytes(bytes, source_evidence)?;

    let output_evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: content_digest.clone(),
        byte_len: source_evidence.byte_len,
        media_type: source_evidence.media_type.clone(),
        schema_id: Some(ctx.output_cell().schema_id.clone()),
        semantic_type_id: Some(ctx.output_cell().semantic_type_id.clone()),
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let staged = StagedArtifact::inline_attempt_artifact(&ctx, bytes.to_vec(), output_evidence)?;
    let output_evidence = staged.evidence().clone();
    let evidence_hash = output_evidence.evidence_hash()?;
    let produced = events::CellProduced {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        cell_id: ctx.node().output_cell.clone(),
        scope_id: ctx.output_cell().scope_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
        schema_id: ctx.output_cell().schema_id.clone(),
        value_lineage: ctx.output_cell().value_lineage.clone(),
        context: ctx.output_cell().context.clone(),
        artifact_id: output_evidence.artifact_id,
        content_digest: output_evidence.digest,
        evidence_hash,
        producer_state_kind: Some(ctx.node().state_kind.clone()),
        producer_state_version: Some(ctx.node().state_version.clone()),
    };
    Ok(ErasedRunnerOutput::from_parts(
        vec![staged],
        Vec::new(),
        vec![RunnerEventPayload::CellProduced(produced)],
    ))
}

struct FrameworkPublicOutputRunner;

impl ErasedNodeRunner for FrameworkPublicOutputRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { render_public_output(ctx) })
    }
}

pub(crate) fn framework_retention_manifest_binding(
    executable_identities: &ExecutableIdentityTemplate,
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a retention-manifest framework node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.project_retention_manifest" {
        return Err(RuntimeError::RunnerBinding(format!(
            "retention-manifest node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        executable_identities.executable(factory_id),
        Arc::new(FrameworkRetentionManifestRunner),
    )
}

struct FrameworkRetentionManifestRunner;

impl ErasedNodeRunner for FrameworkRetentionManifestRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { project_retention_manifest(ctx) })
    }
}

pub(crate) fn framework_complete_run_binding(
    executable_identities: &ExecutableIdentityTemplate,
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::CompleteRun(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a complete-run framework node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.complete_run" {
        return Err(RuntimeError::RunnerBinding(format!(
            "complete-run node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        executable_identities.executable(factory_id),
        Arc::new(FrameworkCompleteRunRunner),
    )
}

pub(crate) fn framework_resolve_saga_terminal_binding(
    executable_identities: &ExecutableIdentityTemplate,
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a resolve-saga-terminal framework node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.resolve_saga_terminal" {
        return Err(RuntimeError::RunnerBinding(format!(
            "resolve-saga-terminal node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        executable_identities.executable(factory_id),
        Arc::new(FrameworkResolveSagaTerminalRunner),
    )
}

struct FrameworkResolveSagaTerminalRunner;

impl ErasedNodeRunner for FrameworkResolveSagaTerminalRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { resolve_saga_terminal_framework(ctx) })
    }
}

struct FrameworkCompleteRunRunner;

impl ErasedNodeRunner for FrameworkCompleteRunRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { complete_run_framework(ctx) })
    }
}

fn render_public_output(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a public-output render node",
            ctx.node().node_id
        )));
    };
    let mut cells = Vec::with_capacity(render.required_cells.len());
    for required in &render.required_cells {
        let Some(produced) = ctx
            .lifecycle()
            .cell(&required.cell_id)
            .and_then(|cell| cell.produced())
        else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "public-output render node {} required incomplete cell {}",
                ctx.node().node_id,
                required.cell_id
            )));
        };
        cells.push(events::NamedTypedCellRef {
            public_field_path: required.public_field_path.clone(),
            cell_id: required.cell_id.clone(),
            producer: required.producer.clone(),
            scope_id: required.scope_id.clone(),
            semantic_type_id: required.semantic_type_id.clone(),
            schema_id: required.schema_id.clone(),
            value_lineage: required.value_lineage.clone(),
            content_digest: produced.content_digest().clone(),
            artifact_id: produced.artifact_id().clone(),
            evidence_hash: produced.evidence_hash().clone(),
        });
    }
    let rendered_digest = public_output_rendered_digest(render, &cells)?;
    let rendered_artifact_id = None;
    let rendered_artifact_evidence_hash = None;
    let receipt_bytes = public_output_receipt_json(
        render,
        &cells,
        &rendered_digest,
        rendered_artifact_id.as_ref(),
    )?;
    let (receipt_artifact, receipt_cell_produced) =
        stage_framework_state_output(&ctx, receipt_bytes)?;
    let receipt_retention_ref = receipt_artifact.evidence().retention_ref()?;
    Ok(ErasedRunnerOutput::from_parts(
        vec![receipt_artifact],
        vec![StagedRetentionRefs::framework_public_output(vec![
            receipt_retention_ref,
        ])],
        vec![
            receipt_cell_produced,
            RunnerEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                receipt_cell_id: ctx.node().output_cell.clone(),
                public_schema_id: render.public_schema_id.clone(),
                output_spec_digest: render.output_spec_digest.clone(),
                cells,
                rendered_digest,
                rendered_artifact_id,
                rendered_artifact_evidence_hash,
                renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
            }),
        ],
    ))
}

fn project_retention_manifest(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a retention-manifest framework node",
            ctx.node().node_id
        )));
    };
    let manifest = build_retention_manifest_artifact(&ctx.runtime_spec(), ctx.lifecycle())?;
    let manifest_artifact = StagedArtifact::inline_retention_manifest_artifact(
        &ctx,
        manifest.bytes.to_vec(),
        manifest.evidence.clone(),
    )?;
    let receipt_bytes =
        retention_manifest_receipt_json(&manifest, current_sequence(ctx.lifecycle()))?;
    let (receipt_artifact, receipt_cell_produced) =
        stage_framework_state_output(&ctx, receipt_bytes)?;
    let receipt_retention_ref = receipt_artifact.evidence().retention_ref()?;
    Ok(ErasedRunnerOutput::from_parts(
        vec![manifest_artifact, receipt_artifact],
        vec![StagedRetentionRefs::runtime_evidence(vec![
            receipt_retention_ref,
        ])],
        vec![receipt_cell_produced],
    ))
}

fn complete_run_framework(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::CompleteRun(_)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a complete-run framework node",
            ctx.node().node_id
        )));
    };
    let completion = run_completion_evidence(&ctx.runtime_spec(), ctx.lifecycle())?;
    let retention_manifest = current_retention_manifest(ctx.lifecycle())?;
    let receipt_bytes = complete_run_receipt_json(
        &completion,
        &retention_manifest,
        current_sequence(ctx.lifecycle()),
    )?;
    let (receipt_artifact, receipt_cell_produced) =
        stage_framework_state_output(&ctx, receipt_bytes)?;
    Ok(ErasedRunnerOutput::from_parts(
        vec![receipt_artifact],
        Vec::new(),
        vec![receipt_cell_produced],
    ))
}

fn resolve_saga_terminal_framework(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a resolve-saga-terminal framework node",
            ctx.node().node_id
        )));
    };
    let outcome = saga_terminal_completion_outcome(&ctx.runtime_spec(), ctx.lifecycle())?;
    let receipt_bytes = resolve_saga_terminal_receipt_json(
        &ctx.runtime_spec(),
        &outcome,
        current_sequence(ctx.lifecycle()),
    )?;
    let (receipt_artifact, receipt_cell_produced) =
        stage_framework_state_output(&ctx, receipt_bytes)?;
    Ok(ErasedRunnerOutput::from_parts(
        vec![receipt_artifact],
        Vec::new(),
        vec![receipt_cell_produced],
    ))
}

fn stage_framework_state_output(
    ctx: &ErasedRunCtx<'_>,
    receipt_bytes: PlainCanonicalJsonBytes,
) -> Result<(StagedArtifact, RunnerEventPayload)> {
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let receipt_artifact = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(ctx.output_cell().schema_id.clone()),
        semantic_type_id: Some(ctx.output_cell().semantic_type_id.clone()),
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let evidence_hash = receipt_artifact.evidence_hash()?;
    let receipt_artifact =
        StagedArtifact::inline_attempt_artifact(ctx, receipt_bytes.to_vec(), receipt_artifact)?;
    Ok((
        receipt_artifact,
        RunnerEventPayload::CellProduced(events::CellProduced {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            cell_id: ctx.node().output_cell.clone(),
            scope_id: ctx.output_cell().scope_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
            schema_id: ctx.output_cell().schema_id.clone(),
            value_lineage: ctx.output_cell().value_lineage.clone(),
            context: ctx.output_cell().context.clone(),
            artifact_id: receipt_artifact_id,
            content_digest: receipt_digest,
            evidence_hash,
            producer_state_kind: Some(ctx.node().state_kind.clone()),
            producer_state_version: Some(ctx.node().state_version.clone()),
        }),
    ))
}

pub(crate) fn public_output_rendered_digest(
    render: &spec::PublicOutputRenderNodeSpec,
    cells: &[events::NamedTypedCellRef],
) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "cells": cells.iter().map(public_output_cell_json).collect::<Vec<_>>(),
        "output_spec_digest": render.output_spec_digest.as_str(),
        "public_schema_id": render.public_schema_id.as_str(),
        "renderer_descriptor_id": render.renderer_descriptor.descriptor_id.as_str(),
    }))
}

fn current_sequence(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Option<u64> {
    let mut sequence = None;
    let _ = lifecycle.visit_records(|record| {
        sequence = Some(record.sequence().as_u64());
        std::ops::ControlFlow::<()>::Continue(())
    });
    sequence
}

pub(crate) fn retention_manifest_receipt_json(
    manifest: &RetentionManifestArtifact,
    pre_projection_sequence: Option<u64>,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "manifest_artifact_id": manifest.evidence.artifact_id.as_str(),
        "manifest_digest": manifest.evidence.digest.as_str(),
        "manifest_seq": manifest.manifest_seq,
        "pre_projection_stream_seq": pre_projection_sequence,
        "previous_manifest_digest": manifest.previous_manifest_digest.as_ref().map(ContentDigest::as_str),
    }))
}

pub(crate) fn complete_run_receipt_json(
    completion: &events::PublicOutputCompletionEvidence,
    retention_manifest: &store::current_lifecycle::CurrentRetentionManifestRef<'_>,
    pre_completion_sequence: Option<u64>,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "public_output_event_id": completion.public_output_event_id.as_str(),
        "public_output_schema_id": completion.public_output_schema_id.as_str(),
        "retention_manifest_artifact_id": retention_manifest.artifact_id().as_str(),
        "retention_manifest_digest": retention_manifest.digest().as_str(),
        "retention_manifest_seq": retention_manifest.sequence(),
        "pre_completion_stream_seq": pre_completion_sequence,
    }))
}

pub(crate) fn resolve_saga_terminal_receipt_json<S>(
    runtime_spec: &S,
    outcome: &events::RunCompletionOutcome,
    pre_resolution_sequence: Option<u64>,
) -> Result<PlainCanonicalJsonBytes>
where
    S: CurrentSpecRead + ?Sized,
{
    canonical_json(serde_json::json!({
        "public_output_schema_id": runtime_spec.spec().public_outputs.public_schema_id.as_str(),
        "terminal_outcome": outcome.kind(),
        "pre_resolution_stream_seq": pre_resolution_sequence,
    }))
}

pub(crate) fn saga_terminal_completion_outcome<S>(
    runtime_spec: &S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<events::RunCompletionOutcome>
where
    S: CurrentSpecRead + ?Sized,
{
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    lifecycle
        .saga_terminal_completion_outcome(&runtime_spec.spec().saga, &terminal_policies)
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))
}

pub(crate) fn run_completion_evidence<S>(
    runtime_spec: &S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<events::PublicOutputCompletionEvidence>
where
    S: CurrentSpecRead + ?Sized,
{
    let public_schema_id = runtime_spec.spec().public_outputs.public_schema_id.clone();
    match lifecycle.public_output(&public_schema_id) {
        Some(output) if output.produced().is_some() => Ok(events::PublicOutputCompletionEvidence {
            public_output_schema_id: public_schema_id,
            public_output_event_id: output.event_id().clone(),
        }),
        Some(_) | None => Err(RuntimeError::InvalidRunStream(
            "run completion requires current public output evidence".to_owned(),
        )),
    }
}

pub(crate) fn current_retention_manifest<'a>(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'a>,
) -> Result<store::current_lifecycle::CurrentRetentionManifestRef<'a>> {
    lifecycle
        .retention()
        .and_then(|retention| retention.current_manifest())
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "run completion requires current retention manifest evidence".to_owned(),
            )
        })
}

pub(crate) fn framework_run_completed_payload<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<Option<events::KernelEventPayload>>
where
    S: CurrentSpecRead + ?Sized,
{
    let run_id = lifecycle.admission()?.run_id().clone();
    let outcome = match &node.framework {
        Some(spec::FrameworkNodeSpec::CompleteRun(complete)) => {
            let certified_node = certified_complete_run_node(runtime_spec)?;
            if certified_node.node_id != node.node_id {
                return Err(RuntimeError::InvalidSpec(format!(
                    "complete-run node {} is not the certified completion lifecycle node",
                    node.node_id
                )));
            }
            if complete.public_schema_id != runtime_spec.spec().public_outputs.public_schema_id {
                return Err(RuntimeError::InvalidSpec(format!(
                    "complete-run node {} references public schema {} outside certified public outputs",
                    node.node_id, complete.public_schema_id
                )));
            }
            let completion = run_completion_evidence(runtime_spec, lifecycle)?;
            current_retention_manifest(lifecycle)?;
            events::RunCompletionOutcome::Completed(Box::new(completion))
        }
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(resolve)) => {
            let certified_node = certified_resolve_saga_terminal_node(runtime_spec)?;
            if certified_node.node_id != node.node_id {
                return Err(RuntimeError::InvalidSpec(format!(
                    "resolve-saga-terminal node {} is not the certified saga terminal lifecycle node",
                    node.node_id
                )));
            }
            if resolve.public_schema_id != runtime_spec.spec().public_outputs.public_schema_id {
                return Err(RuntimeError::InvalidSpec(format!(
                    "resolve-saga-terminal node {} references public schema {} outside certified public outputs",
                    node.node_id, resolve.public_schema_id
                )));
            }
            saga_terminal_completion_outcome(runtime_spec, lifecycle)?
        }
        _ => return Ok(None),
    };
    Ok(Some(events::KernelEventPayload::RunCompleted(
        events::RunCompleted {
            run_id,
            spec_hash: runtime_spec.spec_hash().clone(),
            outcome,
        },
    )))
}

fn public_output_receipt_json(
    render: &spec::PublicOutputRenderNodeSpec,
    cells: &[events::NamedTypedCellRef],
    rendered_digest: &ContentDigest,
    rendered_artifact_id: Option<&ArtifactId>,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "cells": cells.iter().map(public_output_cell_json).collect::<Vec<_>>(),
        "output_spec_digest": render.output_spec_digest.as_str(),
        "public_schema_id": render.public_schema_id.as_str(),
        "rendered_artifact_id": rendered_artifact_id.map(ArtifactId::as_str),
        "rendered_digest": rendered_digest.as_str(),
        "renderer_descriptor_id": render.renderer_descriptor.descriptor_id.as_str(),
    }))
}

fn public_output_cell_json(cell: &events::NamedTypedCellRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": cell.artifact_id.as_str(),
        "cell_id": cell.cell_id.as_str(),
        "content_digest": cell.content_digest.as_str(),
        "producer": cell_producer_json(&cell.producer),
        "public_field_path": cell.public_field_path.as_str(),
        "schema_id": cell.schema_id.as_str(),
        "scope_id": cell.scope_id.as_str(),
        "semantic_type_id": cell.semantic_type_id.as_str(),
        "value_lineage": cell.value_lineage.lineage_digest.as_str(),
    })
}

fn retention_manifest_json<S>(
    runtime_spec: &S,
    run_admitted: &store::current_lifecycle::CurrentAdmissionRef<'_>,
    manifest_seq: u64,
    previous_manifest_digest: Option<&ContentDigest>,
    retained_refs: &[&events::RetentionRef],
    event_schema_ids: &[String],
) -> Result<PlainCanonicalJsonBytes>
where
    S: CurrentSpecRead + ?Sized,
{
    let spec_canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let spec_digest = spec_canonical.content_digest();
    let certificate_canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let retained_by_role = retained_refs_by_role(retained_refs);
    canonical_json(serde_json::json!({
        "adapter_executables": run_admitted.adapter_executables().map(executable_identity_json).collect::<Vec<_>>(),
        "canonicalizer_identity": run_admitted.canonicalizer_identity().as_str(),
        "certificate_artifact": {
            "artifact_id": run_admitted.certificate_artifact().artifact_id.as_str(),
            "byte_len": certificate_canonical.as_bytes().len(),
            "content_digest": run_admitted.certificate_artifact().content_digest.as_str(),
            "media_type": run_admitted.certificate_artifact().media_type.as_str(),
        },
        "config_artifacts": runtime_spec.spec().config_refs.iter().map(config_artifact_json).collect::<Vec<_>>(),
        "descriptor_digests": runtime_spec.spec().descriptor_identities.iter().map(descriptor_digest_json).collect::<Vec<_>>(),
        "descriptor_identities": runtime_spec.spec().descriptor_identities.iter().map(descriptor_identity_json).collect::<Vec<_>>(),
        "entry_point": entry_point_launch_evidence_json(run_admitted.entry_point()),
        "event_schema_ids": event_schema_ids,
        "manifest_seq": manifest_seq,
        "previous_manifest_digest": previous_manifest_digest.map(ContentDigest::as_str),
        "public_output_artifacts": retained_by_role.public_output_artifacts,
        "receipt_artifacts": retained_by_role.receipt_artifacts,
        "confirmation_artifacts": retained_by_role.confirmation_artifacts,
        "retained_refs": retained_refs.iter().map(|retention_ref| retention_ref_json(retention_ref)).collect::<Vec<_>>(),
        "run_id": run_admitted.run_id().as_str(),
        "runner_executables": run_admitted.runner_executables().map(executable_identity_json).collect::<Vec<_>>(),
        "spec_artifact": {
            "artifact_id": run_admitted.spec_artifact().artifact_id.as_str(),
            "byte_len": spec_canonical.as_bytes().len(),
            "content_digest": spec_digest.as_str(),
            "media_type": run_admitted.spec_artifact().media_type.as_str(),
        },
        "spec_hash": runtime_spec.spec_hash().as_str(),
        "value_artifacts": retained_by_role.value_artifacts,
    }))
}

fn entry_point_launch_evidence_json(
    evidence: &events::EntryPointLaunchEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "configured_targets": evidence
            .configured_targets
            .iter()
            .map(|source| {
                serde_json::json!({
                    "digest": source.digest.as_str(),
                    "target": source.target.as_str(),
                    "schema_id": source.schema_id.as_str(),
                })
            })
            .collect::<Vec<_>>(),
        "entry_point_id": evidence.entry_point_id.as_str(),
    })
}

struct RetainedRefsByRole {
    value_artifacts: Vec<String>,
    receipt_artifacts: Vec<String>,
    confirmation_artifacts: Vec<String>,
    public_output_artifacts: Vec<String>,
}

fn retained_refs_by_role(retained_refs: &[&events::RetentionRef]) -> RetainedRefsByRole {
    let mut value_artifacts = Vec::new();
    let mut receipt_artifacts = Vec::new();
    let mut confirmation_artifacts = Vec::new();
    let mut public_output_artifacts = Vec::new();
    for retention_ref in retained_refs {
        match retention_ref.role.contract().retention {
            events::ArtifactRetentionClass::ValueArtifacts => {
                value_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRetentionClass::ReceiptArtifacts => {
                receipt_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRetentionClass::ConfirmationArtifacts => {
                confirmation_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRetentionClass::PublicOutputArtifacts => {
                public_output_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRetentionClass::FrameworkIgnored => {}
        }
    }
    RetainedRefsByRole {
        value_artifacts,
        receipt_artifacts,
        confirmation_artifacts,
        public_output_artifacts,
    }
}

fn config_artifact_json(config: &spec::ConfigRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": config.artifact_id.as_str(),
        "byte_len": config.byte_len,
        "content_digest": config.digest.as_str(),
        "media_type": config.media_type.as_str(),
        "schema_id": config.schema_id.as_str(),
    })
}

fn descriptor_identity_json(identity: &spec::DescriptorIdentity) -> serde_json::Value {
    match identity {
        spec::DescriptorIdentity::State(identity) => serde_json::json!({
            "descriptor_family": "state",
            "descriptor_id": identity.descriptor_id.as_str(),
            "name": identity.name.as_str(),
            "state_kind": identity.state_kind.as_str(),
            "state_version": identity.state_version.as_str(),
        }),
        spec::DescriptorIdentity::Operation(identity) => serde_json::json!({
            "descriptor_family": "operation",
            "descriptor_id": identity.descriptor_id.as_str(),
            "name": identity.name.as_str(),
            "operation_kind": identity.operation_kind.as_str(),
            "operation_version": identity.operation_version.as_str(),
        }),
        spec::DescriptorIdentity::Renderer(identity) => serde_json::json!({
            "descriptor_family": "renderer",
            "descriptor_id": identity.descriptor_id.as_str(),
            "renderer_kind": identity.renderer_kind.as_str(),
            "renderer_version": identity.renderer_version.as_str(),
        }),
    }
}

fn descriptor_digest_json(identity: &spec::DescriptorIdentity) -> serde_json::Value {
    let descriptor_id = match identity {
        spec::DescriptorIdentity::State(identity) => &identity.descriptor_id,
        spec::DescriptorIdentity::Operation(identity) => &identity.descriptor_id,
        spec::DescriptorIdentity::Renderer(identity) => &identity.descriptor_id,
    };
    serde_json::json!({
        "descriptor_id": descriptor_id.as_str(),
        "digest": ContentDigest::from_digest(descriptor_id.algorithm(), *descriptor_id.digest()).as_str(),
    })
}

fn retention_ref_json(retention_ref: &events::RetentionRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": retention_ref.artifact_id.as_str(),
        "content_digest": retention_ref.content_digest.as_str(),
        "evidence_hash": retention_ref.evidence_hash.as_str(),
        "role": retention_ref.role.as_str(),
    })
}

pub(crate) fn retention_reason_str(reason: events::RetentionReason) -> &'static str {
    match reason {
        events::RetentionReason::RunAdmitted => "run_admitted",
        events::RetentionReason::RuntimeEvidence => "runtime_evidence",
        events::RetentionReason::PublicOutput => "public_output",
        events::RetentionReason::ManifestProjection => "manifest_projection",
    }
}

fn cell_producer_json(producer: &spec::CellProducer) -> serde_json::Value {
    match producer {
        spec::CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
        spec::CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetentionManifestArtifact {
    pub(crate) bytes: PlainCanonicalJsonBytes,
    pub(crate) evidence: store::ArtifactEvidenceRef,
    pub(crate) manifest_seq: u64,
    pub(crate) previous_manifest_digest: Option<ContentDigest>,
}

pub(crate) fn public_output_is_produced<S>(
    runtime_spec: &S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> bool
where
    S: CurrentSpecRead + ?Sized,
{
    let public_schema_id = &runtime_spec.spec().public_outputs.public_schema_id;
    lifecycle
        .public_output(public_schema_id)
        .is_some_and(|output| output.produced().is_some())
}

pub(crate) fn build_retention_manifest_artifact<S>(
    runtime_spec: &S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<RetentionManifestArtifact>
where
    S: CurrentSpecRead + ?Sized,
{
    let run_admitted = lifecycle.admission()?;
    if run_admitted.spec_hash() != runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest run-start evidence does not match certified run".to_owned(),
        ));
    }
    let retention = lifecycle.retention();
    let current_manifest = retention
        .as_ref()
        .and_then(store::current_lifecycle::CurrentRetentionRef::current_manifest);
    let manifest_seq = current_manifest
        .as_ref()
        .map(|manifest| {
            manifest.sequence().checked_add(1).ok_or_else(|| {
                RuntimeError::InvalidRunStream("retention manifest sequence overflow".to_owned())
            })
        })
        .transpose()?
        .unwrap_or(1);
    let previous_manifest_digest = current_manifest.map(|manifest| manifest.digest().clone());
    let mut retained_refs = Vec::new();
    if let Some(retention) = &retention {
        let _ = retention.visit_references(|reference| {
            retained_refs.push(reference);
            std::ops::ControlFlow::<()>::Continue(())
        });
    }
    let mut event_schema_ids = BTreeSet::new();
    let _ = lifecycle.visit_records(|record| {
        event_schema_ids.insert(record.event_schema_id().as_str().to_owned());
        std::ops::ControlFlow::<()>::Continue(())
    });
    let event_schema_ids = event_schema_ids.into_iter().collect::<Vec<_>>();
    let manifest_json = retention_manifest_json(
        runtime_spec,
        &run_admitted,
        manifest_seq,
        previous_manifest_digest.as_ref(),
        &retained_refs,
        &event_schema_ids,
    )?;
    let digest = manifest_json.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let byte_len = manifest_json.as_bytes().len() as u64;
    Ok(RetentionManifestArtifact {
        bytes: manifest_json,
        evidence: store::ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len,
            media_type: spec::MediaType::new(
                "application/vnd.mfm.retention-manifest+json;version=1",
            )?,
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::RetentionManifest,
        },
        manifest_seq,
        previous_manifest_digest,
    })
}

#[cfg(test)]
pub(crate) fn certified_retention_manifest_node<S>(runtime_spec: &S) -> Result<&spec::NodeSpec>
where
    S: CurrentSpecRead + ?Sized,
{
    certified_framework_node(
        runtime_spec,
        |framework| {
            matches!(
                framework,
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
            )
        },
        "multiple certified retention manifest framework nodes",
        "retention manifest projection lacks a certified framework retention node",
    )
}

pub(crate) fn certified_complete_run_node<S>(runtime_spec: &S) -> Result<&spec::NodeSpec>
where
    S: CurrentSpecRead + ?Sized,
{
    certified_framework_node(
        runtime_spec,
        |framework| matches!(framework, spec::FrameworkNodeSpec::CompleteRun(_)),
        "multiple certified completion framework nodes",
        "RunCompleted lacks a certified CompleteRun framework node",
    )
}

pub(crate) fn certified_resolve_saga_terminal_node<S>(runtime_spec: &S) -> Result<&spec::NodeSpec>
where
    S: CurrentSpecRead + ?Sized,
{
    certified_framework_node(
        runtime_spec,
        |framework| matches!(framework, spec::FrameworkNodeSpec::ResolveSagaTerminal(_)),
        "multiple certified resolve-saga-terminal framework nodes",
        "RunCompleted lacks a certified ResolveSagaTerminal framework node",
    )
}

fn certified_framework_node<'a, S>(
    runtime_spec: &'a S,
    matches_framework: impl Fn(&spec::FrameworkNodeSpec) -> bool,
    duplicate_message: &'static str,
    missing_message: &'static str,
) -> Result<&'a spec::NodeSpec>
where
    S: CurrentSpecRead + ?Sized,
{
    let mut framework_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if node.framework.as_ref().is_some_and(&matches_framework)
            && framework_node.replace(node).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(duplicate_message.to_owned()));
        }
    }
    framework_node.ok_or_else(|| RuntimeError::InvalidRunStream(missing_message.to_owned()))
}
