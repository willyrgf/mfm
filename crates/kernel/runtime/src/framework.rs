use std::collections::BTreeSet;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, ContentDigest, NodeId, RunId, SchemaId, SemanticTypeId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::{artifact_role_name, StagedArtifact, StagedRetentionRefs};
use crate::invocation::ErasedRunCtx;
use crate::runners::{
    ErasedNodeRunner, ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput,
    RunnerEventPayload,
};
use crate::{
    canonical_json, content_digest_json, retention_ref_for_artifact, validate_public_output,
    CertifiedRuntimeSpec, Result, RuntimeError,
};

pub(crate) fn framework_bootstrap_run_binding(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::BootstrapRun(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a bootstrap framework node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.bootstrap_run" {
        return Err(RuntimeError::RunnerBinding(format!(
            "bootstrap node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        framework_bootstrap_run_executable(factory_id)?,
        Arc::new(FrameworkBootstrapRunner),
    )
}

fn framework_bootstrap_run_executable(
    factory_id: events::RunnerFactoryId,
) -> Result<events::ExecutableIdentity> {
    let package_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "runner": "framework_bootstrap_run",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    let binary_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "factory_id": factory_id.as_str(),
        "runner": "framework_bootstrap_run",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-runtime-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-runtime")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: package_digest,
        binary_digest,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct FrameworkBootstrapRunner;

impl ErasedNodeRunner for FrameworkBootstrapRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            Err(RuntimeError::InvalidRunnerOutput(format!(
                "bootstrap node {} must execute through genesis middleware",
                ctx.node().node_id
            )))
        })
    }
}

pub(crate) fn framework_public_output_binding(
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
        framework_public_output_executable(factory_id)?,
        Arc::new(FrameworkPublicOutputRunner),
    )
}

fn framework_public_output_executable(
    factory_id: events::RunnerFactoryId,
) -> Result<events::ExecutableIdentity> {
    let package_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "runner": "framework_public_output",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    let binary_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "factory_id": factory_id.as_str(),
        "runner": "framework_public_output",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-runtime-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-runtime")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: package_digest,
        binary_digest,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct FrameworkPublicOutputRunner;

impl ErasedNodeRunner for FrameworkPublicOutputRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { render_public_output(ctx) })
    }
}

pub(crate) fn framework_retention_manifest_binding(
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
        framework_retention_manifest_executable(factory_id)?,
        Arc::new(FrameworkRetentionManifestRunner),
    )
}

fn framework_retention_manifest_executable(
    factory_id: events::RunnerFactoryId,
) -> Result<events::ExecutableIdentity> {
    let package_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "runner": "framework_retention_manifest",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    let binary_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "factory_id": factory_id.as_str(),
        "runner": "framework_retention_manifest",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-runtime-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-runtime")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: package_digest,
        binary_digest,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct FrameworkRetentionManifestRunner;

impl ErasedNodeRunner for FrameworkRetentionManifestRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { project_retention_manifest(ctx) })
    }
}

pub(crate) fn framework_complete_run_binding(
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
        framework_complete_run_executable(factory_id)?,
        Arc::new(FrameworkCompleteRunRunner),
    )
}

fn framework_complete_run_executable(
    factory_id: events::RunnerFactoryId,
) -> Result<events::ExecutableIdentity> {
    let package_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "runner": "framework_complete_run",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    let binary_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "factory_id": factory_id.as_str(),
        "runner": "framework_complete_run",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-runtime-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-runtime")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: package_digest,
        binary_digest,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

pub(crate) fn framework_resolve_saga_terminal_binding(
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
        framework_resolve_saga_terminal_executable(factory_id)?,
        Arc::new(FrameworkResolveSagaTerminalRunner),
    )
}

fn framework_resolve_saga_terminal_executable(
    factory_id: events::RunnerFactoryId,
) -> Result<events::ExecutableIdentity> {
    let package_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "runner": "framework_resolve_saga_terminal",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    let binary_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "factory_id": factory_id.as_str(),
        "runner": "framework_resolve_saga_terminal",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-runtime-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-runtime")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: package_digest,
        binary_digest,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
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
        let Some(store::CellTerminalProjection::Produced {
            artifact_id,
            content_digest,
            ..
        }) = ctx.projections().cell_terminal(&required.cell_id)
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
            content_digest: content_digest.clone(),
            artifact_id: artifact_id.clone(),
        });
    }
    let rendered_digest = public_output_rendered_digest(render, &cells)?;
    let rendered_artifact_id = None;
    let receipt_bytes = public_output_receipt_json(
        render,
        &cells,
        &rendered_digest,
        rendered_artifact_id.as_ref(),
    )?;
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
    let receipt_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, receipt_bytes.to_vec(), receipt_artifact)?;
    let receipt_retention_ref = retention_ref_for_artifact(receipt_artifact.evidence());
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![receipt_artifact],
        staged_retention_refs: vec![StagedRetentionRefs::framework_public_output(vec![
            receipt_retention_ref,
        ])],
        payloads: vec![
            RunnerEventPayload::CellProduced(events::CellProduced {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                cell_id: ctx.node().output_cell.clone(),
                scope_id: ctx.output_cell().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
                schema_id: ctx.output_cell().schema_id.clone(),
                value_lineage: ctx.output_cell().value_lineage.clone(),
                artifact_id: receipt_artifact_id,
                content_digest: receipt_digest,
                producer_state_kind: Some(ctx.node().state_kind.clone()),
                producer_state_version: Some(ctx.node().state_version.clone()),
            }),
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
                renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
            }),
        ],
    })
}

fn project_retention_manifest(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a retention-manifest framework node",
            ctx.node().node_id
        )));
    };
    let manifest = build_retention_manifest_artifact_with_producer(
        ctx.runtime_spec(),
        ctx.run_id(),
        ctx.run_stream(),
        Some(ctx.node().node_id.clone()),
    )?;
    let manifest_artifact = StagedArtifact::inline_retention_manifest_artifact(
        &ctx,
        manifest.bytes.to_vec(),
        manifest.evidence.clone(),
    )?;
    let receipt_bytes = retention_manifest_receipt_json(&manifest, ctx.run_stream())?;
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
    let receipt_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, receipt_bytes.to_vec(), receipt_artifact)?;
    let receipt_retention_ref = retention_ref_for_artifact(receipt_artifact.evidence());
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![manifest_artifact, receipt_artifact],
        staged_retention_refs: vec![StagedRetentionRefs::runtime_evidence(vec![
            receipt_retention_ref,
        ])],
        payloads: vec![RunnerEventPayload::CellProduced(events::CellProduced {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            cell_id: ctx.node().output_cell.clone(),
            scope_id: ctx.output_cell().scope_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
            schema_id: ctx.output_cell().schema_id.clone(),
            value_lineage: ctx.output_cell().value_lineage.clone(),
            artifact_id: receipt_artifact_id,
            content_digest: receipt_digest,
            producer_state_kind: Some(ctx.node().state_kind.clone()),
            producer_state_version: Some(ctx.node().state_version.clone()),
        })],
    })
}

fn complete_run_framework(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::CompleteRun(_)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a complete-run framework node",
            ctx.node().node_id
        )));
    };
    let completion = run_completion_evidence(ctx.runtime_spec(), ctx.projections())?;
    let retention_manifest = projected_retention_manifest(ctx.run_id(), ctx.projections())?;
    let receipt_bytes =
        complete_run_receipt_json(&completion, retention_manifest, ctx.run_stream())?;
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
    let receipt_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, receipt_bytes.to_vec(), receipt_artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![receipt_artifact],
        staged_retention_refs: Vec::new(),
        payloads: vec![RunnerEventPayload::CellProduced(events::CellProduced {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            cell_id: ctx.node().output_cell.clone(),
            scope_id: ctx.output_cell().scope_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
            schema_id: ctx.output_cell().schema_id.clone(),
            value_lineage: ctx.output_cell().value_lineage.clone(),
            artifact_id: receipt_artifact_id,
            content_digest: receipt_digest,
            producer_state_kind: Some(ctx.node().state_kind.clone()),
            producer_state_version: Some(ctx.node().state_version.clone()),
        })],
    })
}

fn resolve_saga_terminal_framework(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a resolve-saga-terminal framework node",
            ctx.node().node_id
        )));
    };
    let outcome =
        saga_terminal_completion_outcome(ctx.runtime_spec(), ctx.run_id(), ctx.projections())?;
    let receipt_bytes =
        resolve_saga_terminal_receipt_json(ctx.runtime_spec(), &outcome, ctx.run_stream())?;
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
    let receipt_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, receipt_bytes.to_vec(), receipt_artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![receipt_artifact],
        staged_retention_refs: Vec::new(),
        payloads: vec![RunnerEventPayload::CellProduced(events::CellProduced {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            cell_id: ctx.node().output_cell.clone(),
            scope_id: ctx.output_cell().scope_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
            schema_id: ctx.output_cell().schema_id.clone(),
            value_lineage: ctx.output_cell().value_lineage.clone(),
            artifact_id: receipt_artifact_id,
            content_digest: receipt_digest,
            producer_state_kind: Some(ctx.node().state_kind.clone()),
            producer_state_version: Some(ctx.node().state_version.clone()),
        })],
    })
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

pub(crate) fn retention_manifest_receipt_json(
    manifest: &RetentionManifestArtifact,
    pre_projection_stream: &[store::KernelEventEnvelope],
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "manifest_artifact_id": manifest.evidence.artifact_id.as_str(),
        "manifest_digest": manifest.evidence.digest.as_str(),
        "manifest_seq": manifest.manifest_seq,
        "pre_projection_stream_seq": pre_projection_stream
            .last()
            .map(|event| event.seq().as_u64()),
        "previous_manifest_digest": manifest.previous_manifest_digest.as_ref().map(ContentDigest::as_str),
    }))
}

pub(crate) fn complete_run_receipt_json(
    completion: &events::PublicOutputCompletionEvidence,
    retention_manifest: &store::RetentionManifestProjection,
    pre_completion_stream: &[store::KernelEventEnvelope],
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "public_output_event_id": completion.public_output_event_id.as_str(),
        "public_output_schema_id": completion.public_output_schema_id.as_str(),
        "retention_manifest_artifact_id": retention_manifest.manifest_artifact_id.as_str(),
        "retention_manifest_digest": retention_manifest.manifest_digest.as_str(),
        "retention_manifest_seq": retention_manifest.manifest_seq,
        "pre_completion_stream_seq": pre_completion_stream
            .last()
            .map(|event| event.seq().as_u64()),
    }))
}

pub(crate) fn resolve_saga_terminal_receipt_json(
    runtime_spec: &CertifiedRuntimeSpec,
    outcome: &events::RunCompletionOutcome,
    pre_resolution_stream: &[store::KernelEventEnvelope],
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "public_output_schema_id": runtime_spec.spec().public_outputs.public_schema_id.as_str(),
        "terminal_outcome": outcome.kind(),
        "pre_resolution_stream_seq": pre_resolution_stream
            .last()
            .map(|event| event.seq().as_u64()),
    }))
}

pub(crate) fn saga_terminal_completion_outcome(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
) -> Result<events::RunCompletionOutcome> {
    projections
        .saga_terminal_completion_outcome(run_id, &runtime_spec.spec().saga)
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))
}

pub(crate) fn saga_terminal_proof(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
    prefix_next_seq: store::StreamSeq,
    manual: Option<mfm_manual_auth::VerifiedManualResolutionForPrefix>,
) -> Result<store::SagaTerminalProof> {
    let saga = projections.derive_saga_projection(run_id, &runtime_spec.spec().saga);
    store::SagaTerminalProof::new(&runtime_spec.spec().saga, &saga, prefix_next_seq, manual)
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))
}

pub(crate) fn run_completion_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<events::PublicOutputCompletionEvidence> {
    let public_schema_id = runtime_spec.spec().public_outputs.public_schema_id.clone();
    match projections.public_output(&public_schema_id) {
        Some(store::PublicOutputProjection::Produced { event_id, .. }) => {
            Ok(events::PublicOutputCompletionEvidence {
                public_output_schema_id: public_schema_id,
                public_output_event_id: event_id.clone(),
            })
        }
        Some(store::PublicOutputProjection::RenderFailed { .. }) | None => {
            Err(RuntimeError::InvalidRunStream(
                "run completion requires projected public output evidence".to_owned(),
            ))
        }
    }
}

pub(crate) fn projected_retention_manifest<'a>(
    run_id: &RunId,
    projections: &'a store::ProjectionSnapshot,
) -> Result<&'a store::RetentionManifestProjection> {
    projections
        .retention(run_id)
        .and_then(|projection| projection.manifest.as_ref())
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "run completion requires projected retention manifest evidence".to_owned(),
            )
        })
}

pub(crate) fn framework_run_completed_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<Option<events::KernelEventPayload>> {
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
            let completion = run_completion_evidence(runtime_spec, projections)?;
            projected_retention_manifest(run_id, projections)?;
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
            saga_terminal_completion_outcome(runtime_spec, run_id, projections)?
        }
        _ => return Ok(None),
    };
    Ok(Some(events::KernelEventPayload::RunCompleted(
        events::RunCompleted {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            outcome,
        },
    )))
}

pub(crate) fn bootstrap_run_receipt_artifact(
    ctx: &GenesisContext<'_>,
) -> Result<(PlainCanonicalJsonBytes, store::ArtifactEvidenceRef)> {
    let bytes = bootstrap_run_receipt_json(ctx)?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(ctx.output_cell.schema_id.clone()),
        semantic_type_id: Some(ctx.output_cell.semantic_type_id.clone()),
        producer_node_id: Some(ctx.node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    Ok((bytes, evidence))
}

fn bootstrap_run_receipt_json(ctx: &GenesisContext<'_>) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "adapter_executables": ctx.run_started.adapter_executables.iter().map(executable_json).collect::<Vec<_>>(),
        "attempt_id": ctx.attempt_id.as_str(),
        "canonicalizer_identity": ctx.run_started.canonicalizer_identity.as_str(),
        "certificate_artifact": {
            "artifact_id": ctx.run_started.certificate_artifact_id.as_str(),
            "content_digest": ctx.run_started.certificate_artifact_digest.as_str(),
            "media_type": ctx.run_started.certificate_media_type.as_str(),
        },
        "config_artifacts": ctx.config_artifacts.iter().map(config_evidence_json).collect::<Vec<_>>(),
        "framework_version": ctx.run_started.framework_version.as_str(),
        "node_id": ctx.node.node_id.as_str(),
        "output_cell": ctx.node.output_cell.as_str(),
        "public_output_schema_id": ctx.run_started.public_output_schema_id.as_str(),
        "run_id": ctx.run_id.as_str(),
        "runner_executables": ctx.run_started.runner_executables.iter().map(executable_json).collect::<Vec<_>>(),
        "seed_cells": ctx.run_started.seed_cells.iter().map(seed_cell_json).collect::<Vec<_>>(),
        "source_revision": ctx.run_started.source_revision.as_str(),
        "spec_artifact": {
            "artifact_id": ctx.run_started.spec_artifact_id.as_str(),
            "media_type": ctx.run_started.spec_media_type.as_str(),
        },
        "spec_hash": ctx.runtime_spec.spec_hash().as_str(),
    }))
}

fn seed_cell_json(seed: &events::SeedCellRef) -> serde_json::Value {
    serde_json::json!({
        "artifact": artifact_evidence_ref_json(&seed.seed_artifact),
        "cell_id": seed.cell_id.as_str(),
        "content_digest": seed.digest.as_str(),
        "schema_id": seed.schema_id.as_str(),
        "scope_id": seed.scope_id.as_str(),
        "seed_id": seed.seed_id.as_str(),
        "semantic_type_id": seed.semantic_type_id.as_str(),
    })
}

fn config_evidence_json(config: &store::ArtifactEvidenceRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": config.artifact_id.as_str(),
        "byte_len": config.byte_len,
        "content_digest": config.digest.as_str(),
        "media_type": config.media_type.as_str(),
        "schema_id": config.schema_id.as_ref().map(SchemaId::as_str),
    })
}

fn artifact_evidence_ref_json(artifact: &events::ArtifactEvidenceRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": artifact.artifact_id.as_str(),
        "byte_len": artifact.byte_len,
        "content_digest": artifact.content_digest.as_str(),
        "media_type": artifact.media_type.as_str(),
        "role": artifact_role_name(artifact.role),
        "schema_id": artifact.schema_id.as_str(),
        "semantic_type_id": artifact.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    })
}

pub(crate) fn public_output_receipt_digest(
    render: &spec::PublicOutputRenderNodeSpec,
    cells: &[events::NamedTypedCellRef],
    rendered_digest: &ContentDigest,
    rendered_artifact_id: Option<&ArtifactId>,
) -> Result<ContentDigest> {
    Ok(
        public_output_receipt_json(render, cells, rendered_digest, rendered_artifact_id)?
            .content_digest(),
    )
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

/// Rebuilds framework public-output receipt artifact bytes and evidence.
pub fn build_public_output_receipt_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<(PlainCanonicalJsonBytes, store::ArtifactEvidenceRef)> {
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "public-output payload references missing node {}",
            payload.node_id
        ))
    })?;
    validate_public_output(runtime_spec, node, payload)?;
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidSpec(format!(
            "public-output node {} is not a framework render node",
            node.node_id
        )));
    };
    let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "public-output node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let bytes = public_output_receipt_json(
        render,
        &payload.cells,
        &payload.rendered_digest,
        payload.rendered_artifact_id.as_ref(),
    )?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(output_cell.schema_id.clone()),
        semantic_type_id: Some(output_cell.semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    Ok((bytes, evidence))
}

fn retention_manifest_json(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    run_started: &events::RunStarted,
    manifest_seq: u64,
    previous_manifest_digest: Option<&ContentDigest>,
    retained_refs: &[&events::RetentionRef],
    stream: &[store::KernelEventEnvelope],
) -> Result<PlainCanonicalJsonBytes> {
    let spec_canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let spec_digest = spec_canonical.content_digest();
    let certificate_canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let event_schema_ids = stream
        .iter()
        .map(|event| event.event_schema_id().as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let retained_by_role = retained_refs_by_role(retained_refs);
    canonical_json(serde_json::json!({
        "adapter_executables": run_started.adapter_executables.iter().map(executable_json).collect::<Vec<_>>(),
        "canonicalizer_identity": run_started.canonicalizer_identity.as_str(),
        "certificate_artifact": {
            "artifact_id": run_started.certificate_artifact_id.as_str(),
            "byte_len": certificate_canonical.as_bytes().len(),
            "content_digest": run_started.certificate_artifact_digest.as_str(),
            "media_type": run_started.certificate_media_type.as_str(),
        },
        "config_artifacts": runtime_spec.spec().config_refs.iter().map(config_artifact_json).collect::<Vec<_>>(),
        "descriptor_digests": runtime_spec.spec().descriptor_identities.iter().map(descriptor_digest_json).collect::<Vec<_>>(),
        "descriptor_identities": runtime_spec.spec().descriptor_identities.iter().map(descriptor_identity_json).collect::<Vec<_>>(),
        "event_schema_ids": event_schema_ids,
        "manifest_seq": manifest_seq,
        "previous_manifest_digest": previous_manifest_digest.map(ContentDigest::as_str),
        "public_output_artifacts": retained_by_role.public_output_artifacts,
        "receipt_artifacts": retained_by_role.receipt_artifacts,
        "confirmation_artifacts": retained_by_role.confirmation_artifacts,
        "retained_refs": retained_refs.iter().map(|retention_ref| retention_ref_json(retention_ref)).collect::<Vec<_>>(),
        "run_id": run_id.as_str(),
        "runner_executables": run_started.runner_executables.iter().map(executable_json).collect::<Vec<_>>(),
        "spec_artifact": {
            "artifact_id": run_started.spec_artifact_id.as_str(),
            "byte_len": spec_canonical.as_bytes().len(),
            "content_digest": spec_digest.as_str(),
            "media_type": run_started.spec_media_type.as_str(),
        },
        "spec_hash": runtime_spec.spec_hash().as_str(),
        "value_artifacts": retained_by_role.value_artifacts,
    }))
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
        match retention_ref.role {
            events::ArtifactRole::StateOutput
            | events::ArtifactRole::FactResponse
            | events::ArtifactRole::SideEffectIntent
            | events::ArtifactRole::PreparedInvocation
            | events::ArtifactRole::NotSubmittedProof
            | events::ArtifactRole::Submission
            | events::ArtifactRole::SubmissionUnknownEvidence
            | events::ArtifactRole::AmbiguityEvidence
            | events::ArtifactRole::ManualResolutionEvidence
            | events::ArtifactRole::ManualResolutionAuthorization
            | events::ArtifactRole::RedactedDiagnostic => {
                value_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRole::Receipt => {
                receipt_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRole::Confirmation => {
                confirmation_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRole::PublicOutput => {
                public_output_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRole::TypedExecutionSpec
            | events::ArtifactRole::TypedSpecCertificate
            | events::ArtifactRole::TypedConfig
            | events::ArtifactRole::SeedInput
            | events::ArtifactRole::RetentionManifest => {}
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

fn executable_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
    serde_json::json!({
        "binary_digest": identity.binary_digest.as_str(),
        "cargo_package_digest": identity.cargo_package_digest.as_str(),
        "cargo_package_name": identity.cargo_package_name.as_str(),
        "cargo_package_version": identity.cargo_package_version.as_str(),
        "factory_id": identity.factory_id.as_str(),
        "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(events::NixDerivationHash::as_str),
        "nix_output_hash": identity.nix_output_hash.as_ref().map(events::NixOutputHash::as_str),
        "source_revision": identity.source_revision.as_str(),
    })
}

fn retention_ref_json(retention_ref: &events::RetentionRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": retention_ref.artifact_id.as_str(),
        "content_digest": retention_ref.content_digest.as_str(),
        "role": retention_role_str(retention_ref.role),
    })
}

fn retention_role_str(role: events::ArtifactRole) -> &'static str {
    match role {
        events::ArtifactRole::TypedExecutionSpec => "typed_execution_spec",
        events::ArtifactRole::TypedSpecCertificate => "typed_spec_certificate",
        events::ArtifactRole::TypedConfig => "typed_config",
        events::ArtifactRole::SeedInput => "seed_input",
        events::ArtifactRole::StateOutput => "state_output",
        events::ArtifactRole::FactResponse => "fact_response",
        events::ArtifactRole::SideEffectIntent => "side_effect_intent",
        events::ArtifactRole::PreparedInvocation => "prepared_invocation",
        events::ArtifactRole::NotSubmittedProof => "not_submitted_proof",
        events::ArtifactRole::Submission => "submission",
        events::ArtifactRole::SubmissionUnknownEvidence => "submission_unknown_evidence",
        events::ArtifactRole::Receipt => "receipt",
        events::ArtifactRole::Confirmation => "confirmation",
        events::ArtifactRole::AmbiguityEvidence => "ambiguity_evidence",
        events::ArtifactRole::ManualResolutionEvidence => "manual_resolution_evidence",
        events::ArtifactRole::ManualResolutionAuthorization => "manual_resolution_authorization",
        events::ArtifactRole::PublicOutput => "public_output",
        events::ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
        events::ArtifactRole::RetentionManifest => "retention_manifest",
    }
}

pub(crate) fn retention_reason_str(reason: events::RetentionReason) -> &'static str {
    match reason {
        events::RetentionReason::RunStarted => "run_started",
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

pub(crate) struct GenesisContext<'a> {
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    pub(crate) run_id: &'a RunId,
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) output_cell: &'a spec::CellSpec,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) run_started: &'a events::RunStarted,
    pub(crate) config_artifacts: &'a [store::ArtifactEvidenceRef],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetentionManifestArtifact {
    pub(crate) bytes: PlainCanonicalJsonBytes,
    pub(crate) evidence: store::ArtifactEvidenceRef,
    pub(crate) manifest_seq: u64,
    pub(crate) previous_manifest_digest: Option<ContentDigest>,
}

pub(crate) fn public_output_is_produced(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> bool {
    let public_schema_id = &runtime_spec.spec().public_outputs.public_schema_id;
    matches!(
        projections.public_output(public_schema_id),
        Some(store::PublicOutputProjection::Produced { .. })
    )
}

pub(crate) fn build_retention_manifest_artifact_with_producer(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
    producer_node_id: Option<NodeId>,
) -> Result<RetentionManifestArtifact> {
    store::ProjectionSnapshot::validate_run_stream(stream)?;
    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
    let run_started = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "retention manifest requires RunStarted evidence".to_owned(),
            )
        })?;
    if &run_started.run_id != run_id || &run_started.spec_hash != runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest run-start evidence does not match certified run".to_owned(),
        ));
    }
    let retention = projection.retention(run_id).cloned().unwrap_or_default();
    let manifest_seq = retention
        .manifest
        .as_ref()
        .map(|manifest| {
            manifest.manifest_seq.checked_add(1).ok_or_else(|| {
                RuntimeError::InvalidRunStream("retention manifest sequence overflow".to_owned())
            })
        })
        .transpose()?
        .unwrap_or(1);
    let previous_manifest_digest = retention
        .manifest
        .as_ref()
        .map(|manifest| manifest.manifest_digest.clone());
    let retained_refs = retention.refs.values().collect::<Vec<_>>();
    let manifest_json = retention_manifest_json(
        runtime_spec,
        run_id,
        run_started,
        manifest_seq,
        previous_manifest_digest.as_ref(),
        &retained_refs,
        stream,
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
            producer_node_id,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::RetentionManifest,
        },
        manifest_seq,
        previous_manifest_digest,
    })
}

pub(crate) fn certified_bootstrap_run_node(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<&spec::NodeSpec> {
    let mut bootstrap_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::BootstrapRun(_))
        ) && bootstrap_node.replace(node).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(
                "multiple certified bootstrap framework nodes".to_owned(),
            ));
        }
    }
    bootstrap_node.ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "RunStarted lacks a certified BootstrapRun framework node".to_owned(),
        )
    })
}

pub(crate) fn certified_retention_manifest_node(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<&spec::NodeSpec> {
    let mut retention_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        ) && retention_node.replace(node).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(
                "multiple certified retention manifest framework nodes".to_owned(),
            ));
        }
    }
    retention_node.ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "retention manifest projection lacks a certified framework retention node".to_owned(),
        )
    })
}

pub(crate) fn certified_complete_run_node(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<&spec::NodeSpec> {
    let mut completion_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::CompleteRun(_))
        ) && completion_node.replace(node).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(
                "multiple certified completion framework nodes".to_owned(),
            ));
        }
    }
    completion_node.ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "RunCompleted lacks a certified CompleteRun framework node".to_owned(),
        )
    })
}

pub(crate) fn certified_resolve_saga_terminal_node(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<&spec::NodeSpec> {
    let mut resolve_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        ) && resolve_node.replace(node).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(
                "multiple certified resolve-saga-terminal framework nodes".to_owned(),
            ));
        }
    }
    resolve_node.ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "RunCompleted lacks a certified ResolveSagaTerminal framework node".to_owned(),
        )
    })
}
