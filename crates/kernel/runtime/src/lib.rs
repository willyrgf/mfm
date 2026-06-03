#![warn(missing_docs)]
//! Serial typed scheduler for certified MFM execution specs.
//!
//! This crate owns the first certified runtime boundary. It derives runnable nodes, materialized
//! input evidence, runner bindings, and runtime capabilities only from a verified
//! [`mfm_certify::CertifiedTypedSpec`] plus store-owned typed projections.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, NodeId, RunId, SchemaId, SemanticTypeId,
    SpecHash,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

mod artifacts;
mod error;
mod invocation;
mod runners;
mod spec_authority;

pub use artifacts::{
    RuntimeArtifactStageFuture, RuntimeArtifactStager, StagedArtifact, StagedArtifactBindingKind,
    StagedArtifactHandle, StagedRetentionRefs, StagedSideEffectArtifactPhase,
};
pub use error::RuntimeError;
pub use invocation::{
    CertifiedRuntimeCapabilities, ErasedRunCtx, MaterializedCell, MaterializedCellTerminal,
    MaterializedInputNode, MaterializedInputs, NamedMaterializedInput, PreparedRunnerInvocation,
    RecordedFact, RecordedFacts,
};
pub use runners::{
    ErasedNodeRunner, ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput,
    ErasedRunnerRegistry, RunnerEventPayload,
};
pub use spec_authority::CertifiedRuntimeSpec;

use artifacts::{
    artifact_role_name, staged_artifact_binding_kind, staged_artifact_binding_role,
    verify_artifact_bytes,
};
use error::async_store_error;

#[cfg(test)]
use artifacts::staged_side_effect_artifact_phase;

/// Result type for typed runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

fn framework_bootstrap_run_binding(
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

fn framework_public_output_binding(
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

fn framework_retention_manifest_binding(
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

fn framework_complete_run_binding(
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

fn public_output_rendered_digest(
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

fn retention_manifest_receipt_json(
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

fn complete_run_receipt_json(
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

fn run_completion_evidence(
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

fn projected_retention_manifest<'a>(
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

fn framework_run_completed_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<Option<events::KernelEventPayload>> {
    let Some(spec::FrameworkNodeSpec::CompleteRun(complete)) = &node.framework else {
        return Ok(None);
    };
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
    Ok(Some(events::KernelEventPayload::RunCompleted(
        events::RunCompleted {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            outcome: events::RunCompletionOutcome::Completed(completion),
        },
    )))
}

fn bootstrap_run_receipt_artifact(
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

fn public_output_receipt_digest(
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
        events::ArtifactRole::PublicOutput => "public_output",
        events::ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
        events::ArtifactRole::RetentionManifest => "retention_manifest",
    }
}

fn retention_reason_str(reason: events::RetentionReason) -> &'static str {
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

#[derive(Debug, Clone)]
struct RuntimeRunView {
    stream: Vec<store::KernelEventEnvelope>,
    projections: store::ProjectionSnapshot,
    seed_cells: BTreeMap<CellId, events::SeedCellRef>,
    config_artifacts: BTreeMap<String, store::ArtifactEvidenceRef>,
    artifact_refs: BTreeMap<ArtifactId, CommittedArtifactReference>,
    next_seq: store::StreamSeq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommittedArtifactReference {
    evidence: store::ArtifactEvidenceRef,
    attempt_id: Option<AttemptId>,
    commit_seq: store::StreamSeq,
    commit_key: store::CommitKey,
}

/// Store-owned run stream validated against certified runtime authority.
#[derive(Debug, Clone)]
pub struct VerifiedRunStream {
    run_id: RunId,
    spec_hash: SpecHash,
    stream: Vec<store::KernelEventEnvelope>,
    projection: store::ProjectionSnapshot,
}

impl VerifiedRunStream {
    /// Loads the authoritative run stream from a typed store and validates it against certified
    /// runtime authority.
    pub fn from_store<S>(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<Self>
    where
        S: store::TypedRunEventStore + ?Sized,
    {
        let stream = store.load_run_stream(run_id);
        Self::from_stream(runtime_spec, run_id, &stream)
    }

    /// Loads the authoritative run stream from an async typed store and validates it against
    /// certified runtime authority.
    pub async fn from_async_store<S>(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<Self>
    where
        S: store::AsyncTypedRunEventStore + ?Sized,
    {
        let stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        Self::from_stream(runtime_spec, run_id, &stream)
    }

    fn from_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        stream: &[store::KernelEventEnvelope],
    ) -> Result<Self> {
        let view = RuntimeRunView::from_stream(runtime_spec, run_id, stream)?;
        Ok(Self {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            stream: view.stream,
            projection: view.projections,
        })
    }

    /// Run id covered by this verified stream.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Certified spec hash covered by this verified stream.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Authoritative committed event envelopes covered by this verified stream.
    pub fn events(&self) -> &[store::KernelEventEnvelope] {
        &self.stream
    }

    /// Projection rebuilt from the verified committed stream.
    pub fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        &self.projection
    }
}

impl RuntimeRunView {
    fn from_store<S: store::TypedRunEventStore + ?Sized>(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<Self> {
        let stream = store.load_run_stream(run_id);
        Self::from_stream(runtime_spec, run_id, &stream)
    }

    fn from_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        stream: &[store::KernelEventEnvelope],
    ) -> Result<Self> {
        store::ProjectionSnapshot::validate_run_stream(stream)?;
        let next_seq = next_seq_after_stream(stream)?;
        let projections = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
        let mut run_started = None;
        for event in stream {
            match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => {
                    if &payload.run_id != run_id {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "run stream contains RunStarted for {} while executing {}",
                            payload.run_id, run_id
                        )));
                    }
                    if &payload.spec_hash != runtime_spec.spec_hash() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "RunStarted spec hash {} does not match certified {}",
                            payload.spec_hash,
                            runtime_spec.spec_hash()
                        )));
                    }
                    if run_started.replace(payload.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(
                            "run stream contains multiple RunStarted events".to_owned(),
                        ));
                    }
                }
                payload if payload_spec_hash(payload) != *runtime_spec.spec_hash() => {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "event payload spec hash {} does not match certified {}",
                        payload_spec_hash(payload),
                        runtime_spec.spec_hash()
                    )));
                }
                _ => {}
            }
        }
        let Some(run_started) = run_started else {
            return Err(RuntimeError::InvalidRunStream(
                "run has not started with certified RunStarted evidence".to_owned(),
            ));
        };
        validate_historical_run_stream(runtime_spec, run_id, stream, &projections)?;
        let seed_cells = validate_seed_cells(runtime_spec, &run_started.seed_cells)?;
        let config_artifacts = config_artifacts_from_stream(runtime_spec, stream)?;
        let artifact_refs = artifact_refs_from_stream(stream)?;
        Ok(Self {
            stream: stream.to_vec(),
            projections,
            seed_cells,
            config_artifacts,
            artifact_refs,
            next_seq,
        })
    }
}

fn next_seq_after_stream(stream: &[store::KernelEventEnvelope]) -> Result<store::StreamSeq> {
    let Some(event) = stream.last() else {
        return Ok(store::StreamSeq::FIRST);
    };
    let next =
        event.seq().as_u64().checked_add(1).ok_or_else(|| {
            RuntimeError::InvalidRunStream("run stream sequence overflow".to_owned())
        })?;
    store::StreamSeq::new(next).map_err(RuntimeError::from)
}

/// Validates a stored typed run stream against the certified runtime spec without executing work.
///
/// This is the read-only counterpart to scheduler resume: callers that inspect, render, or
/// append-only resume a run must still prove the historical stream is bound to the stored certified
/// spec before trusting projections.
pub fn validate_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    VerifiedRunStream::from_stream(runtime_spec, run_id, stream).map(|_| ())
}

/// Launch evidence needed to prepare a typed run genesis commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchEvidence {
    /// Staged certified spec bytes and the evidence to admit with `RunStarted`.
    pub spec_artifact: RunLaunchArtifact,
    /// Staged certified spec certificate bytes and the evidence to admit with `RunStarted`.
    pub certificate_artifact: RunLaunchArtifact,
    /// Staged config artifacts for every certified config reference.
    pub config_artifacts: Vec<RunLaunchArtifact>,
    /// Framework build/version identity.
    pub framework_version: events::FrameworkVersion,
    /// Source revision identity.
    pub source_revision: events::SourceRevision,
    /// Adapter executable identities bound to the run.
    pub adapter_executables: Vec<events::ExecutableIdentity>,
    /// Seed cells materialized at run start.
    pub seed_cells: Vec<RunLaunchSeedCell>,
}

/// Staged launch artifact bytes plus typed evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchArtifact {
    /// Artifact bytes to stage through runtime middleware before the genesis commit.
    pub bytes: Vec<u8>,
    /// Typed artifact evidence to admit atomically with the genesis commit.
    pub evidence: store::ArtifactEvidenceRef,
}

/// Staged launch seed bytes plus the seed cell authority bound into `RunStarted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchSeedCell {
    /// Seed artifact bytes to stage through runtime middleware before the genesis commit.
    pub bytes: Vec<u8>,
    /// Seed cell reference to persist in `RunStarted`.
    pub cell: events::SeedCellRef,
}

/// Prepared genesis launch authority accepted by runtime-owned start middleware.
pub struct PreparedRunLaunch {
    commit: store::PreparedTypedCommit,
    artifacts_to_stage: Vec<PreparedStagedArtifact>,
}

struct GenesisContext<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    output_cell: &'a spec::CellSpec,
    attempt_id: &'a AttemptId,
    run_started: &'a events::RunStarted,
    config_artifacts: &'a [store::ArtifactEvidenceRef],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RetentionManifestArtifact {
    bytes: PlainCanonicalJsonBytes,
    evidence: store::ArtifactEvidenceRef,
    manifest_seq: u64,
    previous_manifest_digest: Option<ContentDigest>,
}

/// Result of one serial scheduler drive call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerStatus {
    /// At least one node attempt ran and committed.
    Advanced,
    /// No node is currently runnable.
    Blocked,
    /// Public output has already been projected.
    PublicOutputProjected,
}

struct RunnableNode<'a> {
    node: &'a spec::NodeSpec,
    attempt: AttemptPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AttemptPlan {
    StartNew,
    Continue {
        attempt_id: AttemptId,
        attempt_no: u32,
    },
}

struct RunnerOutputCommitInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    attempt_id: &'a AttemptId,
    caps: &'a CertifiedRuntimeCapabilities,
    recorded_facts: &'a RecordedFacts,
    view: &'a RuntimeRunView,
    output: ErasedRunnerOutput,
}

struct RunnerInvocationInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    attempt_id: &'a AttemptId,
    attempt_no: u32,
    view: &'a RuntimeRunView,
}

struct FrameworkNodeAttemptInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    view: &'a RuntimeRunView,
    runnable: RunnableNode<'a>,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    binding: ErasedRunnerBinding,
}

struct CompleteRunCommitValidation<'a> {
    run_id: &'a RunId,
    spec_hash: &'a SpecHash,
    completion: &'a events::PublicOutputCompletionEvidence,
    completion_node_id: &'a NodeId,
    attempt_id: &'a AttemptId,
    receipt_cell_id: &'a CellId,
    receipt_artifact_id: &'a ArtifactId,
    receipt_digest: &'a ContentDigest,
    expected_receipt_ref: &'a events::ArtifactEvidenceRef,
}

struct PreparedRunnerOutput {
    commit: store::PreparedTypedCommit,
    artifacts_to_stage: Vec<PreparedStagedArtifact>,
}

struct PreparedStagedArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn prepare_runner_invocation<'a>(
    input: RunnerInvocationInput<'a>,
) -> Result<PreparedRunnerInvocation<'a>> {
    let RunnerInvocationInput {
        runtime_spec,
        run_id,
        node,
        descriptor,
        output_cell,
        attempt_id,
        attempt_no,
        view,
    } = input;
    let config_artifact = committed_config_artifact(node, view)?;
    let inputs = materialize_inputs(runtime_spec, node, view)?;
    let caps =
        CertifiedRuntimeCapabilities::new(node.node_id.clone(), node.capability_bindings.clone());
    let recorded_facts = recorded_facts_for_attempt(&view.projections, &node.node_id, attempt_id)?;
    Ok(PreparedRunnerInvocation {
        runtime_spec,
        run_id,
        spec_hash: runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id,
        attempt_no,
        config_artifact,
        inputs,
        caps,
        recorded_facts,
        projections: &view.projections,
        run_stream: &view.stream,
    })
}

struct RuntimeMutationMiddleware;

impl RuntimeMutationMiddleware {
    fn prepare_run_launch(
        runners: &ErasedRunnerRegistry,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
    ) -> Result<PreparedRunLaunch> {
        let spec_input = evidence.spec_artifact;
        verify_artifact_bytes(&spec_input.bytes, &spec_input.evidence)?;
        let spec_artifact = validate_spec_artifact(runtime_spec, spec_input.evidence.clone())?;
        let certificate_input = evidence.certificate_artifact;
        verify_artifact_bytes(&certificate_input.bytes, &certificate_input.evidence)?;
        let certificate_artifact =
            validate_certificate_artifact(runtime_spec, certificate_input.evidence.clone())?;
        let config_inputs = evidence.config_artifacts;
        let config_artifacts = validate_config_artifacts(
            runtime_spec,
            config_inputs
                .iter()
                .map(|artifact| {
                    verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
                    Ok(artifact.evidence.clone())
                })
                .collect::<Result<Vec<_>>>()?,
        )?;
        let mut config_staged_artifacts = launch_artifacts_by_id(config_inputs, "config")?;
        let config_reference_payloads =
            config_artifact_reference_payloads(runtime_spec.spec_hash(), &config_artifacts)?;
        let seed_inputs = evidence.seed_cells;
        let seed_cell_refs = seed_inputs
            .iter()
            .map(|seed| seed.cell.clone())
            .collect::<Vec<_>>();
        let seed_cells = validate_seed_cells(runtime_spec, &seed_cell_refs)?;
        let seed_staged_artifacts =
            validate_launch_seed_artifacts(seed_inputs, seed_cells.values())?;
        let runner_executables = runners.executables_for_spec(runtime_spec)?;
        let bootstrap_node = certified_bootstrap_run_node(runtime_spec)?;
        let bootstrap_output_cell =
            runtime_spec
                .cell(&bootstrap_node.output_cell)
                .ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "bootstrap lifecycle node {} output cell {} is missing",
                        bootstrap_node.node_id, bootstrap_node.output_cell
                    ))
                })?;
        let bootstrap_attempt_id = attempt_id(
            &run_id,
            runtime_spec.spec_hash(),
            &bootstrap_node.node_id,
            1,
        )?;
        let mut required_artifacts =
            Vec::with_capacity(2 + config_artifacts.len() + seed_cells.len());
        required_artifacts.push(spec_artifact.clone());
        required_artifacts.push(certificate_artifact.clone());
        required_artifacts.extend(config_artifacts.iter().cloned());
        required_artifacts.extend(seed_cells.values().map(store_seed_artifact));
        let run_started = events::RunStarted {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            spec_artifact_id: spec_artifact.artifact_id.clone(),
            certificate_artifact_id: certificate_artifact.artifact_id.clone(),
            certificate_artifact_digest: certificate_artifact.digest.clone(),
            certificate_media_type: certificate_artifact.media_type.clone(),
            spec_media_type: runtime_spec.spec().media_type.clone(),
            spec_version: runtime_spec.spec().spec_version.clone(),
            lowering_version: runtime_spec.spec().lowering_version.clone(),
            public_output_schema_id: runtime_spec.spec().public_outputs.public_schema_id.clone(),
            descriptor_identities: runtime_spec.spec().descriptor_identities.clone(),
            runner_executables,
            adapter_executables: evidence.adapter_executables,
            canonicalizer_identity: runtime_spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
                .clone(),
            framework_version: evidence.framework_version,
            source_revision: evidence.source_revision,
            seed_cells: seed_cell_refs,
        };
        let genesis = GenesisContext {
            runtime_spec,
            run_id: &run_id,
            node: bootstrap_node,
            output_cell: bootstrap_output_cell,
            attempt_id: &bootstrap_attempt_id,
            run_started: &run_started,
            config_artifacts: &config_artifacts,
        };
        let (bootstrap_receipt_bytes, bootstrap_receipt_artifact) =
            bootstrap_run_receipt_artifact(&genesis)?;
        let mut run_started_retention_refs = required_artifacts
            .iter()
            .map(retention_ref_for_artifact)
            .collect::<Vec<_>>();
        run_started_retention_refs.push(retention_ref_for_artifact(&bootstrap_receipt_artifact));
        required_artifacts.push(bootstrap_receipt_artifact.clone());
        let admitted_artifacts = required_artifacts.clone();
        let start_payload = events::KernelEventPayload::RunStarted(run_started);
        let bootstrap_start =
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: bootstrap_node.node_id.clone(),
                attempt_id: bootstrap_attempt_id.clone(),
                attempt_no: 1,
                state_kind: bootstrap_node.state_kind.clone(),
                state_version: bootstrap_node.state_version.clone(),
            });
        let bootstrap_cell = events::KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: runtime_spec.spec_hash().clone(),
            node_id: bootstrap_node.node_id.clone(),
            cell_id: bootstrap_node.output_cell.clone(),
            scope_id: bootstrap_output_cell.scope_id.clone(),
            attempt_id: bootstrap_attempt_id.clone(),
            semantic_type_id: bootstrap_output_cell.semantic_type_id.clone(),
            schema_id: bootstrap_output_cell.schema_id.clone(),
            value_lineage: bootstrap_output_cell.value_lineage.clone(),
            artifact_id: bootstrap_receipt_artifact.artifact_id.clone(),
            content_digest: bootstrap_receipt_artifact.digest.clone(),
            producer_state_kind: Some(bootstrap_node.state_kind.clone()),
            producer_state_version: Some(bootstrap_node.state_version.clone()),
        });
        let bootstrap_completed =
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: bootstrap_node.node_id.clone(),
                attempt_id: bootstrap_attempt_id.clone(),
                output_cell_id: bootstrap_node.output_cell.clone(),
            });
        let bootstrap_ref =
            events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: Some(bootstrap_node.node_id.clone()),
                attempt_id: Some(bootstrap_attempt_id.clone()),
                artifact_ref: event_artifact_ref_from_store(&bootstrap_receipt_artifact)?,
            });
        let retention_payload =
            events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                refs: run_started_retention_refs,
                reason: events::RetentionReason::RunStarted,
            });
        let mut payloads = Vec::with_capacity(6 + config_reference_payloads.len());
        payloads.push(start_payload);
        payloads.extend(config_reference_payloads);
        payloads.push(bootstrap_start);
        payloads.push(bootstrap_cell);
        payloads.push(bootstrap_completed);
        payloads.push(bootstrap_ref);
        payloads.push(retention_payload);
        let request = store::TypedCommitRequest {
            run_id,
            expected_next_seq,
            commit_key: store::CommitKey::new(format!(
                "run-start:{}",
                runtime_spec.spec_hash().as_str()
            ))?,
            payloads,
            required_artifacts,
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::Absent,
                ..store::CommitPreconditions::default()
            },
        };
        let commit = store::PreparedTypedCommit::new(request, admitted_artifacts)?;
        let mut artifacts_to_stage =
            Vec::with_capacity(3 + config_artifacts.len() + seed_staged_artifacts.len());
        artifacts_to_stage.push(PreparedStagedArtifact {
            bytes: spec_input.bytes,
            evidence: spec_artifact,
        });
        artifacts_to_stage.push(PreparedStagedArtifact {
            bytes: certificate_input.bytes,
            evidence: certificate_artifact,
        });
        for artifact in &config_artifacts {
            let staged = config_staged_artifacts
                .remove(&artifact.artifact_id)
                .ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "missing staged config artifact bytes for {}",
                        artifact.artifact_id
                    ))
                })?;
            artifacts_to_stage.push(PreparedStagedArtifact {
                bytes: staged.bytes,
                evidence: artifact.clone(),
            });
        }
        artifacts_to_stage.extend(seed_staged_artifacts);
        artifacts_to_stage.push(PreparedStagedArtifact {
            bytes: bootstrap_receipt_bytes.to_vec(),
            evidence: bootstrap_receipt_artifact,
        });
        Ok(PreparedRunLaunch {
            commit,
            artifacts_to_stage,
        })
    }

    fn prepare_attempt_start(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        attempt_no: u32,
        view: &RuntimeRunView,
    ) -> Result<store::PreparedTypedCommit> {
        let start_payload =
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            });
        let mut preconditions = store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        };
        preconditions
            .required_cell_states
            .extend(node_cell_preconditions(runtime_spec, node)?);
        let request = store::TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: view.next_seq,
            commit_key: store::CommitKey::new(format!(
                "attempt-start:{}:{}",
                node.node_id, attempt_id
            ))?,
            payloads: vec![start_payload],
            required_artifacts: Vec::new(),
            preconditions,
        };
        store::PreparedTypedCommit::new(request, Vec::new()).map_err(RuntimeError::from)
    }

    fn prepare_runner_output(input: RunnerOutputCommitInput<'_>) -> Result<PreparedRunnerOutput> {
        Self::prepare_runner_output_with_start(input, None)
    }

    fn prepare_started_runner_output(
        input: RunnerOutputCommitInput<'_>,
        attempt_no: u32,
    ) -> Result<PreparedRunnerOutput> {
        Self::prepare_runner_output_with_start(input, Some(attempt_no))
    }

    fn prepare_runner_output_with_start(
        input: RunnerOutputCommitInput<'_>,
        started_in_same_commit: Option<u32>,
    ) -> Result<PreparedRunnerOutput> {
        let ErasedRunnerOutput {
            staged_artifacts,
            staged_retention_refs,
            payloads: runner_payloads,
        } = input.output;
        let runner_payloads = runner_payloads_with_derived_lifecycle(
            input.runtime_spec,
            input.node,
            input.attempt_id,
            runner_payloads,
        )?;
        validate_runner_output(
            input.runtime_spec,
            input.node,
            input.attempt_id,
            input.caps,
            input.recorded_facts,
            &input.view.projections,
            &runner_payloads,
        )?;
        if matches!(
            &input.node.framework,
            Some(spec::FrameworkNodeSpec::CompleteRun(_))
        ) && !staged_retention_refs.is_empty()
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "complete-run framework node {} cannot stage retention refs after manifest projection",
                input.node.node_id
            )));
        }
        let staged_artifacts = validate_staged_artifacts(
            input.run_id,
            input.node,
            input.attempt_id,
            &staged_artifacts,
        )?;
        let retention_manifest = framework_retention_manifest_artifact(
            input.runtime_spec,
            input.run_id,
            input.node,
            &input.view.stream,
            &staged_artifacts,
        )?;
        let payload_bound_artifacts = staged_artifacts
            .iter()
            .filter(|artifact| artifact.binding != StagedArtifactBindingKind::RetentionManifest)
            .cloned()
            .collect::<Vec<_>>();
        validate_staged_artifact_payload_bindings(
            input.node,
            input.attempt_id,
            &runner_payloads,
            &payload_bound_artifacts,
        )?;
        let mut payloads = if let Some(attempt_no) = started_in_same_commit {
            vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: input.runtime_spec.spec_hash().clone(),
                    node_id: input.node.node_id.clone(),
                    attempt_id: input.attempt_id.clone(),
                    attempt_no,
                    state_kind: input.node.state_kind.clone(),
                    state_version: input.node.state_version.clone(),
                },
            )]
        } else {
            Vec::new()
        };
        payloads.extend(runner_payloads);
        if let Some(manifest) = retention_manifest {
            payloads.extend(retention_manifest_payloads(
                input.runtime_spec,
                input.run_id,
                manifest,
            ));
        }
        payloads.extend(staged_artifact_reference_payloads(
            input.runtime_spec.spec_hash(),
            input.node,
            input.attempt_id,
            &payloads,
            &payload_bound_artifacts,
        ));
        let artifacts_to_stage = staged_artifacts
            .iter()
            .filter_map(|artifact| {
                artifact.bytes.as_ref().map(|bytes| PreparedStagedArtifact {
                    bytes: bytes.clone(),
                    evidence: artifact.evidence.clone(),
                })
            })
            .collect::<Vec<_>>();
        let required_artifacts = staged_artifacts
            .iter()
            .map(|artifact| artifact.evidence.clone())
            .collect::<Vec<_>>();
        payloads.extend(bind_staged_retention_refs(
            input.runtime_spec,
            input.run_id,
            input.node,
            &required_artifacts,
            staged_retention_refs,
        )?);
        if let Some(run_completed) = framework_run_completed_payload(
            input.runtime_spec,
            input.run_id,
            input.node,
            &input.view.projections,
        )? {
            payloads.push(run_completed);
        }
        let admitted_artifacts = required_artifacts.clone();
        let preconditions = runner_output_preconditions(
            input.runtime_spec,
            input.run_id,
            input.node,
            input.attempt_id,
            &input.view.projections,
            &payloads,
            started_in_same_commit.is_none(),
        )?;
        let request = store::TypedCommitRequest {
            run_id: input.run_id.clone(),
            expected_next_seq: input.view.next_seq,
            commit_key: runner_output_commit_key(input.node, input.attempt_id, &payloads)?,
            payloads,
            required_artifacts,
            preconditions,
        };
        let commit = store::PreparedTypedCommit::new(request, admitted_artifacts)?;
        Ok(PreparedRunnerOutput {
            commit,
            artifacts_to_stage,
        })
    }
}

fn launch_artifacts_by_id(
    artifacts: Vec<RunLaunchArtifact>,
    kind: &'static str,
) -> Result<BTreeMap<ArtifactId, RunLaunchArtifact>> {
    let mut by_artifact = BTreeMap::new();
    for artifact in artifacts {
        verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
        if by_artifact
            .insert(artifact.evidence.artifact_id.clone(), artifact)
            .is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate staged {kind} launch artifact"
            )));
        }
    }
    Ok(by_artifact)
}

fn validate_launch_seed_artifacts<'a>(
    seeds: Vec<RunLaunchSeedCell>,
    validated_cells: impl IntoIterator<Item = &'a events::SeedCellRef>,
) -> Result<Vec<PreparedStagedArtifact>> {
    let mut by_cell = BTreeMap::new();
    for seed in seeds {
        if by_cell.insert(seed.cell.cell_id.clone(), seed).is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "duplicate staged seed launch artifact".to_owned(),
            ));
        }
    }
    let mut staged = Vec::with_capacity(by_cell.len());
    for cell in validated_cells {
        let seed = by_cell.remove(&cell.cell_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing staged seed bytes for cell {}",
                cell.cell_id
            ))
        })?;
        let evidence = store_seed_artifact(cell);
        verify_artifact_bytes(&seed.bytes, &evidence)?;
        staged.push(PreparedStagedArtifact {
            bytes: seed.bytes,
            evidence,
        });
    }
    if !by_cell.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "staged seed launch artifacts contain entries not certified by the spec".to_owned(),
        ));
    }
    Ok(staged)
}

/// Serial typed scheduler.
#[derive(Clone)]
pub struct SerialTypedScheduler {
    runners: ErasedRunnerRegistry,
    artifact_stager: Arc<dyn RuntimeArtifactStager>,
}

impl SerialTypedScheduler {
    /// Creates a scheduler using a certified runner registry and runtime artifact stager.
    pub fn new(
        runners: ErasedRunnerRegistry,
        artifact_stager: Arc<dyn RuntimeArtifactStager>,
    ) -> Self {
        Self {
            runners,
            artifact_stager,
        }
    }

    /// Prepares sealed genesis launch authority for a certified run.
    pub fn prepare_run_launch(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
    ) -> Result<PreparedRunLaunch> {
        RuntimeMutationMiddleware::prepare_run_launch(
            &self.runners,
            runtime_spec,
            run_id,
            evidence,
            expected_next_seq,
        )
    }

    /// Appends the prepared typed genesis commit after staging middleware-owned artifacts.
    pub async fn start_run<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        Ok(store.append_prepared_typed_commit(launch.commit)?)
    }

    /// Appends the prepared typed genesis commit through an async typed store.
    pub async fn start_run_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(launch.commit)
            .await
            .map_err(async_store_error)
    }

    /// Runs one deterministic runnable node, if any.
    pub async fn drive_once<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let view = RuntimeRunView::from_store(runtime_spec, run_id, store)?;
        if view.projections.run_state(run_id) == store::RunState::Completed {
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        if public_output_is_produced(runtime_spec, &view.projections) {
            if let Some(runnable) = next_runnable_node(runtime_spec, &view)? {
                self.run_node_attempt(store, runtime_spec, run_id, &view, runnable)
                    .await?;
                return Ok(SchedulerStatus::Advanced);
            }
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        let Some(runnable) = next_runnable_node(runtime_spec, &view)? else {
            return Ok(SchedulerStatus::Blocked);
        };
        self.run_node_attempt(store, runtime_spec, run_id, &view, runnable)
            .await?;
        Ok(SchedulerStatus::Advanced)
    }

    /// Runs deterministic runnable nodes until no node is runnable or public output is projected.
    pub async fn drive_until_blocked<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        loop {
            match self.drive_once(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    /// Runs one deterministic runnable node against an async durable typed store, if any.
    pub async fn drive_once_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let view = RuntimeRunView::from_stream(runtime_spec, run_id, &stream)?;
        if view.projections.run_state(run_id) == store::RunState::Completed {
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        if public_output_is_produced(runtime_spec, &view.projections) {
            if let Some(runnable) = next_runnable_node(runtime_spec, &view)? {
                self.run_node_attempt_async(store, runtime_spec, run_id, &view, runnable)
                    .await?;
                return Ok(SchedulerStatus::Advanced);
            }
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        let Some(runnable) = next_runnable_node(runtime_spec, &view)? else {
            return Ok(SchedulerStatus::Blocked);
        };
        self.run_node_attempt_async(store, runtime_spec, run_id, &view, runnable)
            .await?;
        Ok(SchedulerStatus::Advanced)
    }

    /// Runs deterministic runnable nodes against an async durable typed store until blocked.
    pub async fn drive_until_blocked_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        loop {
            match self.drive_once_async(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    async fn run_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
            )
        ) {
            self.run_started_framework_node_attempt(
                store,
                FrameworkNodeAttemptInput {
                    runtime_spec,
                    run_id,
                    view,
                    runnable,
                    descriptor,
                    output_cell,
                    binding,
                },
            )
            .await?;
            return Ok(());
        }
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew => {
                let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                prepare_runner_invocation(RunnerInvocationInput {
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    attempt_id: &attempt_id,
                    attempt_no,
                    view,
                })?;
                let start_commit = RuntimeMutationMiddleware::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                store.append_prepared_typed_commit(start_commit)?;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_stream = store.load_run_stream(run_id);
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view: &latest_view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output =
            RuntimeMutationMiddleware::prepare_runner_output(RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view: &latest_view,
                output,
            })?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store.append_prepared_typed_commit(terminal_output.commit)?;
        Ok(())
    }

    async fn run_started_framework_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        input: FrameworkNodeAttemptInput<'_>,
    ) -> Result<()> {
        let FrameworkNodeAttemptInput {
            runtime_spec,
            run_id,
            view,
            runnable,
            descriptor,
            output_cell,
            binding,
        } = input;
        let node = runnable.node;
        let AttemptPlan::StartNew = runnable.attempt else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "framework lifecycle node {} attempt was split across commits",
                node.node_id
            )));
        };
        let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = RuntimeMutationMiddleware::prepare_started_runner_output(
            RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view,
                output,
            },
            attempt_no,
        )?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store.append_prepared_typed_commit(terminal_output.commit)?;
        Ok(())
    }

    async fn run_node_attempt_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
            )
        ) {
            self.run_started_framework_node_attempt_async(
                store,
                FrameworkNodeAttemptInput {
                    runtime_spec,
                    run_id,
                    view,
                    runnable,
                    descriptor,
                    output_cell,
                    binding,
                },
            )
            .await?;
            return Ok(());
        }
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew => {
                let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                prepare_runner_invocation(RunnerInvocationInput {
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    attempt_id: &attempt_id,
                    attempt_no,
                    view,
                })?;
                let start_commit = RuntimeMutationMiddleware::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                store
                    .append_prepared_typed_commit(start_commit)
                    .await
                    .map_err(async_store_error)?;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view: &latest_view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output =
            RuntimeMutationMiddleware::prepare_runner_output(RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view: &latest_view,
                output,
            })?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
            .map_err(async_store_error)?;
        Ok(())
    }

    async fn run_started_framework_node_attempt_async<
        S: store::AsyncTypedRunEventStore + ?Sized,
    >(
        &self,
        store: &S,
        input: FrameworkNodeAttemptInput<'_>,
    ) -> Result<()> {
        let FrameworkNodeAttemptInput {
            runtime_spec,
            run_id,
            view,
            runnable,
            descriptor,
            output_cell,
            binding,
        } = input;
        let node = runnable.node;
        let AttemptPlan::StartNew = runnable.attempt else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "framework lifecycle node {} attempt was split across commits",
                node.node_id
            )));
        };
        let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = RuntimeMutationMiddleware::prepare_started_runner_output(
            RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view,
                output,
            },
            attempt_no,
        )?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
            .map_err(async_store_error)?;
        Ok(())
    }

    async fn stage_prepared_artifacts(&self, artifacts: &[PreparedStagedArtifact]) -> Result<()> {
        for artifact in artifacts {
            self.artifact_stager
                .stage_verified_artifact(artifact.bytes.clone(), artifact.evidence.clone())
                .await?;
        }
        Ok(())
    }
}

fn public_output_is_produced(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> bool {
    let public_schema_id = &runtime_spec.spec().public_outputs.public_schema_id;
    matches!(
        projections.public_output(public_schema_id),
        Some(store::PublicOutputProjection::Produced { .. })
    )
}

fn build_retention_manifest_artifact_with_producer(
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

fn runner_output_commit_key(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<store::CommitKey> {
    let mut fragments = BTreeSet::new();
    for payload in payloads {
        fragments.insert(runner_output_commit_fragment(payload));
    }
    if fragments.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let suffix = content_digest_json(serde_json::json!({
        "fragments": fragments.into_iter().collect::<Vec<_>>(),
    }))?;
    Ok(store::CommitKey::new(format!(
        "attempt-output:{}:{}:{}",
        node.node_id, attempt_id, suffix
    ))?)
}

fn runner_output_commit_fragment(payload: &events::KernelEventPayload) -> String {
    match payload {
        events::KernelEventPayload::StateAttemptCompleted(payload) => {
            format!("completed:{}", payload.output_cell_id)
        }
        events::KernelEventPayload::StateAttemptFailed(_) => "failed".to_owned(),
        events::KernelEventPayload::CellProduced(payload) => {
            format!("cell-produced:{}", payload.cell_id)
        }
        events::KernelEventPayload::CellSkipped(payload) => {
            format!("cell-skipped:{}", payload.cell_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => format!("fact:{}", payload.fact_key),
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            format!("artifact:{}", payload.artifact_ref.artifact_id)
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            format!("public-output:{}", payload.public_schema_id)
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            format!("public-output-failed:{}", payload.public_schema_id)
        }
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            format!("sidefx-intent:{}", payload.ledger_key)
        }
        events::KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx-claim:{}:{}:{}",
            payload.ledger_key, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx-claim-takeover:{}:{}:{}",
            payload.ledger_key, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx-prepared:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx-started:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx-not-submitted:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx-submission:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx-submission-unknown:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx-receipt:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx-confirmation:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!("sidefx-ambiguous:{}", payload.ledger_key)
        }
        events::KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx-failed:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::RetentionManifestProjected(payload) => {
            format!(
                "retention-manifest:{}:{}",
                payload.manifest_seq, payload.manifest_digest
            )
        }
        events::KernelEventPayload::RetentionRefsAppended(payload) => format!(
            "retention-refs:{}:{}",
            retention_reason_str(payload.reason),
            payload.refs.len()
        ),
        events::KernelEventPayload::RunStarted(_)
        | events::KernelEventPayload::RunCompleted(_)
        | events::KernelEventPayload::StateAttemptStarted(_) => "scheduler-owned".to_owned(),
    }
}

fn validate_staged_artifacts(
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    staged_artifacts: &[StagedArtifact],
) -> Result<Vec<ValidatedStagedArtifact>> {
    let mut by_artifact = BTreeMap::<ArtifactId, ValidatedStagedArtifact>::new();
    for staged in staged_artifacts {
        let handle = staged.handle();
        if handle.run_id() != run_id
            || handle.node_id() != &node.node_id
            || handle.attempt_id() != attempt_id
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} outside its sealed attempt",
                node.node_id,
                handle.evidence().artifact_id
            )));
        }
        if let Some(bytes) = staged.bytes() {
            verify_artifact_bytes(bytes, handle.evidence())?;
        }
        if let Some(existing) = by_artifact.get(&handle.evidence().artifact_id) {
            if existing.evidence != *handle.evidence() || existing.binding != *handle.binding() {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged conflicting evidence for artifact {}",
                    node.node_id,
                    handle.evidence().artifact_id
                )));
            }
            continue;
        }
        by_artifact.insert(
            handle.evidence().artifact_id.clone(),
            ValidatedStagedArtifact {
                evidence: handle.evidence().clone(),
                binding: handle.binding().clone(),
                bytes: staged.bytes().map(ToOwned::to_owned),
            },
        );
    }
    Ok(by_artifact.into_values().collect())
}

fn framework_retention_manifest_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    pre_projection_stream: &[store::KernelEventEnvelope],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Result<Option<RetentionManifestArtifact>> {
    let manifests = staged_artifacts
        .iter()
        .filter(|artifact| artifact.binding == StagedArtifactBindingKind::RetentionManifest)
        .collect::<Vec<_>>();
    if manifests.is_empty() {
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        ) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "retention framework node {} did not stage a retention manifest",
                node.node_id
            )));
        }
        return Ok(None);
    }
    if !matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    ) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} staged retention manifest outside framework retention authority",
            node.node_id
        )));
    }
    if manifests.len() != 1 {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged multiple retention manifests",
            node.node_id
        )));
    }
    let staged = manifests[0];
    let Some(bytes) = staged.bytes.as_deref() else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged manifest without bytes",
            node.node_id
        )));
    };
    let expected = build_retention_manifest_artifact_with_producer(
        runtime_spec,
        run_id,
        pre_projection_stream,
        Some(node.node_id.clone()),
    )?;
    if staged.evidence != expected.evidence || bytes != expected.bytes.as_bytes() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged manifest outside authoritative stream",
            node.node_id
        )));
    }
    Ok(Some(expected))
}

fn retention_manifest_payloads(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    manifest: RetentionManifestArtifact,
) -> Vec<events::KernelEventPayload> {
    let manifest_ref = retention_ref_for_artifact(&manifest.evidence);
    vec![
        events::KernelEventPayload::RetentionManifestProjected(
            events::RetentionManifestProjected {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                manifest_seq: manifest.manifest_seq,
                manifest_digest: manifest.evidence.digest.clone(),
                previous_manifest_digest: manifest.previous_manifest_digest,
                manifest_artifact_id: manifest.evidence.artifact_id.clone(),
            },
        ),
        events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            refs: vec![manifest_ref],
            reason: events::RetentionReason::ManifestProjection,
        }),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedStagedArtifact {
    evidence: store::ArtifactEvidenceRef,
    binding: StagedArtifactBindingKind,
    bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StagedArtifactRequirement {
    artifact_id: ArtifactId,
    digest: ContentDigest,
    byte_len: Option<u64>,
    media_type: Option<spec::MediaType>,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<SemanticTypeId>,
    role: events::ArtifactRole,
    binding: StagedArtifactBindingKind,
}

fn validate_staged_artifact_payload_bindings(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Result<()> {
    let requirements = staged_payload_artifact_requirements(node, attempt_id, payloads)?;
    let mut requirements_by_artifact: BTreeMap<ArtifactId, &StagedArtifactRequirement> =
        BTreeMap::new();
    for requirement in &requirements {
        if let Some(existing) = requirements_by_artifact.get(&requirement.artifact_id) {
            if !staged_artifact_requirements_are_compatible(existing, requirement) {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} returned conflicting typed payload requirements for artifact {}",
                    node.node_id, requirement.artifact_id
                )));
            }
        }
        requirements_by_artifact.insert(requirement.artifact_id.clone(), requirement);
    }
    let staged_by_artifact = staged_artifacts
        .iter()
        .map(|artifact| (artifact.evidence.artifact_id.clone(), artifact))
        .collect::<BTreeMap<_, _>>();

    for staged in staged_artifacts {
        let Some(requirement) = requirements_by_artifact.get(&staged.evidence.artifact_id) else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} without typed payload reference",
                node.node_id, staged.evidence.artifact_id
            )));
        };
        validate_staged_artifact_requirement(node, &staged.evidence, &staged.binding, requirement)?;
    }

    for requirement in requirements {
        let Some(staged) = staged_by_artifact.get(&requirement.artifact_id) else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} referenced artifact {} without staged artifact",
                node.node_id, requirement.artifact_id
            )));
        };
        validate_staged_artifact_requirement(
            node,
            &staged.evidence,
            &staged.binding,
            &requirement,
        )?;
    }
    Ok(())
}

fn validate_staged_artifact_requirement(
    node: &spec::NodeSpec,
    evidence: &store::ArtifactEvidenceRef,
    binding: &StagedArtifactBindingKind,
    requirement: &StagedArtifactRequirement,
) -> Result<()> {
    if evidence.artifact_id != requirement.artifact_id
        || evidence.digest != requirement.digest
        || requirement
            .byte_len
            .is_some_and(|byte_len| evidence.byte_len != byte_len)
        || requirement
            .media_type
            .as_ref()
            .is_some_and(|media_type| &evidence.media_type != media_type)
        || requirement
            .schema_id
            .as_ref()
            .is_some_and(|schema_id| evidence.schema_id.as_ref() != Some(schema_id))
        || requirement
            .semantic_type_id
            .as_ref()
            .is_some_and(|semantic_type_id| {
                evidence.semantic_type_id.as_ref() != Some(semantic_type_id)
            })
        || evidence.producer_node_id.as_ref() != Some(&node.node_id)
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != requirement.role
        || binding != &requirement.binding
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} staged artifact {} does not match typed payload binding",
            node.node_id, evidence.artifact_id
        )));
    }
    Ok(())
}

fn staged_artifact_requirements_are_compatible(
    left: &StagedArtifactRequirement,
    right: &StagedArtifactRequirement,
) -> bool {
    left.artifact_id == right.artifact_id
        && left.digest == right.digest
        && optional_requirements_are_compatible(left.byte_len.as_ref(), right.byte_len.as_ref())
        && optional_requirements_are_compatible(left.media_type.as_ref(), right.media_type.as_ref())
        && optional_requirements_are_compatible(left.schema_id.as_ref(), right.schema_id.as_ref())
        && optional_requirements_are_compatible(
            left.semantic_type_id.as_ref(),
            right.semantic_type_id.as_ref(),
        )
        && left.role == right.role
        && left.binding == right.binding
}

fn optional_requirements_are_compatible<T: Eq>(left: Option<&T>, right: Option<&T>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn staged_payload_artifact_requirements(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<Vec<StagedArtifactRequirement>> {
    let mut requirements = Vec::new();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::FactRecorded(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(StagedArtifactRequirement {
                    artifact_id: payload.artifact_id.clone(),
                    digest: payload.response_hash.clone(),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.response_schema_id.clone()),
                    semantic_type_id: None,
                    role: events::ArtifactRole::FactResponse,
                    binding: StagedArtifactBindingKind::FactResponse,
                });
            }
            events::KernelEventPayload::CellProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(StagedArtifactRequirement {
                    artifact_id: payload.artifact_id.clone(),
                    digest: payload.content_digest.clone(),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.schema_id.clone()),
                    semantic_type_id: Some(payload.semantic_type_id.clone()),
                    role: events::ArtifactRole::StateOutput,
                    binding: StagedArtifactBindingKind::StateOutput,
                });
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(artifact_id) = &payload.rendered_artifact_id {
                    requirements.push(StagedArtifactRequirement {
                        artifact_id: artifact_id.clone(),
                        digest: payload.rendered_digest.clone(),
                        byte_len: None,
                        media_type: None,
                        schema_id: Some(payload.public_schema_id.clone()),
                        semantic_type_id: None,
                        role: events::ArtifactRole::PublicOutput,
                        binding: StagedArtifactBindingKind::PublicOutput,
                    });
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(ref diagnostic) = payload.error.diagnostic_ref {
                    push_staged_event_artifact_requirement(
                        node,
                        diagnostic,
                        StagedArtifactBindingKind::RedactedDiagnostic,
                        &mut requirements,
                    )?;
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(ref diagnostic) = payload.error.diagnostic_ref {
                    push_staged_event_artifact_requirement(
                        node,
                        diagnostic,
                        StagedArtifactBindingKind::RedactedDiagnostic,
                        &mut requirements,
                    )?;
                }
            }
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.intent_artifact_id.clone(),
                    payload.intent_hash.clone(),
                    Some(payload.intent_schema_id.clone()),
                    events::ArtifactRole::SideEffectIntent,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Intent,
                ));
            }
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let (Some(artifact_id), Some(hash)) =
                    (&payload.prepared_artifact_id, &payload.prepared_hash)
                {
                    requirements.push(side_effect_artifact_requirement(
                        artifact_id.clone(),
                        hash.clone(),
                        None,
                        events::ArtifactRole::PreparedInvocation,
                        payload.ledger_key.clone(),
                        payload.invocation_epoch,
                        StagedSideEffectArtifactPhase::PreparedInvocation,
                    ));
                }
            }
            events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.proof_artifact_id.clone(),
                    payload.proof_hash.clone(),
                    Some(payload.proof_schema_id.clone()),
                    events::ArtifactRole::NotSubmittedProof,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::NotSubmittedProof,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.submission_artifact_id.clone(),
                    payload.submission_hash.clone(),
                    Some(payload.submission_schema_id.clone()),
                    events::ArtifactRole::Submission,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Submission,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    Some(payload.evidence_schema_id.clone()),
                    events::ArtifactRole::SubmissionUnknownEvidence,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::SubmissionUnknownEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.receipt_artifact_id.clone(),
                    payload.receipt_hash.clone(),
                    Some(payload.receipt_schema_id.clone()),
                    events::ArtifactRole::Receipt,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Receipt,
                ));
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.confirmation_artifact_id.clone(),
                    payload.confirmation_hash.clone(),
                    Some(payload.confirmation_schema_id.clone()),
                    events::ArtifactRole::Confirmation,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Confirmation,
                ));
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    Some(payload.evidence_schema_id.clone()),
                    events::ArtifactRole::AmbiguityEvidence,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::AmbiguityEvidence,
                ));
            }
            events::KernelEventPayload::RunStarted(_)
            | events::KernelEventPayload::ArtifactReferenced(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_)
            | events::KernelEventPayload::StateAttemptCompleted(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {}
        }
    }
    Ok(requirements)
}

fn staged_artifact_reference_payloads(
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Vec<events::KernelEventPayload> {
    let existing_refs = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(&node.node_id)
                    && payload.attempt_id.as_ref() == Some(attempt_id) =>
            {
                Some(payload.artifact_ref.artifact_id.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut refs = Vec::new();
    for artifact in staged_artifacts {
        if staged_artifact_binding_kind(artifact.evidence.artifact_role).is_none() {
            continue;
        }
        if existing_refs.contains(&artifact.evidence.artifact_id) {
            continue;
        }
        let Some(schema_id) = artifact.evidence.schema_id.clone() else {
            continue;
        };
        refs.push(events::KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: spec_hash.clone(),
                node_id: Some(node.node_id.clone()),
                attempt_id: Some(attempt_id.clone()),
                artifact_ref: events::ArtifactEvidenceRef {
                    artifact_id: artifact.evidence.artifact_id.clone(),
                    role: artifact.evidence.artifact_role,
                    schema_id,
                    semantic_type_id: artifact.evidence.semantic_type_id.clone(),
                    content_digest: artifact.evidence.digest.clone(),
                    byte_len: artifact.evidence.byte_len,
                    media_type: artifact.evidence.media_type.clone(),
                },
            },
        ));
    }
    refs
}

fn push_staged_event_artifact_requirement(
    node: &spec::NodeSpec,
    artifact: &events::ArtifactEvidenceRef,
    binding: StagedArtifactBindingKind,
    requirements: &mut Vec<StagedArtifactRequirement>,
) -> Result<()> {
    let role = staged_artifact_binding_role(&binding);
    if artifact.role != role {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} diagnostic artifact role {} does not match staged binding",
            node.node_id,
            artifact_role_name(artifact.role)
        )));
    }
    requirements.push(StagedArtifactRequirement {
        artifact_id: artifact.artifact_id.clone(),
        digest: artifact.content_digest.clone(),
        byte_len: Some(artifact.byte_len),
        media_type: Some(artifact.media_type.clone()),
        schema_id: Some(artifact.schema_id.clone()),
        semantic_type_id: artifact.semantic_type_id.clone(),
        role: artifact.role,
        binding,
    });
    Ok(())
}

fn side_effect_artifact_requirement(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: Option<SchemaId>,
    role: events::ArtifactRole,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    phase: StagedSideEffectArtifactPhase,
) -> StagedArtifactRequirement {
    StagedArtifactRequirement {
        artifact_id,
        digest,
        byte_len: None,
        media_type: None,
        schema_id,
        semantic_type_id: None,
        role,
        binding: StagedArtifactBindingKind::SideEffectEvidence {
            ledger_key,
            invocation_epoch,
            phase,
        },
    }
}

fn bind_staged_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    required_artifacts: &[store::ArtifactEvidenceRef],
    staged: Vec<StagedRetentionRefs>,
) -> Result<Vec<events::KernelEventPayload>> {
    let artifact_evidence = required_artifacts
        .iter()
        .map(|artifact| (artifact.artifact_id.clone(), artifact))
        .collect::<BTreeMap<_, _>>();
    let mut payloads = Vec::with_capacity(staged.len());
    for staged_refs in staged {
        let reason = staged_refs.reason;
        validate_staged_retention_reason(runtime_spec, node, &artifact_evidence, &staged_refs)?;
        if staged_refs.refs.is_empty() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged empty retention refs",
                node.node_id
            )));
        }
        for retention_ref in &staged_refs.refs {
            let Some(artifact) = artifact_evidence.get(&retention_ref.artifact_id) else {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged retention for artifact {} without staged artifact evidence",
                    node.node_id, retention_ref.artifact_id
                )));
            };
            if artifact.digest != retention_ref.content_digest
                || artifact.artifact_role != retention_ref.role
            {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged retention evidence for artifact {} does not match artifact evidence",
                    node.node_id, retention_ref.artifact_id
                )));
            }
        }
        payloads.push(events::KernelEventPayload::RetentionRefsAppended(
            events::RetentionRefsAppended {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                refs: staged_refs.refs,
                reason,
            },
        ));
    }
    Ok(payloads)
}

fn validate_staged_retention_reason(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    artifact_evidence: &BTreeMap<ArtifactId, &store::ArtifactEvidenceRef>,
    staged_refs: &StagedRetentionRefs,
) -> Result<()> {
    match staged_refs.reason {
        events::RetentionReason::RunStarted | events::RetentionReason::ManifestProjection => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged middleware-owned retention reason {}",
                node.node_id,
                retention_reason_str(staged_refs.reason)
            )));
        }
        events::RetentionReason::PublicOutput => {
            if !matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            ) {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged public-output retention outside sealed framework renderer",
                    node.node_id
                )));
            }
            let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "public-output framework node {} references missing output cell {}",
                    node.node_id, node.output_cell
                ))
            })?;
            for retention_ref in &staged_refs.refs {
                let artifact = artifact_evidence
                    .get(&retention_ref.artifact_id)
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunnerOutput(format!(
                            "node {} staged public-output retention for artifact {} without staged artifact evidence",
                            node.node_id, retention_ref.artifact_id
                        ))
                    })?;
                let framework_artifact = artifact.producer_node_id.as_ref() == Some(&node.node_id)
                    && matches!(
                        artifact.artifact_role,
                        events::ArtifactRole::StateOutput | events::ArtifactRole::PublicOutput
                    )
                    && (artifact.artifact_role != events::ArtifactRole::StateOutput
                        || artifact.schema_id.as_ref() == Some(&output_cell.schema_id));
                if !framework_artifact {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} staged public-output retention for non-framework artifact {}",
                        node.node_id, retention_ref.artifact_id
                    )));
                }
            }
        }
        events::RetentionReason::RuntimeEvidence => {}
    }
    Ok(())
}

fn runner_output_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    projections: &store::ProjectionSnapshot,
    payloads: &[events::KernelEventPayload],
    require_existing_attempt: bool,
) -> Result<store::CommitPreconditions> {
    let mut preconditions = store::CommitPreconditions {
        required_run_state: store::RequiredRunState::NotCompleted,
        required_cell_states: vec![store::CellStatePrecondition {
            cell_id: node.output_cell.clone(),
            required: store::RequiredCellState::Absent,
        }],
        required_public_output_absent: matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        ),
        ..store::CommitPreconditions::default()
    };
    if require_existing_attempt {
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))?);
    } else {
        preconditions
            .required_cell_states
            .extend(node_cell_preconditions(runtime_spec, node)?);
    }
    if matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    ) {
        let completion = run_completion_evidence(runtime_spec, projections)?;
        let retention_manifest = projected_retention_manifest(run_id, projections)?;
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "public_output:{}",
                completion.public_output_schema_id
            ))?);
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "retention:{}:manifest:{}",
                run_id, retention_manifest.manifest_seq
            ))?);
    }

    if node.side_effect.is_none() {
        return Ok(preconditions);
    }

    let mut requires_terminal_confirmation = false;
    let prepared_in_batch = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                Some(payload.ledger_key.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::Absent,
                    },
                );
            }
            events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
                if !prepared_in_batch.contains(&payload.ledger_key) {
                    preconditions.required_side_effect_states.push(
                        store::SideEffectStatePrecondition {
                            ledger_key: payload.ledger_key.clone(),
                            required: store::RequiredSideEffectState::InvocationPrepared,
                        },
                    );
                }
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::SubmissionResult,
                    },
                );
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::ReceiptObserved,
                    },
                );
            }
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::StateAttemptCompleted(_) => {
                requires_terminal_confirmation = true;
            }
            _ => {}
        }
    }

    if requires_terminal_confirmation {
        let projection = side_effect_projection_for_attempt(projections, node, attempt_id)?
            .ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} attempted output without ledger evidence",
                    node.node_id
                ))
            })?;
        preconditions
            .required_side_effect_states
            .push(store::SideEffectStatePrecondition {
                ledger_key: projection.ledger_key.clone(),
                required: store::RequiredSideEffectState::ConfirmationObserved,
            });
    }

    Ok(preconditions)
}

fn next_runnable_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
) -> Result<Option<RunnableNode<'a>>> {
    if view
        .projections
        .side_effects()
        .any(|(_, projection)| matches!(projection.phase, store::SideEffectPhase::Ambiguous { .. }))
    {
        return Ok(None);
    }
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        let Some(attempt) = attempt_plan(runtime_spec, node, view)? else {
            continue;
        };
        if node_inputs_ready(runtime_spec, node, view)? {
            return Ok(Some(RunnableNode { node, attempt }));
        }
    }
    Ok(None)
}

fn attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if node.side_effect.is_some() {
        side_effect_attempt_plan(runtime_spec, node, view)
    } else {
        non_side_effect_attempt_plan(runtime_spec, node, view)
    }
}

fn non_side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view.projections.cell_terminal(&node.output_cell) {
        validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        return Ok(None);
    }

    let mut started = None::<(AttemptId, u32)>;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        match &projection.status {
            store::AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "started attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "node {} attempt {} completed output cell {} without terminal cell authority",
                    node.node_id, attempt_id, output_cell_id
                )));
            }
            store::AttemptStatus::Failed { retryable, .. } => {
                if !*retryable {
                    return Ok(None);
                }
            }
        }
    }

    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew))
    }
}

fn side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view.projections.cell_terminal(&node.output_cell) {
        let attempt_id = validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        validate_side_effect_terminal_evidence(&view.projections, node, &attempt_id)?;
        return Ok(None);
    }

    let mut started = None::<(AttemptId, u32)>;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        match &projection.status {
            store::AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "started side-effect attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                if let Some(projection) =
                    side_effect_projection_for_attempt(&view.projections, node, attempt_id)?
                {
                    match projection.phase {
                        store::SideEffectPhase::Ambiguous { .. } => return Ok(None),
                        store::SideEffectPhase::Failed { .. } => {
                            return Err(RuntimeError::InvalidRunStream(format!(
                                "side-effect ledger {} failed while attempt {} for node {} remained started",
                                projection.ledger_key, attempt_id, node.node_id
                            )));
                        }
                        _ => {}
                    }
                }
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal side-effect attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                if view.projections.cell_terminal(output_cell_id).is_none() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "side-effect node {} attempt {} completed without terminal output cell {}",
                        node.node_id, attempt_id, output_cell_id
                    )));
                }
            }
            store::AttemptStatus::Failed { retryable, .. } => {
                if !*retryable {
                    return Ok(None);
                }
            }
        }
    }

    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew))
    }
}

fn validate_terminal_cell_has_completed_attempt(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    terminal: &store::CellTerminalProjection,
) -> Result<AttemptId> {
    let certified = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let (terminal_node_id, terminal_attempt_id, schema_id, semantic_type_id) = match terminal {
        store::CellTerminalProjection::Produced {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        }
        | store::CellTerminalProjection::Skipped {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        } => (node_id, attempt_id, schema_id, semantic_type_id),
    };
    if terminal_node_id != &node.node_id
        || certified.producer != spec::CellProducer::Node(node.node_id.clone())
        || schema_id != &certified.schema_id
        || semantic_type_id != &certified.semantic_type_id
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} is not certified terminal evidence for node {}",
            node.output_cell, node.node_id
        )));
    }
    match projections.attempt(&node.node_id, terminal_attempt_id) {
        Some(store::AttemptProjection {
            status: store::AttemptStatus::Completed { output_cell_id },
            ..
        }) if output_cell_id == &node.output_cell => Ok(terminal_attempt_id.clone()),
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} for node {} lacks matching completed attempt {}",
            node.output_cell, node.node_id, terminal_attempt_id
        ))),
    }
}

fn side_effect_projection_for_attempt<'a>(
    projections: &'a store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<Option<&'a store::SideEffectProjection>> {
    let mut found = None;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == node.node_id
            && projection.intent.attempt_id == *attempt_id
            && found.replace(projection).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledger projections",
                node.node_id, attempt_id
            )));
        }
    }
    Ok(found)
}

fn validate_side_effect_terminal_evidence(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output without ledger evidence",
            node.node_id, attempt_id
        )));
    };
    if matches!(
        projection.phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node.node_id, attempt_id
        )))
    }
}

fn recorded_facts_for_attempt(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<RecordedFacts> {
    let mut facts = BTreeMap::new();
    for ((fact_node_id, fact_attempt_id, fact_key), projection) in projections.facts() {
        if fact_node_id != node_id || fact_attempt_id != attempt_id {
            continue;
        }
        if projection.node_id != *node_id
            || projection.attempt_id != *attempt_id
            || projection.fact_key != *fact_key
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "fact projection {} for node {} attempt {} is internally inconsistent",
                fact_key, node_id, attempt_id
            )));
        }
        facts.insert(
            fact_key.clone(),
            RecordedFact {
                fact_key: fact_key.clone(),
                request_schema_id: projection.request_schema_id.clone(),
                request_hash: projection.request_hash.clone(),
                response_schema_id: projection.response_schema_id.clone(),
                response_hash: projection.response_hash.clone(),
                artifact_id: projection.artifact_id.clone(),
                capability_kind: projection.capability_kind.clone(),
                capability_version: projection.capability_version.clone(),
                adapter_kind: projection.adapter_kind.clone(),
                adapter_version: projection.adapter_version.clone(),
            },
        );
    }
    Ok(RecordedFacts { facts })
}

fn node_inputs_ready(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<bool> {
    let input_cells = runtime_spec.validate_input_binding(&node.input_bindings.root)?;
    for cell_id in input_cells {
        let cell = runtime_spec.cell(&cell_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} input cell {} is missing",
                node.node_id, cell_id
            ))
        })?;
        match &cell.producer {
            spec::CellProducer::Seed(_) => {
                if !view.seed_cells.contains_key(&cell_id) {
                    return Ok(false);
                }
            }
            spec::CellProducer::Node(_) => {
                if view.projections.cell_terminal(&cell_id).is_none() {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn node_cell_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
) -> Result<Vec<store::CellStatePrecondition>> {
    let mut preconditions = Vec::new();
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        let cell = runtime_spec
            .cell(&cell_id)
            .expect("validated input binding cell exists");
        if matches!(cell.producer, spec::CellProducer::Node(_)) {
            preconditions.push(store::CellStatePrecondition {
                cell_id,
                required: store::RequiredCellState::Terminal,
            });
        }
    }
    Ok(preconditions)
}

fn materialize_inputs(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputs> {
    Ok(MaterializedInputs {
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        root: materialize_input_node(runtime_spec, &node.input_bindings.root, view)?,
    })
}

fn materialize_input_node(
    runtime_spec: &CertifiedRuntimeSpec,
    input: &spec::InputBindingNodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputNode> {
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(MaterializedInputNode::Unit),
        spec::InputBindingNodeSpec::Cell(cell) => Ok(MaterializedInputNode::Cell(Box::new(
            materialize_cell(runtime_spec, cell, view)?,
        ))),
        spec::InputBindingNodeSpec::Tuple(elements) => Ok(MaterializedInputNode::Tuple(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Struct(fields) => Ok(MaterializedInputNode::Struct(
            fields
                .iter()
                .map(|field| {
                    Ok(NamedMaterializedInput {
                        field_path: field.field_path.clone(),
                        node: materialize_input_node(runtime_spec, &field.node, view)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Vec { elements, .. } => Ok(MaterializedInputNode::Vec(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            Ok(MaterializedInputNode::NonEmptyVec(
                elements
                    .iter()
                    .map(|element| materialize_input_node(runtime_spec, element, view))
                    .collect::<Result<Vec<_>>>()?,
            ))
        }
    }
}

fn materialize_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    cell: &spec::InputBindingCellSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedCell> {
    let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {} is not certified by the spec",
            cell.cell_id
        ))
    })?;
    if certified.schema_id != cell.schema_id
        || certified.semantic_type_id != cell.semantic_type_id
        || certified.value_lineage != cell.value_lineage
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "input cell {} metadata does not match certified cell",
            cell.cell_id
        )));
    }
    let terminal = match &certified.producer {
        spec::CellProducer::Seed(seed_id) => {
            let seed = view.seed_cells.get(&cell.cell_id).ok_or_else(|| {
                RuntimeError::InputMaterialization(format!(
                    "seed cell {} has no RunStarted evidence",
                    cell.cell_id
                ))
            })?;
            let seed_artifact =
                committed_input_artifact(view, &cell.cell_id, &seed.seed_artifact.artifact_id)?;
            if seed_artifact.evidence.digest != seed.digest
                || seed_artifact.evidence.byte_len != seed.seed_artifact.byte_len
                || seed_artifact.evidence.media_type != seed.seed_artifact.media_type
                || seed_artifact.evidence.schema_id.as_ref() != Some(&seed.schema_id)
                || seed_artifact.evidence.semantic_type_id.as_ref() != Some(&seed.semantic_type_id)
                || seed_artifact.evidence.producer_node_id.is_some()
                || seed_artifact.evidence.producer_seed_id.as_ref() != Some(seed_id)
                || seed_artifact.evidence.artifact_role != events::ArtifactRole::SeedInput
            {
                return Err(RuntimeError::InputMaterialization(format!(
                    "seed cell {} committed artifact evidence does not match certified seed",
                    cell.cell_id
                )));
            }
            MaterializedCellTerminal::Seed {
                seed_id: seed_id.clone(),
                artifact_id: seed.seed_artifact.artifact_id.clone(),
                content_digest: seed.digest.clone(),
            }
        }
        spec::CellProducer::Node(_) => {
            let projection = view
                .projections
                .cell_terminal(&cell.cell_id)
                .ok_or_else(|| {
                    RuntimeError::InputMaterialization(format!(
                        "input cell {} is not terminal",
                        cell.cell_id
                    ))
                })?;
            match projection {
                store::CellTerminalProjection::Produced {
                    node_id,
                    attempt_id,
                    schema_id,
                    semantic_type_id,
                    artifact_id,
                    content_digest,
                    ..
                } => {
                    if certified.producer != spec::CellProducer::Node(node_id.clone())
                        || schema_id != &cell.schema_id
                        || semantic_type_id != &cell.semantic_type_id
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    match view.projections.attempt(node_id, attempt_id) {
                        Some(store::AttemptProjection {
                            status: store::AttemptStatus::Completed { output_cell_id },
                            ..
                        }) if output_cell_id == &cell.cell_id => {}
                        _ => {
                            return Err(RuntimeError::InputMaterialization(format!(
                                "produced cell {} is not backed by a completed producer attempt",
                                cell.cell_id
                            )));
                        }
                    }
                    let artifact = committed_input_artifact(view, &cell.cell_id, artifact_id)?;
                    if artifact.evidence.digest != *content_digest
                        || artifact.evidence.schema_id.as_ref() != Some(schema_id)
                        || artifact.evidence.semantic_type_id.as_ref() != Some(semantic_type_id)
                        || artifact.evidence.producer_node_id.as_ref() != Some(node_id)
                        || artifact.evidence.producer_seed_id.is_some()
                        || artifact.evidence.artifact_role != events::ArtifactRole::StateOutput
                        || artifact.attempt_id.as_ref() != Some(attempt_id)
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} committed artifact evidence does not match terminal projection",
                            cell.cell_id
                        )));
                    }
                    MaterializedCellTerminal::Produced {
                        artifact_id: artifact_id.clone(),
                        content_digest: content_digest.clone(),
                    }
                }
                store::CellTerminalProjection::Skipped {
                    node_id,
                    attempt_id,
                    schema_id,
                    semantic_type_id,
                    skip_reason,
                    ..
                } => {
                    if cell.required_terminal == spec::RequiredTerminal::ProducedOnly {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "input cell {} requires produced terminal but was skipped",
                            cell.cell_id
                        )));
                    }
                    if certified.producer != spec::CellProducer::Node(node_id.clone())
                        || schema_id != &cell.schema_id
                        || semantic_type_id != &cell.semantic_type_id
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "skipped cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    match view.projections.attempt(node_id, attempt_id) {
                        Some(store::AttemptProjection {
                            status: store::AttemptStatus::Completed { output_cell_id },
                            ..
                        }) if output_cell_id == &cell.cell_id => {}
                        _ => {
                            return Err(RuntimeError::InputMaterialization(format!(
                                "skipped cell {} is not backed by a completed producer attempt",
                                cell.cell_id
                            )));
                        }
                    }
                    MaterializedCellTerminal::Skipped {
                        skip_reason: skip_reason.clone(),
                    }
                }
            }
        }
    };
    Ok(MaterializedCell {
        cell_id: cell.cell_id.clone(),
        schema_id: cell.schema_id.clone(),
        semantic_type_id: cell.semantic_type_id.clone(),
        value_lineage: cell.value_lineage.clone(),
        terminal,
    })
}

fn validate_seed_cells(
    runtime_spec: &CertifiedRuntimeSpec,
    seed_cells: &[events::SeedCellRef],
) -> Result<BTreeMap<CellId, events::SeedCellRef>> {
    let mut by_seed = BTreeMap::<_, _>::new();
    let mut by_cell = BTreeMap::<_, _>::new();
    for seed in seed_cells {
        if by_seed.insert(seed.seed_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed evidence for {}",
                seed.seed_id
            )));
        }
        if by_cell.insert(seed.cell_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed cell evidence for {}",
                seed.cell_id
            )));
        }
    }
    for declared in &runtime_spec.spec().seeds {
        let seed = by_seed.get(&declared.seed_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing RunStarted seed evidence for {}",
                declared.seed_id
            ))
        })?;
        if seed.cell_id != declared.cell_id
            || seed.scope_id != declared.scope_id
            || seed.schema_id != declared.schema_id
            || seed.semantic_type_id != declared.semantic_type_id
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunStarted seed {} metadata does not match certified seed",
                declared.seed_id
            )));
        }
        if let Some(required_digest) = &declared.required_digest {
            if &seed.digest != required_digest {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "RunStarted seed {} digest does not match certified required digest",
                    declared.seed_id
                )));
            }
        }
        if seed.seed_artifact.role != events::ArtifactRole::SeedInput
            || seed.seed_artifact.schema_id != declared.schema_id
            || seed.seed_artifact.semantic_type_id.as_ref() != Some(&declared.semantic_type_id)
            || seed.seed_artifact.content_digest != seed.digest
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunStarted seed {} artifact evidence does not match certified seed",
                declared.seed_id
            )));
        }
    }
    if by_seed.len() != runtime_spec.spec().seeds.len() {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted contains seed evidence not certified by the spec".to_owned(),
        ));
    }
    Ok(by_cell)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoricalSideEffectPhase {
    IntentPersisted,
    Claimed,
    InvocationPrepared,
    InvocationStarted,
    NotSubmittedProven,
    SubmissionObserved,
    SubmissionUnknown,
    ReceiptObserved,
    ConfirmationObserved,
    Ambiguous,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoricalSideEffectLedger {
    node_id: NodeId,
    attempt_id: AttemptId,
    phase: HistoricalSideEffectPhase,
}

fn validate_historical_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    let mut available_cells = BTreeSet::<CellId>::new();
    let mut active_attempts = BTreeSet::<(NodeId, AttemptId)>::new();
    let mut side_effect_ledgers =
        BTreeMap::<events::SideEffectLedgerKey, HistoricalSideEffectLedger>::new();
    let mut seen_run_started = false;
    let mut produced_public_output = None::<events::PublicOutputCompletionEvidence>;
    let mut retention_manifest_projected_seq = None::<store::StreamSeq>;
    let mut completed = false;
    for event in stream {
        if completed {
            return Err(RuntimeError::InvalidRunStream(
                "run stream contains events after RunCompleted".to_owned(),
            ));
        }
        if !seen_run_started
            && !matches!(event.payload(), events::KernelEventPayload::RunStarted(_))
        {
            return Err(RuntimeError::InvalidRunStream(
                "run stream events appeared before RunStarted".to_owned(),
            ));
        }
        match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => {
                if seen_run_started {
                    return Err(RuntimeError::InvalidRunStream(
                        "run stream contains multiple RunStarted events".to_owned(),
                    ));
                }
                seen_run_started = true;
                for cell_id in validate_seed_cells(runtime_spec, &payload.seed_cells)?.into_keys() {
                    available_cells.insert(cell_id);
                }
            }
            events::KernelEventPayload::RunCompleted(payload) => {
                if !active_attempts.is_empty() {
                    return Err(RuntimeError::InvalidRunStream(
                        "RunCompleted cannot finalize while state attempts are active".to_owned(),
                    ));
                }
                validate_historical_run_completed(
                    runtime_spec,
                    run_id,
                    event,
                    payload,
                    produced_public_output.as_ref(),
                    retention_manifest_projected_seq,
                )?;
                completed = true;
            }
            events::KernelEventPayload::RetentionRefsAppended(_) => {}
            events::KernelEventPayload::RetentionManifestProjected(payload) => {
                if &payload.run_id == run_id && payload.spec_hash == *runtime_spec.spec_hash() {
                    retention_manifest_projected_seq = Some(event.seq());
                }
            }
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt started for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.state_kind != node.state_kind
                    || payload.state_version != node.state_version
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} carries state identity outside the certified spec",
                        payload.attempt_id, payload.node_id
                    )));
                }
                validate_attempt_start_boundary(runtime_spec, node, payload, &available_cells)?;
                if !active_attempts.insert((payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} was started more than once",
                        payload.attempt_id, payload.node_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} completed uncertified output cell {}",
                        payload.attempt_id, payload.node_id, payload.output_cell_id
                    )));
                }
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt completion for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if runtime_spec
                    .node(&payload.node_id)
                    .expect("checked above")
                    .side_effect
                    .is_some()
                {
                    validate_historical_side_effect_failure(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt failure for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::CellProduced(payload) => {
                validate_historical_produced_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                validate_historical_skipped_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "fact recorded for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let caps = CertifiedRuntimeCapabilities::new(
                    node.node_id.clone(),
                    node.capability_bindings.clone(),
                );
                require_capability(
                    &caps,
                    &payload.capability_kind,
                    &payload.capability_version,
                    &node.node_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_adapter(node, &payload.adapter_kind, &payload.adapter_version)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_projected_attempt(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    "fact",
                )?;
                if !active_attempts.contains(&(payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} for node {} attempt {} was recorded outside an active started attempt",
                        payload.fact_key, payload.node_id, payload.attempt_id
                    )));
                }
                let fact = projections
                    .fact(&payload.node_id, &payload.attempt_id, &payload.fact_key)
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "fact {} for node {} attempt {} is not projected",
                            payload.fact_key, payload.node_id, payload.attempt_id
                        ))
                    })?;
                if fact.event_id != *event.event_id()
                    || fact.request_schema_id != payload.request_schema_id
                    || fact.request_hash != payload.request_hash
                    || fact.response_schema_id != payload.response_schema_id
                    || fact.response_hash != payload.response_hash
                    || fact.artifact_id != payload.artifact_id
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} projection does not match authoritative event",
                        payload.fact_key
                    )));
                }
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                if let Some(node_id) = &payload.node_id {
                    runtime_spec.node(node_id).ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "artifact referenced for uncertified node {node_id}"
                        ))
                    })?;
                }
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output(runtime_spec, node, payload)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                validate_historical_public_output_produced(
                    runtime_spec,
                    projections,
                    event,
                    node,
                    payload,
                )?;
                produced_public_output = Some(events::PublicOutputCompletionEvidence {
                    public_output_schema_id: payload.public_schema_id.clone(),
                    public_output_event_id: event.event_id().clone(),
                });
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output failure by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                validate_historical_public_output_failed(projections, payload)?;
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {
                validate_historical_side_effect_payload(
                    runtime_spec,
                    &active_attempts,
                    &mut side_effect_ledgers,
                    event.payload(),
                )?;
            }
        }
    }
    validate_atomic_terminal_pairs(runtime_spec, stream)?;
    validate_historical_bootstrap_run_batch(runtime_spec, run_id, stream)?;
    validate_historical_retention_ref_batches(runtime_spec, stream)?;
    validate_historical_retention_manifest_batches(runtime_spec, stream)?;
    validate_historical_complete_run_tail(runtime_spec, run_id, stream)?;
    validate_atomic_side_effect_failure_pairs(runtime_spec, stream)?;
    validate_recovery_frontier(runtime_spec, projections)?;
    Ok(())
}

fn validate_historical_side_effect_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    active_attempts: &BTreeSet<(NodeId, AttemptId)>,
    ledgers: &mut BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    let (node_id, attempt_id, ledger_key, phase) = side_effect_payload_ref(payload)
        .ok_or_else(|| RuntimeError::InvalidRunStream("expected side-effect payload".to_owned()))?;
    let node = runtime_spec.node(node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("side-effect event for uncertified node {node_id}"))
    })?;
    if node.side_effect.is_none() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "non-side-effect node {} emitted side-effect ledger event",
            node.node_id
        )));
    }
    if !active_attempts.contains(&(node_id.clone(), attempt_id.clone())) {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect ledger {} for node {} attempt {} was recorded outside an active started attempt",
            ledger_key, node_id, attempt_id
        )));
    }

    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if payload.scope_id != node.scope_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect intent for node {} carries uncertified scope {}",
                    node.node_id, payload.scope_id
                )));
            }
            let caps = CertifiedRuntimeCapabilities::new(
                node.node_id.clone(),
                node.capability_bindings.clone(),
            );
            require_capability(
                &caps,
                &payload.capability_kind,
                &payload.capability_version,
                &node.node_id,
            )
            .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
            require_adapter(node, &payload.adapter_kind, &payload.adapter_version)
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
            if ledgers
                .insert(
                    ledger_key.clone(),
                    HistoricalSideEffectLedger {
                        node_id: node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        phase,
                    },
                )
                .is_some()
            {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} persisted intent more than once",
                    ledger_key
                )));
            }
        }
        _ => {
            let ledger = ledgers.get_mut(ledger_key).ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} advanced before intent was persisted",
                    ledger_key
                ))
            })?;
            if ledger.node_id != *node_id || ledger.attempt_id != *attempt_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} changed node or attempt authority",
                    ledger_key
                )));
            }
            ledger.phase = phase;
        }
    }

    Ok(())
}

fn side_effect_payload_ref(
    payload: &events::KernelEventPayload,
) -> Option<(
    &NodeId,
    &AttemptId,
    &events::SideEffectLedgerKey,
    HistoricalSideEffectPhase,
)> {
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::IntentPersisted,
        )),
        events::KernelEventPayload::SideEffectClaimed(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Claimed,
        )),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Claimed,
        )),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::InvocationPrepared,
        )),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::InvocationStarted,
        )),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::NotSubmittedProven,
        )),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::SubmissionObserved,
        )),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::SubmissionUnknown,
        )),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::ReceiptObserved,
        )),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::ConfirmationObserved,
        )),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Ambiguous,
        )),
        events::KernelEventPayload::SideEffectFailed(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Failed,
        )),
        _ => None,
    }
}

fn validate_historical_side_effect_confirmation(
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = historical_side_effect_ledger_for_attempt(ledgers, node_id, attempt_id)?;
    if ledger.phase == HistoricalSideEffectPhase::ConfirmationObserved {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node_id, attempt_id
        )))
    }
}

fn validate_historical_side_effect_failure(
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = historical_side_effect_ledger_for_attempt(ledgers, node_id, attempt_id)?;
    if ledger.phase == HistoricalSideEffectPhase::Failed {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} failed without side-effect failure evidence",
            node_id, attempt_id
        )))
    }
}

fn historical_side_effect_ledger_for_attempt<'a>(
    ledgers: &'a BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<&'a HistoricalSideEffectLedger> {
    let mut found = None;
    for ledger in ledgers.values() {
        if ledger.node_id == *node_id
            && ledger.attempt_id == *attempt_id
            && found.replace(ledger).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledgers in history",
                node_id, attempt_id
            )));
        }
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} lacks ledger evidence",
            node_id, attempt_id
        ))
    })
}

fn validate_attempt_start_boundary(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    payload: &events::StateAttemptStarted,
    available_cells: &BTreeSet<CellId>,
) -> Result<()> {
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        if !available_cells.contains(&cell_id) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "attempt {} for node {} started before certified input cell {} was terminal",
                payload.attempt_id, node.node_id, cell_id
            )));
        }
    }
    Ok(())
}

fn validate_atomic_terminal_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut completions = BTreeSet::new();
    let mut terminal_cells = BTreeSet::new();
    let mut public_outputs = BTreeSet::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                completions.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.output_cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "cell produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "cell skipped by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if !matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                ) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "public output produced by non-render node {}",
                        payload.node_id
                    )));
                }
                public_outputs.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.receipt_cell_id.clone(),
                ));
            }
            _ => {}
        }
    }

    for (seq, node_id, attempt_id, output_cell_id) in &completions {
        if !terminal_cells.contains(&(
            *seq,
            node_id.clone(),
            attempt_id.clone(),
            output_cell_id.clone(),
        )) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "attempt {} for node {} completed without terminal cell {} in the same commit",
                attempt_id, node_id, output_cell_id
            )));
        }
    }
    for (seq, node_id, attempt_id, cell_id) in &terminal_cells {
        if !completions.contains(&(*seq, node_id.clone(), attempt_id.clone(), cell_id.clone())) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "terminal cell {} for node {} lacks StateAttemptCompleted in the same commit",
                cell_id, node_id
            )));
        }
    }
    for (seq, node_id, attempt_id, receipt_cell_id) in &public_outputs {
        let terminal = (
            *seq,
            node_id.clone(),
            attempt_id.clone(),
            receipt_cell_id.clone(),
        );
        if !terminal_cells.contains(&terminal) || !completions.contains(&terminal) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} attempt {} was split from receipt terminal cell {}",
                node_id, attempt_id, receipt_cell_id
            )));
        }
    }
    Ok(())
}

fn validate_historical_retention_manifest_batches(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
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
        validate_historical_retention_manifest_batch(runtime_spec, &stream[..index], commit)?;
        index = end;
    }
    Ok(())
}

type RetentionRefKey = (ArtifactId, ContentDigest, events::ArtifactRole);

fn validate_historical_retention_ref_batches(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
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
        validate_historical_retention_ref_batch(runtime_spec, &stream[index..end])?;
        index = end;
    }
    Ok(())
}

fn validate_historical_bootstrap_run_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let Some(first) = stream.first() else {
        return Err(RuntimeError::InvalidRunStream(
            "run stream is missing sealed BootstrapRun genesis commit".to_owned(),
        ));
    };
    if first.seq() != store::StreamSeq::FIRST || first.ordinal() != store::CommitOrdinal::new(0) {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted must be the first event in the sealed BootstrapRun genesis commit"
                .to_owned(),
        ));
    }
    let first_seq = first.seq();
    let first_commit_key = first.commit_key().clone();
    let mut end = 1;
    while end < stream.len()
        && stream[end].seq() == first_seq
        && stream[end].commit_key() == &first_commit_key
    {
        end += 1;
    }
    validate_bootstrap_run_commit_payload_set(runtime_spec, run_id, &stream[..end])
}

fn validate_bootstrap_run_commit_payload_set(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let run_started = match commit.first().map(store::KernelEventEnvelope::payload) {
        Some(events::KernelEventPayload::RunStarted(payload)) => payload,
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "sealed BootstrapRun genesis commit must start with RunStarted".to_owned(),
            ));
        }
    };
    validate_run_started_matches_certified_spec(runtime_spec, run_id, run_started)?;

    let config_count = runtime_spec.spec().config_refs.len();
    let expected_len = config_count + 6;
    if commit.len() != expected_len {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit has unexpected payload count".to_owned(),
        ));
    }

    let mut config_artifacts = Vec::with_capacity(config_count);
    for event in &commit[1..1 + config_count] {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            return Err(RuntimeError::InvalidRunStream(
                "sealed BootstrapRun genesis commit has non-config payload in config slot"
                    .to_owned(),
            ));
        };
        if payload.spec_hash != *runtime_spec.spec_hash()
            || payload.node_id.is_some()
            || payload.attempt_id.is_some()
            || payload.artifact_ref.role != events::ArtifactRole::TypedConfig
        {
            return Err(RuntimeError::InvalidRunStream(
                "sealed BootstrapRun genesis commit has invalid config artifact reference"
                    .to_owned(),
            ));
        }
        config_artifacts.push(store_artifact_from_event_ref(
            &payload.artifact_ref,
            None,
            None,
        ));
    }
    let config_artifacts = validate_config_artifacts(runtime_spec, config_artifacts)?;
    let expected_config_payloads =
        config_artifact_reference_payloads(runtime_spec.spec_hash(), &config_artifacts)?;
    for (event, expected) in commit[1..1 + config_count]
        .iter()
        .zip(expected_config_payloads)
    {
        if event.payload() != &expected {
            return Err(RuntimeError::InvalidRunStream(
                "sealed BootstrapRun genesis commit config references do not match certified config refs"
                    .to_owned(),
            ));
        }
    }

    let bootstrap_node = certified_bootstrap_run_node(runtime_spec)?;
    let bootstrap_output_cell =
        runtime_spec
            .cell(&bootstrap_node.output_cell)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "bootstrap lifecycle node {} output cell {} is missing",
                    bootstrap_node.node_id, bootstrap_node.output_cell
                ))
            })?;
    let bootstrap_attempt_id =
        attempt_id(run_id, runtime_spec.spec_hash(), &bootstrap_node.node_id, 1)?;
    let genesis = GenesisContext {
        runtime_spec,
        run_id,
        node: bootstrap_node,
        output_cell: bootstrap_output_cell,
        attempt_id: &bootstrap_attempt_id,
        run_started,
        config_artifacts: &config_artifacts,
    };
    let (bootstrap_receipt_bytes, bootstrap_receipt_artifact) =
        bootstrap_run_receipt_artifact(&genesis)?;
    let expected_receipt_ref = event_artifact_ref_from_store(&bootstrap_receipt_artifact)?;
    let expected_refs = run_started_retention_refs(
        runtime_spec,
        run_started,
        &config_artifacts,
        &bootstrap_receipt_artifact,
    )?;

    let mut pos = 1 + config_count;
    let expected_start =
        events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: runtime_spec.spec_hash().clone(),
            node_id: bootstrap_node.node_id.clone(),
            attempt_id: bootstrap_attempt_id.clone(),
            attempt_no: 1,
            state_kind: bootstrap_node.state_kind.clone(),
            state_version: bootstrap_node.state_version.clone(),
        });
    if commit[pos].payload() != &expected_start {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching StateAttemptStarted".to_owned(),
        ));
    }
    pos += 1;

    let expected_cell = events::KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: runtime_spec.spec_hash().clone(),
        node_id: bootstrap_node.node_id.clone(),
        cell_id: bootstrap_node.output_cell.clone(),
        scope_id: bootstrap_output_cell.scope_id.clone(),
        attempt_id: bootstrap_attempt_id.clone(),
        semantic_type_id: bootstrap_output_cell.semantic_type_id.clone(),
        schema_id: bootstrap_output_cell.schema_id.clone(),
        value_lineage: bootstrap_output_cell.value_lineage.clone(),
        artifact_id: bootstrap_receipt_artifact.artifact_id.clone(),
        content_digest: bootstrap_receipt_artifact.digest.clone(),
        producer_state_kind: Some(bootstrap_node.state_kind.clone()),
        producer_state_version: Some(bootstrap_node.state_version.clone()),
    });
    if commit[pos].payload() != &expected_cell {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching receipt cell".to_owned(),
        ));
    }
    pos += 1;

    let expected_completed =
        events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
            spec_hash: runtime_spec.spec_hash().clone(),
            node_id: bootstrap_node.node_id.clone(),
            attempt_id: bootstrap_attempt_id.clone(),
            output_cell_id: bootstrap_node.output_cell.clone(),
        });
    if commit[pos].payload() != &expected_completed {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching StateAttemptCompleted".to_owned(),
        ));
    }
    pos += 1;

    let expected_ref = events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
        spec_hash: runtime_spec.spec_hash().clone(),
        node_id: Some(bootstrap_node.node_id.clone()),
        attempt_id: Some(bootstrap_attempt_id.clone()),
        artifact_ref: expected_receipt_ref,
    });
    if commit[pos].payload() != &expected_ref {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching bootstrap receipt artifact reference"
                .to_owned(),
        ));
    }
    pos += 1;

    match commit[pos].payload() {
        events::KernelEventPayload::RetentionRefsAppended(payload)
            if payload.run_id == *run_id
                && payload.spec_hash == *runtime_spec.spec_hash()
                && payload.reason == events::RetentionReason::RunStarted
                && payload.refs == expected_refs =>
        {
            verify_artifact_bytes(
                bootstrap_receipt_bytes.as_bytes(),
                &bootstrap_receipt_artifact,
            )?;
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching run-start retention refs".to_owned(),
        )),
    }
}

fn validate_run_started_matches_certified_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    run_started: &events::RunStarted,
) -> Result<()> {
    if run_started.run_id != *run_id
        || run_started.spec_hash != *runtime_spec.spec_hash()
        || run_started.spec_artifact_id
            != ArtifactId::from_digest(
                runtime_spec.spec_hash().algorithm(),
                *runtime_spec.spec_hash().digest(),
            )
        || run_started.spec_media_type != runtime_spec.spec().media_type
        || run_started.spec_version != runtime_spec.spec().spec_version
        || run_started.lowering_version != runtime_spec.spec().lowering_version
        || run_started.public_output_schema_id
            != runtime_spec.spec().public_outputs.public_schema_id
        || run_started.canonicalizer_identity
            != runtime_spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
        || run_started.descriptor_identities != runtime_spec.spec().descriptor_identities
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted payload does not match the certified runtime spec".to_owned(),
        ));
    }
    let certificate_canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let certificate_digest = certificate_canonical.content_digest();
    if run_started.certificate_artifact_id
        != ArtifactId::from_digest(certificate_digest.algorithm(), *certificate_digest.digest())
        || run_started.certificate_artifact_digest != certificate_digest
        || run_started.certificate_media_type
            != spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)?
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted certificate evidence does not match the certified runtime spec".to_owned(),
        ));
    }
    validate_seed_cells(runtime_spec, &run_started.seed_cells)?;
    Ok(())
}

fn run_started_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    run_started: &events::RunStarted,
    config_artifacts: &[store::ArtifactEvidenceRef],
    bootstrap_receipt_artifact: &store::ArtifactEvidenceRef,
) -> Result<Vec<events::RetentionRef>> {
    let mut refs = Vec::with_capacity(3 + config_artifacts.len() + run_started.seed_cells.len());
    refs.push(events::RetentionRef {
        artifact_id: run_started.spec_artifact_id.clone(),
        content_digest: ContentDigest::from_digest(
            run_started.spec_hash.algorithm(),
            *run_started.spec_hash.digest(),
        ),
        role: events::ArtifactRole::TypedExecutionSpec,
    });
    refs.push(events::RetentionRef {
        artifact_id: run_started.certificate_artifact_id.clone(),
        content_digest: run_started.certificate_artifact_digest.clone(),
        role: events::ArtifactRole::TypedSpecCertificate,
    });
    refs.extend(config_artifacts.iter().map(retention_ref_for_artifact));
    let seed_cells = validate_seed_cells(runtime_spec, &run_started.seed_cells)?;
    refs.extend(seed_cells.values().map(|seed| events::RetentionRef {
        artifact_id: seed.seed_artifact.artifact_id.clone(),
        content_digest: seed.seed_artifact.content_digest.clone(),
        role: seed.seed_artifact.role,
    }));
    refs.push(retention_ref_for_artifact(bootstrap_receipt_artifact));
    Ok(refs)
}

fn validate_historical_retention_ref_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let retention_refs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload) => Some((event, payload)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let run_started = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if run_started.len() > 1 {
        return Err(RuntimeError::InvalidRunStream(
            "commit contains multiple RunStarted payloads".to_owned(),
        ));
    }
    if run_started.len() == 1 {
        let run_started_refs = retention_refs
            .iter()
            .filter(|(_, payload)| payload.reason == events::RetentionReason::RunStarted)
            .count();
        if retention_refs.len() != 1 || run_started_refs != 1 {
            return Err(RuntimeError::InvalidRunStream(
                "RunStarted commit must include exactly one run-start retention refs append"
                    .to_owned(),
            ));
        }
    }
    if retention_refs.is_empty() {
        return Ok(());
    }

    let has_manifest_projection = commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::RetentionManifestProjected(_)
        )
    });
    let artifact_refs = same_commit_artifact_reference_keys(commit);
    let typed_payload_refs = same_commit_typed_artifact_keys(commit);

    for (event, payload) in retention_refs {
        if event.run_id() != &payload.run_id {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention refs for run {} were appended to stream {}",
                payload.run_id,
                event.run_id()
            )));
        }
        if payload.spec_hash != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunStream(
                "retention refs spec hash does not match certified runtime spec".to_owned(),
            ));
        }
        if payload.refs.is_empty() {
            return Err(RuntimeError::InvalidRunStream(
                "retention refs append cannot be empty".to_owned(),
            ));
        }
        match payload.reason {
            events::RetentionReason::RunStarted => {
                if run_started.len() != 1 {
                    return Err(RuntimeError::InvalidRunStream(
                        "run-start retention refs must be appended in the RunStarted commit"
                            .to_owned(),
                    ));
                }
                let expected =
                    run_started_retention_ref_keys(runtime_spec, run_started[0], commit)?;
                let actual = payload
                    .refs
                    .iter()
                    .map(retention_ref_key)
                    .collect::<BTreeSet<_>>();
                if actual != expected {
                    return Err(RuntimeError::InvalidRunStream(
                        "run-start retention refs do not match launch artifact evidence".to_owned(),
                    ));
                }
            }
            events::RetentionReason::ManifestProjection => {
                if !has_manifest_projection {
                    return Err(RuntimeError::InvalidRunStream(
                        "manifest-projection retention refs must be appended with the manifest projection"
                            .to_owned(),
                    ));
                }
            }
            events::RetentionReason::RuntimeEvidence => {
                validate_same_commit_retention_ref_evidence(
                    payload,
                    &artifact_refs,
                    &typed_payload_refs,
                )?;
            }
            events::RetentionReason::PublicOutput => {
                validate_same_commit_retention_ref_evidence(
                    payload,
                    &artifact_refs,
                    &typed_payload_refs,
                )?;
                validate_public_output_retention_refs(runtime_spec, commit, payload)?;
            }
        }
    }
    Ok(())
}

fn validate_same_commit_retention_ref_evidence(
    payload: &events::RetentionRefsAppended,
    artifact_refs: &BTreeSet<RetentionRefKey>,
    typed_payload_refs: &BTreeSet<RetentionRefKey>,
) -> Result<()> {
    for retention_ref in &payload.refs {
        let key = retention_ref_key(retention_ref);
        if retention_ref_requires_artifact_reference(retention_ref.role)
            && !artifact_refs.contains(&key)
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention ref for artifact {} lacks same-commit artifact reference evidence",
                retention_ref.artifact_id
            )));
        }
        if !typed_payload_refs.contains(&key) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention ref for artifact {} lacks same-commit typed payload evidence",
                retention_ref.artifact_id
            )));
        }
    }
    Ok(())
}

fn validate_public_output_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    commit: &[store::KernelEventEnvelope],
    payload: &events::RetentionRefsAppended,
) -> Result<()> {
    let public_outputs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::PublicOutputProduced(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if public_outputs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "public-output retention refs must be appended with exactly one framework public-output payload"
                .to_owned(),
        ));
    }
    let public_output = public_outputs[0];
    let node = runtime_spec.node(&public_output.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "public-output retention refs reference uncertified node {}",
            public_output.node_id
        ))
    })?;
    if !matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    ) {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public-output retention refs were appended by non-render node {}",
            public_output.node_id
        )));
    }
    let allowed = public_output_retention_ref_keys(commit, public_output)?;
    for retention_ref in &payload.refs {
        if !allowed.contains(&retention_ref_key(retention_ref)) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public-output retention ref for artifact {} is not sealed to framework public-output evidence",
                retention_ref.artifact_id
            )));
        }
    }
    Ok(())
}

fn public_output_retention_ref_keys(
    commit: &[store::KernelEventEnvelope],
    public_output: &events::PublicOutputProduced,
) -> Result<BTreeSet<RetentionRefKey>> {
    let mut allowed = BTreeSet::new();
    for event in commit {
        if let events::KernelEventPayload::CellProduced(payload) = event.payload() {
            if payload.node_id == public_output.node_id
                && payload.attempt_id == public_output.attempt_id
                && payload.cell_id == public_output.receipt_cell_id
            {
                allowed.insert((
                    payload.artifact_id.clone(),
                    payload.content_digest.clone(),
                    events::ArtifactRole::StateOutput,
                ));
            }
        }
    }
    if let Some(artifact_id) = &public_output.rendered_artifact_id {
        allowed.insert((
            artifact_id.clone(),
            public_output.rendered_digest.clone(),
            events::ArtifactRole::PublicOutput,
        ));
    }
    if allowed.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "public-output retention refs lack same-commit framework receipt/render evidence"
                .to_owned(),
        ));
    }
    Ok(allowed)
}

fn retention_ref_requires_artifact_reference(role: events::ArtifactRole) -> bool {
    staged_artifact_binding_kind(role).is_some()
}

fn retention_ref_key(retention_ref: &events::RetentionRef) -> RetentionRefKey {
    (
        retention_ref.artifact_id.clone(),
        retention_ref.content_digest.clone(),
        retention_ref.role,
    )
}

fn event_artifact_ref_key(artifact: &events::ArtifactEvidenceRef) -> RetentionRefKey {
    (
        artifact.artifact_id.clone(),
        artifact.content_digest.clone(),
        artifact.role,
    )
}

fn same_commit_artifact_reference_keys(
    commit: &[store::KernelEventEnvelope],
) -> BTreeSet<RetentionRefKey> {
    commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                Some(event_artifact_ref_key(&payload.artifact_ref))
            }
            _ => None,
        })
        .collect()
}

fn same_commit_typed_artifact_keys(
    commit: &[store::KernelEventEnvelope],
) -> BTreeSet<RetentionRefKey> {
    let mut keys = BTreeSet::new();
    for event in commit {
        match event.payload() {
            events::KernelEventPayload::CellProduced(payload) => {
                keys.insert((
                    payload.artifact_id.clone(),
                    payload.content_digest.clone(),
                    events::ArtifactRole::StateOutput,
                ));
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                keys.insert((
                    payload.artifact_id.clone(),
                    payload.response_hash.clone(),
                    events::ArtifactRole::FactResponse,
                ));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                if let Some(artifact_id) = &payload.rendered_artifact_id {
                    keys.insert((
                        artifact_id.clone(),
                        payload.rendered_digest.clone(),
                        events::ArtifactRole::PublicOutput,
                    ));
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                keys.insert((
                    payload.intent_artifact_id.clone(),
                    payload.intent_hash.clone(),
                    events::ArtifactRole::SideEffectIntent,
                ));
            }
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                if let (Some(artifact_id), Some(content_digest)) =
                    (&payload.prepared_artifact_id, &payload.prepared_hash)
                {
                    keys.insert((
                        artifact_id.clone(),
                        content_digest.clone(),
                        events::ArtifactRole::PreparedInvocation,
                    ));
                }
            }
            events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                keys.insert((
                    payload.proof_artifact_id.clone(),
                    payload.proof_hash.clone(),
                    events::ArtifactRole::NotSubmittedProof,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                keys.insert((
                    payload.submission_artifact_id.clone(),
                    payload.submission_hash.clone(),
                    events::ArtifactRole::Submission,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                keys.insert((
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    events::ArtifactRole::SubmissionUnknownEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                keys.insert((
                    payload.receipt_artifact_id.clone(),
                    payload.receipt_hash.clone(),
                    events::ArtifactRole::Receipt,
                ));
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                keys.insert((
                    payload.confirmation_artifact_id.clone(),
                    payload.confirmation_hash.clone(),
                    events::ArtifactRole::Confirmation,
                ));
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                keys.insert((
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    events::ArtifactRole::AmbiguityEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            _ => {}
        }
    }
    keys
}

fn insert_error_diagnostic_key(error: &events::MfmErrorInfo, keys: &mut BTreeSet<RetentionRefKey>) {
    if let Some(diagnostic) = &error.diagnostic_ref {
        keys.insert(event_artifact_ref_key(diagnostic));
    }
}

fn run_started_retention_ref_keys(
    runtime_spec: &CertifiedRuntimeSpec,
    run_started: &events::RunStarted,
    commit: &[store::KernelEventEnvelope],
) -> Result<BTreeSet<RetentionRefKey>> {
    let mut keys = BTreeSet::new();
    keys.insert((
        run_started.spec_artifact_id.clone(),
        ContentDigest::from_digest(
            run_started.spec_hash.algorithm(),
            *run_started.spec_hash.digest(),
        ),
        events::ArtifactRole::TypedExecutionSpec,
    ));
    keys.insert((
        run_started.certificate_artifact_id.clone(),
        run_started.certificate_artifact_digest.clone(),
        events::ArtifactRole::TypedSpecCertificate,
    ));
    for seed in &run_started.seed_cells {
        keys.insert(event_artifact_ref_key(&seed.seed_artifact));
    }
    for event in commit {
        if let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() {
            if payload.artifact_ref.role == events::ArtifactRole::TypedConfig {
                keys.insert(event_artifact_ref_key(&payload.artifact_ref));
            }
        }
    }
    let bootstrap_node = certified_bootstrap_run_node(runtime_spec)?;
    for event in commit {
        if let events::KernelEventPayload::CellProduced(payload) = event.payload() {
            if payload.node_id == bootstrap_node.node_id
                && payload.cell_id == bootstrap_node.output_cell
            {
                keys.insert((
                    payload.artifact_id.clone(),
                    payload.content_digest.clone(),
                    events::ArtifactRole::StateOutput,
                ));
            }
        }
    }
    Ok(keys)
}

fn validate_historical_retention_manifest_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    pre_projection_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let projections = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionManifestProjected(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if projections.is_empty() {
        return Ok(());
    }
    if projections.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit contains multiple manifest projections"
                .to_owned(),
        ));
    }
    let projection = projections[0];
    let retention_node = certified_retention_manifest_node(runtime_spec)?;
    if projection.spec_hash != *runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection spec hash does not match certified runtime spec"
                .to_owned(),
        ));
    }
    let expected = build_retention_manifest_artifact_with_producer(
        runtime_spec,
        &projection.run_id,
        pre_projection_stream,
        Some(retention_node.node_id.clone()),
    )?;
    if projection.manifest_seq != expected.manifest_seq
        || projection.manifest_digest != expected.evidence.digest
        || projection.previous_manifest_digest != expected.previous_manifest_digest
        || projection.manifest_artifact_id != expected.evidence.artifact_id
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection does not match the authoritative pre-projection stream"
                .to_owned(),
        ));
    }

    let receipt_bytes = retention_manifest_receipt_json(&expected, pre_projection_stream)?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let produced = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == retention_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if produced.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection was not produced by exactly one framework retention receipt cell"
                .to_owned(),
        ));
    }
    let produced = produced[0];
    if produced.spec_hash != *runtime_spec.spec_hash()
        || produced.cell_id != retention_node.output_cell
        || produced.artifact_id != receipt_artifact_id
        || produced.content_digest != receipt_digest
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt cell does not match the projected manifest".to_owned(),
        ));
    }

    let started_count = commit
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if payload.node_id == retention_node.node_id
                        && payload.attempt_id == produced.attempt_id
                        && payload.spec_hash == *runtime_spec.spec_hash()
                        && payload.state_kind == retention_node.state_kind
                        && payload.state_version == retention_node.state_version
            )
        })
        .count();
    if started_count != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection lacks matching framework StateAttemptStarted in the same commit"
                .to_owned(),
        ));
    }

    let completed_count = commit
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptCompleted(payload)
                    if payload.node_id == retention_node.node_id
                        && payload.attempt_id == produced.attempt_id
                        && payload.output_cell_id == retention_node.output_cell
            )
        })
        .count();
    if completed_count != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection lacks matching framework StateAttemptCompleted in the same commit"
                .to_owned(),
        ));
    }

    let refs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    let manifest_refs = refs
        .iter()
        .copied()
        .filter(|payload| payload.reason == events::RetentionReason::ManifestProjection)
        .collect::<Vec<_>>();
    if manifest_refs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit must contain exactly one manifest retention refs append"
                .to_owned(),
        ));
    }
    let manifest_refs = manifest_refs[0];
    if manifest_refs.run_id != projection.run_id
        || manifest_refs.spec_hash != *runtime_spec.spec_hash()
        || manifest_refs.refs.len() != 1
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection retention refs are not sealed to manifest projection"
                .to_owned(),
        ));
    }
    let manifest_ref = &manifest_refs.refs[0];
    if manifest_ref.artifact_id != expected.evidence.artifact_id
        || manifest_ref.content_digest != expected.evidence.digest
        || manifest_ref.role != events::ArtifactRole::RetentionManifest
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection retention ref does not match the manifest artifact"
                .to_owned(),
        ));
    }

    let receipt_refs = refs
        .iter()
        .copied()
        .filter(|payload| payload.reason == events::RetentionReason::RuntimeEvidence)
        .collect::<Vec<_>>();
    if receipt_refs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit must retain its framework receipt artifact"
                .to_owned(),
        ));
    }
    let receipt_refs = receipt_refs[0];
    if receipt_refs.run_id != projection.run_id
        || receipt_refs.spec_hash != *runtime_spec.spec_hash()
        || receipt_refs.refs.len() != 1
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt retention refs are not sealed to runtime evidence"
                .to_owned(),
        ));
    }
    let receipt_ref = &receipt_refs.refs[0];
    if receipt_ref.artifact_id != receipt_artifact_id
        || receipt_ref.content_digest != receipt_digest
        || receipt_ref.role != events::ArtifactRole::StateOutput
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt retention ref does not match the receipt artifact"
                .to_owned(),
        ));
    }
    if refs.len() != 2 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit contains unsupported retention refs".to_owned(),
        ));
    }
    validate_retention_projection_commit_payload_set(
        commit,
        &retention_node.node_id,
        &produced.attempt_id,
        &retention_node.output_cell,
        &receipt_artifact_id,
        &receipt_digest,
        &expected.evidence.artifact_id,
    )?;
    Ok(())
}

fn validate_retention_projection_commit_payload_set(
    commit: &[store::KernelEventEnvelope],
    retention_node_id: &NodeId,
    attempt_id: &AttemptId,
    receipt_cell_id: &CellId,
    receipt_artifact_id: &ArtifactId,
    receipt_digest: &ContentDigest,
    manifest_artifact_id: &ArtifactId,
) -> Result<()> {
    let mut started = 0_usize;
    let mut produced = 0_usize;
    let mut completed = 0_usize;
    let mut artifact_referenced = 0_usize;
    let mut manifest_projected = 0_usize;
    let mut retention_refs = 0_usize;

    for event in commit {
        match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == *retention_node_id && payload.attempt_id == *attempt_id =>
            {
                started += 1;
            }
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == *retention_node_id
                    && payload.attempt_id == *attempt_id
                    && payload.cell_id == *receipt_cell_id
                    && payload.artifact_id == *receipt_artifact_id
                    && payload.content_digest == *receipt_digest =>
            {
                produced += 1;
            }
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.node_id == *retention_node_id
                    && payload.attempt_id == *attempt_id
                    && payload.output_cell_id == *receipt_cell_id =>
            {
                completed += 1;
            }
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(retention_node_id)
                    && payload.attempt_id.as_ref() == Some(attempt_id)
                    && payload.artifact_ref.artifact_id == *receipt_artifact_id
                    && payload.artifact_ref.content_digest == *receipt_digest
                    && payload.artifact_ref.role == events::ArtifactRole::StateOutput =>
            {
                artifact_referenced += 1;
            }
            events::KernelEventPayload::RetentionManifestProjected(payload)
                if payload.manifest_artifact_id == *manifest_artifact_id =>
            {
                manifest_projected += 1;
            }
            events::KernelEventPayload::RetentionRefsAppended(payload) => match payload.reason {
                events::RetentionReason::ManifestProjection
                    if payload
                        .refs
                        .iter()
                        .all(|reference| reference.artifact_id == *manifest_artifact_id) =>
                {
                    retention_refs += 1;
                }
                events::RetentionReason::RuntimeEvidence
                    if payload
                        .refs
                        .iter()
                        .all(|reference| reference.artifact_id == *receipt_artifact_id) =>
                {
                    retention_refs += 1;
                }
                _ => {
                    return Err(RuntimeError::InvalidRunStream(
                        "retention manifest projection commit contains unsupported payload"
                            .to_owned(),
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::InvalidRunStream(
                    "retention manifest projection commit contains unsupported payload".to_owned(),
                ));
            }
        }
    }

    if started == 1
        && produced == 1
        && completed == 1
        && artifact_referenced == 1
        && manifest_projected == 1
        && retention_refs == 2
        && commit.len() == 7
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit does not match the sealed framework batch"
                .to_owned(),
        ))
    }
}

fn validate_historical_complete_run_tail(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let completion_node = certified_complete_run_node(runtime_spec)?;
    let mut last_retention_commit_end = None::<usize>;
    let mut completion_commit = None::<(usize, usize)>;
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
        let has_retention_projection = commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        });
        if has_retention_projection {
            last_retention_commit_end = Some(end);
        }
        let has_completion_node_payload = commit
            .iter()
            .any(|event| payload_targets_node(event.payload(), &completion_node.node_id));
        let completion_count = commit
            .iter()
            .filter(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
            .count();
        if has_completion_node_payload || completion_count > 0 {
            if completion_commit.replace((index, end)).is_some() {
                return Err(RuntimeError::InvalidRunStream(
                    "run stream contains multiple CompleteRun commits".to_owned(),
                ));
            }
            if completion_count == 0 {
                return Err(RuntimeError::InvalidRunStream(
                    "CompleteRun commit is missing RunCompleted payload".to_owned(),
                ));
            }
            if completion_count != 1 {
                return Err(RuntimeError::InvalidRunStream(
                    "CompleteRun commit contains multiple RunCompleted payloads".to_owned(),
                ));
            }
        }
        index = end;
    }

    match (last_retention_commit_end, completion_commit) {
        (Some(retention_end), Some((completion_start, completion_end)))
            if retention_end == completion_start =>
        {
            validate_historical_complete_run_batch(
                runtime_spec,
                run_id,
                &stream[..completion_start],
                &stream[completion_start..completion_end],
            )
        }
        (Some(_), Some(_)) => Err(RuntimeError::InvalidRunStream(
            "run stream contains events after retention manifest projection outside sealed CompleteRun commit"
                .to_owned(),
        )),
        (Some(retention_end), None) if retention_end == stream.len() => Ok(()),
        (Some(_), None) => Err(RuntimeError::InvalidRunStream(
            "run stream contains events after retention manifest projection outside sealed CompleteRun commit"
                .to_owned(),
        )),
        (None, Some(_)) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before retention manifest projection".to_owned(),
        )),
        (None, None) => Ok(()),
    }
}

fn payload_targets_node(payload: &events::KernelEventPayload, node_id: &NodeId) -> bool {
    match payload {
        events::KernelEventPayload::StateAttemptStarted(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::StateAttemptCompleted(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::StateAttemptFailed(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::CellProduced(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::CellSkipped(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::FactRecorded(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            payload.node_id.as_ref() == Some(node_id)
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            payload.node_id == *node_id
        }
        payload => side_effect_payload_ref(payload)
            .map(|(payload_node_id, _, _, _)| payload_node_id == node_id)
            .unwrap_or(false),
    }
}

fn validate_historical_complete_run_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    pre_completion_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let completion_payload = commit
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .expect("caller checked completion count");
    let completion_node = certified_complete_run_node(runtime_spec)?;
    let pre_completion_projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(pre_completion_stream)?;
    let completion = run_completion_evidence(runtime_spec, &pre_completion_projection)?;
    let retention_manifest = projected_retention_manifest(run_id, &pre_completion_projection)?;
    if completion_payload.run_id != *run_id
        || completion_payload.spec_hash != *runtime_spec.spec_hash()
        || completion_payload.outcome != events::RunCompletionOutcome::Completed(completion.clone())
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunCompleted payload does not match sealed CompleteRun evidence".to_owned(),
        ));
    }

    let receipt_bytes =
        complete_run_receipt_json(&completion, retention_manifest, pre_completion_stream)?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let produced = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == completion_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if produced.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun commit was not produced by exactly one framework completion receipt cell"
                .to_owned(),
        ));
    }
    let produced = produced[0];
    if produced.spec_hash != *runtime_spec.spec_hash()
        || produced.cell_id != completion_node.output_cell
        || produced.artifact_id != receipt_artifact_id
        || produced.content_digest != receipt_digest
    {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun receipt cell does not match the sealed completion evidence".to_owned(),
        ));
    }
    let completion_cell = runtime_spec
        .cell(&completion_node.output_cell)
        .ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "completion lifecycle node {} output cell {} is missing",
                completion_node.node_id, completion_node.output_cell
            ))
        })?;
    let expected_receipt_ref = events::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        role: events::ArtifactRole::StateOutput,
        schema_id: completion_cell.schema_id.clone(),
        semantic_type_id: Some(completion_cell.semantic_type_id.clone()),
        content_digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
    };

    validate_complete_run_commit_payload_set(
        commit,
        CompleteRunCommitValidation {
            run_id,
            spec_hash: runtime_spec.spec_hash(),
            completion: &completion,
            completion_node_id: &completion_node.node_id,
            attempt_id: &produced.attempt_id,
            receipt_cell_id: &completion_node.output_cell,
            receipt_artifact_id: &receipt_artifact_id,
            receipt_digest: &receipt_digest,
            expected_receipt_ref: &expected_receipt_ref,
        },
    )
}

fn validate_complete_run_commit_payload_set(
    commit: &[store::KernelEventEnvelope],
    expected: CompleteRunCommitValidation<'_>,
) -> Result<()> {
    let CompleteRunCommitValidation {
        run_id,
        spec_hash,
        completion,
        completion_node_id,
        attempt_id,
        receipt_cell_id,
        receipt_artifact_id,
        receipt_digest,
        expected_receipt_ref,
    } = expected;
    if commit.len() != 5 {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun commit does not match the sealed framework batch".to_owned(),
        ));
    }
    match commit[0].payload() {
        events::KernelEventPayload::StateAttemptStarted(payload)
            if payload.node_id == *completion_node_id && payload.attempt_id == *attempt_id => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "CompleteRun commit does not match the sealed framework batch".to_owned(),
            ));
        }
    }
    match commit[1].payload() {
        events::KernelEventPayload::CellProduced(payload)
            if payload.node_id == *completion_node_id
                && payload.attempt_id == *attempt_id
                && payload.cell_id == *receipt_cell_id
                && payload.artifact_id == *receipt_artifact_id
                && payload.content_digest == *receipt_digest => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "CompleteRun commit does not match the sealed framework batch".to_owned(),
            ));
        }
    }
    match commit[2].payload() {
        events::KernelEventPayload::StateAttemptCompleted(payload)
            if payload.node_id == *completion_node_id
                && payload.attempt_id == *attempt_id
                && payload.output_cell_id == *receipt_cell_id => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "CompleteRun commit does not match the sealed framework batch".to_owned(),
            ));
        }
    }
    match commit[3].payload() {
        events::KernelEventPayload::ArtifactReferenced(payload)
            if payload.node_id.as_ref() == Some(completion_node_id)
                && payload.attempt_id.as_ref() == Some(attempt_id)
                && payload.artifact_ref == *expected_receipt_ref => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "CompleteRun commit does not match the sealed framework batch".to_owned(),
            ));
        }
    }
    match commit[4].payload() {
        events::KernelEventPayload::RunCompleted(payload)
            if payload.run_id == *run_id
                && payload.spec_hash == *spec_hash
                && payload.outcome
                    == events::RunCompletionOutcome::Completed(completion.clone()) =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(
            "CompleteRun commit does not match the sealed framework batch".to_owned(),
        )),
    }
}

fn certified_bootstrap_run_node(runtime_spec: &CertifiedRuntimeSpec) -> Result<&spec::NodeSpec> {
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

fn certified_retention_manifest_node(
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

fn certified_complete_run_node(runtime_spec: &CertifiedRuntimeSpec) -> Result<&spec::NodeSpec> {
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

fn validate_atomic_side_effect_failure_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut side_effect_failures = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::SideEffectFailed(payload) => {
                side_effect_failures.insert(
                    (
                        event.seq(),
                        payload.node_id.clone(),
                        payload.attempt_id.clone(),
                    ),
                    payload.retryable,
                );
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if node.side_effect.is_some() {
                    attempt_failures.insert(
                        (
                            event.seq(),
                            payload.node_id.clone(),
                            payload.attempt_id.clone(),
                        ),
                        payload.retryable,
                    );
                }
            }
            _ => {}
        }
    }
    for (failure, side_effect_retryable) in &side_effect_failures {
        match attempt_failures.get(failure) {
            Some(attempt_retryable) if attempt_retryable == side_effect_retryable => {}
            Some(_) => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect failure for node {} attempt {} disagrees with StateAttemptFailed retryability",
                    failure.1, failure.2
                )));
            }
            None => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect failure for node {} attempt {} lacks StateAttemptFailed in the same commit",
                    failure.1, failure.2
                )));
            }
        }
    }
    for failure in attempt_failures.keys() {
        if !side_effect_failures.contains_key(failure) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect attempt failure for node {} attempt {} lacks SideEffectFailed in the same commit",
                failure.1, failure.2
            )));
        }
    }
    Ok(())
}

fn validate_recovery_frontier(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if let Some(terminal) = projections.cell_terminal(&node.output_cell) {
            let attempt_id = validate_terminal_cell_has_completed_attempt(
                runtime_spec,
                projections,
                node,
                terminal,
            )?;
            if node.side_effect.is_some() {
                validate_side_effect_terminal_evidence(projections, node, &attempt_id)?;
            }
        }
        let mut started = None;
        for ((attempt_node_id, attempt_id), projection) in projections.attempts() {
            if attempt_node_id != &node.node_id {
                continue;
            }
            match &projection.status {
                store::AttemptStatus::Started { .. } => {
                    if projections.cell_terminal(&node.output_cell).is_some() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "node {} has a started attempt after its output cell became terminal",
                            node.node_id
                        )));
                    }
                    if started.replace(attempt_id.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "node {} has multiple started attempts during recovery",
                            node.node_id
                        )));
                    }
                }
                store::AttemptStatus::Completed { output_cell_id }
                    if projections.cell_terminal(output_cell_id).is_none() =>
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} attempt {} completed without terminal cell projection",
                        node.node_id, attempt_id
                    )));
                }
                store::AttemptStatus::Completed { .. } | store::AttemptStatus::Failed { .. } => {}
            }
        }
    }
    Ok(())
}

fn require_projected_attempt(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    event_kind: &'static str,
) -> Result<store::AttemptStatus> {
    let attempt = projections.attempt(node_id, attempt_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "{event_kind} event references missing attempt {attempt_id} for node {node_id}"
        ))
    })?;
    Ok(attempt.status.clone())
}

fn validate_historical_produced_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::CellProduced,
) -> Result<()> {
    let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("produced uncertified cell {}", payload.cell_id))
    })?;
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "cell {} was produced by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal(&payload.cell_id) {
        Some(store::CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && artifact_id == &payload.artifact_id
            && content_digest == &payload.content_digest =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} projection does not match authoritative event",
            payload.cell_id
        ))),
    }
}

fn validate_historical_skipped_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::CellSkipped,
) -> Result<()> {
    let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("skipped uncertified cell {}", payload.cell_id))
    })?;
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "cell {} was skipped by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
        || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal(&payload.cell_id) {
        Some(store::CellTerminalProjection::Skipped {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            skip_reason,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && skip_reason == &payload.skip_reason =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} projection does not match authoritative event",
            payload.cell_id
        ))),
    }
}

fn validate_historical_public_output_produced(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    match require_projected_attempt(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        "public output",
    )? {
        store::AttemptStatus::Completed { output_cell_id }
            if output_cell_id == node.output_cell => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} attempt {} is not backed by a completed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    if projections.cell_terminal(&node.output_cell).is_none() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} has no terminal render receipt cell",
            payload.node_id
        )));
    }
    for cell in &payload.cells {
        let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "public output references uncertified cell {}",
                cell.cell_id
            ))
        })?;
        match projections.cell_terminal(&cell.cell_id) {
            Some(store::CellTerminalProjection::Produced {
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                ..
            }) if schema_id == &cell.schema_id
                && semantic_type_id == &cell.semantic_type_id
                && artifact_id == &cell.artifact_id
                && content_digest == &cell.content_digest
                && certified.producer == cell.producer
                && certified.scope_id == cell.scope_id
                && certified.value_lineage == cell.value_lineage => {}
            _ => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "public output source cell {} is not backed by matching terminal evidence",
                    cell.cell_id
                )));
            }
        }
    }
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} is not backed by a render node",
            node.node_id
        )));
    };
    let expected_rendered_digest = public_output_rendered_digest(render, &payload.cells)?;
    if payload.rendered_digest != expected_rendered_digest {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} carries a rendered digest that does not match certified cells",
            node.node_id
        )));
    }
    let expected_receipt_digest = public_output_receipt_digest(
        render,
        &payload.cells,
        &expected_rendered_digest,
        payload.rendered_artifact_id.as_ref(),
    )?;
    let expected_receipt_artifact_id = ArtifactId::from_digest(
        expected_receipt_digest.algorithm(),
        *expected_receipt_digest.digest(),
    );
    let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "public output render node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let receipt_schema_id = spec::public_output_receipt_schema_id()?;
    match projections.cell_terminal(&node.output_cell) {
        Some(store::CellTerminalProjection::Produced {
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
            ..
        }) if schema_id == &receipt_schema_id
            && semantic_type_id == &output_cell.semantic_type_id
            && artifact_id == &expected_receipt_artifact_id
            && content_digest == &expected_receipt_digest => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} has no matching terminal receipt cell",
                node.node_id
            )));
        }
    }
    match projections.public_output(&payload.public_schema_id) {
        Some(store::PublicOutputProjection::Produced {
            event_id,
            rendered_digest,
            rendered_artifact_id,
        }) if event_id == event.event_id()
            && rendered_digest == &payload.rendered_digest
            && rendered_artifact_id == &payload.rendered_artifact_id =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "public output projection for schema {} does not match authoritative event",
            payload.public_schema_id
        ))),
    }
}

fn validate_historical_run_completed(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    event: &store::KernelEventEnvelope,
    payload: &events::RunCompleted,
    produced_public_output: Option<&events::PublicOutputCompletionEvidence>,
    retention_manifest_projected_seq: Option<store::StreamSeq>,
) -> Result<()> {
    if &payload.run_id != run_id {
        return Err(RuntimeError::InvalidRunStream(format!(
            "RunCompleted references run {} while validating {}",
            payload.run_id, run_id
        )));
    }
    let events::RunCompletionOutcome::Completed(completion) = &payload.outcome else {
        return Err(RuntimeError::InvalidRunStream(
            "RunCompleted failed/cancelled outcomes require sealed CompleteRun authority"
                .to_owned(),
        ));
    };
    if completion.public_output_schema_id != runtime_spec.spec().public_outputs.public_schema_id {
        return Err(RuntimeError::InvalidRunStream(format!(
            "RunCompleted references public schema {} outside certified public outputs",
            completion.public_output_schema_id
        )));
    }
    match produced_public_output {
        Some(produced) if produced == completion => Ok(()),
        Some(_) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted public-output evidence does not match preceding PublicOutputProduced"
                .to_owned(),
        )),
        None => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before PublicOutputProduced".to_owned(),
        )),
    }?;
    match retention_manifest_projected_seq {
        Some(seq) if seq < event.seq() => Ok(()),
        Some(_) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted must follow a prior retention manifest projection commit".to_owned(),
        )),
        None => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before retention manifest projection".to_owned(),
        )),
    }
}

fn validate_historical_public_output_failed(
    projections: &store::ProjectionSnapshot,
    payload: &events::PublicOutputRenderFailed,
) -> Result<()> {
    match require_projected_attempt(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        "public output failure",
    )? {
        store::AttemptStatus::Failed { .. } => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output failure for node {} attempt {} is not backed by a failed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    match projections.public_output(&payload.public_schema_id) {
        Some(store::PublicOutputProjection::Produced { .. })
        | Some(store::PublicOutputProjection::RenderFailed { .. }) => Ok(()),
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "public output failure for schema {} is not reflected in public-output projection",
            payload.public_schema_id
        ))),
    }
}

fn validate_spec_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let digest = canonical.content_digest();
    let expected_artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.artifact_id != expected_artifact_id
        || evidence.digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != runtime_spec.spec().media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedExecutionSpec
    {
        return Err(RuntimeError::InvalidRunStream(
            "typed execution spec artifact evidence does not match the certified spec".to_owned(),
        ));
    }
    Ok(evidence)
}

fn validate_certificate_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let digest = canonical.content_digest();
    let expected_artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let media_type = spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
    if evidence.artifact_id != expected_artifact_id
        || evidence.digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedSpecCertificate
    {
        return Err(RuntimeError::InvalidRunStream(
            "typed spec certificate artifact evidence does not match the certified spec".to_owned(),
        ));
    }
    Ok(evidence)
}

fn validate_config_artifacts(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: Vec<store::ArtifactEvidenceRef>,
) -> Result<Vec<store::ArtifactEvidenceRef>> {
    let mut by_key = BTreeMap::new();
    for artifact in evidence {
        let Some(schema_id) = artifact.schema_id.clone() else {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact evidence must carry schema_id".to_owned(),
            ));
        };
        if artifact.artifact_role != events::ArtifactRole::TypedConfig
            || artifact.semantic_type_id.is_some()
            || artifact.producer_node_id.is_some()
            || artifact.producer_seed_id.is_some()
        {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact evidence has invalid role or producer metadata".to_owned(),
            ));
        }
        let key = format!("{}:{}", schema_id, artifact.digest);
        if by_key.insert(key, artifact).is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "duplicate typed config artifact evidence".to_owned(),
            ));
        }
    }

    let mut validated = Vec::with_capacity(runtime_spec.spec().config_refs.len());
    for config in &runtime_spec.spec().config_refs {
        let key = config_ref_key(config);
        let artifact = by_key.remove(&key).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing typed config artifact evidence for schema {} digest {}",
                config.schema_id, config.digest
            ))
        })?;
        if artifact.artifact_id != config.artifact_id
            || artifact.digest != config.digest
            || artifact.byte_len != config.byte_len
            || artifact.media_type != config.media_type
            || artifact.schema_id.as_ref() != Some(&config.schema_id)
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "typed config artifact evidence for schema {} digest {} does not match certified config ref",
                config.schema_id, config.digest
            )));
        }
        validated.push(artifact);
    }
    if !by_key.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "typed config artifact evidence contains entries not certified by the spec".to_owned(),
        ));
    }
    Ok(validated)
}

fn config_artifacts_from_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<BTreeMap<String, store::ArtifactEvidenceRef>> {
    let (start_seq, start_commit_key) = stream
        .iter()
        .find(|event| matches!(event.payload(), events::KernelEventPayload::RunStarted(_)))
        .map(|event| (event.seq(), event.commit_key().clone()))
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "run has not started with certified RunStarted evidence".to_owned(),
            )
        })?;
    let mut referenced = Vec::new();
    for event in stream {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != events::ArtifactRole::TypedConfig {
            continue;
        }
        if payload.node_id.is_some() || payload.attempt_id.is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact references must not be scoped to a runner attempt"
                    .to_owned(),
            ));
        }
        if event.seq() != start_seq || event.commit_key() != &start_commit_key {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact references must be part of the RunStarted commit".to_owned(),
            ));
        }
        referenced.push(store_artifact_from_event_ref(
            &payload.artifact_ref,
            None,
            None,
        ));
    }
    let validated = validate_config_artifacts(runtime_spec, referenced)?;
    Ok(validated
        .into_iter()
        .map(|artifact| {
            let key = format!(
                "{}:{}",
                artifact
                    .schema_id
                    .as_ref()
                    .expect("validated config artifact has schema id"),
                artifact.digest
            );
            (key, artifact)
        })
        .collect())
}

fn artifact_refs_from_stream(
    stream: &[store::KernelEventEnvelope],
) -> Result<BTreeMap<ArtifactId, CommittedArtifactReference>> {
    let mut artifacts = BTreeMap::new();
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
            match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => {
                    for seed in &payload.seed_cells {
                        insert_committed_artifact(
                            &mut artifacts,
                            CommittedArtifactReference {
                                evidence: store_seed_artifact(seed),
                                attempt_id: None,
                                commit_seq: event.seq(),
                                commit_key: event.commit_key().clone(),
                            },
                        )?;
                    }
                }
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if staged_artifact_binding_kind(payload.artifact_ref.role).is_some() =>
                {
                    let (Some(node_id), Some(attempt_id)) =
                        (payload.node_id.clone(), payload.attempt_id.clone())
                    else {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "staged artifact reference {} must be scoped to a producer attempt",
                            payload.artifact_ref.artifact_id
                        )));
                    };
                    if !artifact_reference_matches_same_commit_payload(commit, payload) {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "staged artifact reference {} is not bound to a same-commit typed payload",
                            payload.artifact_ref.artifact_id
                        )));
                    }
                    if payload.artifact_ref.role == events::ArtifactRole::StateOutput {
                        insert_committed_artifact(
                            &mut artifacts,
                            CommittedArtifactReference {
                                evidence: store_artifact_from_event_ref(
                                    &payload.artifact_ref,
                                    Some(node_id),
                                    None,
                                ),
                                attempt_id: Some(attempt_id),
                                commit_seq: event.seq(),
                                commit_key: event.commit_key().clone(),
                            },
                        )?;
                    }
                }
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role != events::ArtifactRole::TypedConfig =>
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "unsupported artifact reference role {} for {}",
                        artifact_role_name(payload.artifact_ref.role),
                        payload.artifact_ref.artifact_id
                    )));
                }
                _ => {}
            }
        }
        index = end;
    }
    Ok(artifacts)
}

fn artifact_reference_matches_same_commit_payload(
    commit: &[store::KernelEventEnvelope],
    reference: &events::ArtifactReferenced,
) -> bool {
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

fn insert_committed_artifact(
    artifacts: &mut BTreeMap<ArtifactId, CommittedArtifactReference>,
    artifact: CommittedArtifactReference,
) -> Result<()> {
    if let Some(existing) = artifacts.get(&artifact.evidence.artifact_id) {
        if existing != &artifact {
            return Err(RuntimeError::InvalidRunStream(format!(
                "conflicting committed artifact evidence for {}",
                artifact.evidence.artifact_id
            )));
        }
        return Ok(());
    }
    artifacts.insert(artifact.evidence.artifact_id.clone(), artifact);
    Ok(())
}

fn committed_config_artifact(
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<store::ArtifactEvidenceRef> {
    let key = config_ref_key(&node.config_ref);
    let artifact = view.config_artifacts.get(&key).ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "node {} config artifact {} is not committed in the run stream",
            node.node_id, node.config_ref.artifact_id
        ))
    })?;
    if artifact.artifact_id != node.config_ref.artifact_id
        || artifact.digest != node.config_ref.digest
        || artifact.byte_len != node.config_ref.byte_len
        || artifact.media_type != node.config_ref.media_type
        || artifact.schema_id.as_ref() != Some(&node.config_ref.schema_id)
        || artifact.semantic_type_id.is_some()
        || artifact.producer_node_id.is_some()
        || artifact.producer_seed_id.is_some()
        || artifact.artifact_role != events::ArtifactRole::TypedConfig
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "node {} committed config artifact evidence does not match certified config ref",
            node.node_id
        )));
    }
    Ok(artifact.clone())
}

fn committed_input_artifact(
    view: &RuntimeRunView,
    cell_id: &CellId,
    artifact_id: &ArtifactId,
) -> Result<CommittedArtifactReference> {
    view.artifact_refs.get(artifact_id).cloned().ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {cell_id} artifact {artifact_id} is not committed in the run stream",
        ))
    })
}

fn store_artifact_from_event_ref(
    evidence: &events::ArtifactEvidenceRef,
    producer_node_id: Option<NodeId>,
    producer_seed_id: Option<mfm_ids::SeedId>,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        digest: evidence.content_digest.clone(),
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
        schema_id: Some(evidence.schema_id.clone()),
        semantic_type_id: evidence.semantic_type_id.clone(),
        producer_node_id,
        producer_seed_id,
        artifact_role: evidence.role,
    }
}

fn event_artifact_ref_from_store(
    artifact: &store::ArtifactEvidenceRef,
) -> Result<events::ArtifactEvidenceRef> {
    let schema_id = artifact.schema_id.clone().ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "artifact {} cannot be referenced without schema id",
            artifact.artifact_id
        ))
    })?;
    Ok(events::ArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id,
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    })
}

fn runner_payloads_with_derived_lifecycle(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    runner_payloads: Vec<RunnerEventPayload>,
) -> Result<Vec<events::KernelEventPayload>> {
    let mut payloads = runner_payloads
        .into_iter()
        .map(events::KernelEventPayload::from)
        .collect::<Vec<_>>();
    let mut terminal_cell = false;
    let mut failure: Option<(bool, events::MfmErrorInfo)> = None;
    for payload in &payloads {
        match payload {
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::CellSkipped(_) => {
                terminal_cell = true;
            }
            events::KernelEventPayload::SideEffectFailed(payload) => {
                if failure
                    .replace((payload.retryable, payload.error.clone()))
                    .is_some()
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                if failure
                    .replace((payload.error.retryable, payload.error.clone()))
                    .is_some()
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
            }
            _ => {}
        }
    }

    if let Some((retryable, error)) = failure {
        payloads.push(events::KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                retryable,
                error,
            },
        ));
    } else if terminal_cell {
        payloads.push(events::KernelEventPayload::StateAttemptCompleted(
            events::StateAttemptCompleted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: node.output_cell.clone(),
            },
        ));
    }

    Ok(payloads)
}

fn validate_runner_output(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    recorded_facts: &RecordedFacts,
    projections: &store::ProjectionSnapshot,
    payloads: &[events::KernelEventPayload],
) -> Result<()> {
    if payloads.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let mut completed = false;
    let mut failed = false;
    let mut terminal_cell = false;
    let mut public_output_produced = false;
    let mut public_output_failed = false;
    let mut side_effect_payload = false;
    let mut side_effect_failed = false;
    let mut attempt_failure_retryable = None;
    let mut side_effect_failure_retryable = None;
    for payload in payloads {
        if payload_spec_hash(payload) != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "runner for node {} returned payload with mismatched spec hash",
                node.node_id
            )));
        }
        match payload {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} completed output cell {} instead of certified {}",
                        node.node_id, payload.output_cell_id, node.output_cell
                    )));
                }
                completed = true;
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if failed {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
                attempt_failure_retryable = Some(payload.retryable);
                failed = true;
            }
            events::KernelEventPayload::CellProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced cell metadata outside certified spec",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                    || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped a cell outside certified skip policy",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if !recorded_facts.is_empty() {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} attempted to record fact {} after committed facts existed for the same attempt",
                        node.node_id, payload.fact_key
                    )));
                }
                require_capability(
                    caps,
                    &payload.capability_kind,
                    &payload.capability_version,
                    &node.node_id,
                )?;
                require_adapter(node, &payload.adapter_kind, &payload.adapter_version)?;
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned artifact reference payload for {}",
                    node.node_id, payload.artifact_ref.artifact_id
                )));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output(runtime_spec, node, payload)?;
                public_output_produced = true;
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )?;
                public_output_failed = true;
            }
            events::KernelEventPayload::RunStarted(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned scheduler-owned payload",
                    node.node_id
                )));
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {
                validate_runner_side_effect_payload(node, attempt_id, caps, projections, payload)?;
                side_effect_payload = true;
                if let events::KernelEventPayload::SideEffectFailed(payload) = payload {
                    side_effect_failed = true;
                    side_effect_failure_retryable = Some(payload.retryable);
                }
            }
        }
    }
    if node.side_effect.is_some() {
        validate_side_effect_resume_output(projections, node, attempt_id, payloads)?;
        if failed {
            if !side_effect_failed {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned StateAttemptFailed without SideEffectFailed",
                    node.node_id
                )));
            }
            if side_effect_failure_retryable != attempt_failure_retryable {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned inconsistent failure retryability",
                    node.node_id
                )));
            }
            if completed || terminal_cell || public_output_produced {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} mixed side-effect failure with successful terminal evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if side_effect_failed {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} returned SideEffectFailed without StateAttemptFailed",
                node.node_id
            )));
        }
        if side_effect_payload {
            if completed || terminal_cell || public_output_produced || public_output_failed {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} mixed ledger phase events with terminal output evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if !completed || !terminal_cell {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} output commit must pair StateAttemptCompleted with terminal cell evidence",
                node.node_id
            )));
        }
        validate_side_effect_terminal_evidence(projections, node, attempt_id)
            .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        if public_output_produced && !completed {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} projected public output without completing its certified output cell",
                node.node_id
            )));
        }
        return Ok(());
    }
    if failed && (completed || terminal_cell || public_output_produced) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} mixed failure with successful terminal evidence",
            node.node_id
        )));
    }
    if public_output_failed && !failed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned public-output failure without StateAttemptFailed",
            node.node_id
        )));
    }
    if !failed && (!completed || !terminal_cell) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} successful terminal commit must pair StateAttemptCompleted with terminal cell evidence",
            node.node_id
        )));
    }
    if public_output_produced && !completed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} projected public output without completing its certified output cell",
            node.node_id
        )));
    }
    Ok(())
}

fn validate_runner_side_effect_payload(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    projections: &store::ProjectionSnapshot,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    if node.side_effect.is_none() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "non-side-effect node {} returned side-effect payload",
            node.node_id
        )));
    }
    let (payload_node_id, payload_attempt_id, _, _) =
        side_effect_payload_ref(payload).ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
        })?;
    require_attempt(node, attempt_id, payload_node_id, payload_attempt_id)?;
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if payload.scope_id != node.scope_id {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} persisted intent for uncertified scope {}",
                    node.node_id, payload.scope_id
                )));
            }
            require_capability(
                caps,
                &payload.capability_kind,
                &payload.capability_version,
                &node.node_id,
            )?;
            require_adapter(node, &payload.adapter_kind, &payload.adapter_version)?;
        }
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            if payload.new_claim_owner == payload.previous_claim_owner {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} takeover reused the previous claim owner",
                    node.node_id
                )));
            }
            if payload.claim_generation <= payload.previous_claim_generation {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} takeover did not increase claim generation",
                    node.node_id
                )));
            }
            if let Some(projection) =
                side_effect_projection_for_attempt(projections, node, attempt_id)?
            {
                let claim = projection.claim.as_ref().ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover requires an active claim",
                        node.node_id
                    ))
                })?;
                if payload.claim_fencing_token == claim.claim_fencing_token {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover reused the previous fencing token",
                        node.node_id
                    )));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_side_effect_resume_output(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Ok(());
    };
    let has_takeover = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectClaimTakenOver(_)
        )
    });
    let has_claim = payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::SideEffectClaimed(_)));
    let has_invocation_prepared = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationPrepared(_)
        )
    });
    let has_invocation_started = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationStarted(_)
        )
    });
    let has_submission_recovery = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectNotSubmittedProven(_)
                | events::KernelEventPayload::SideEffectSubmissionObserved(_)
                | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
                | events::KernelEventPayload::SideEffectAmbiguous(_)
        )
    });

    match projection.phase {
        store::SideEffectPhase::Claimed { .. } => {
            if !has_takeover && !has_invocation_prepared {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed claimed ledger {} without takeover or prepared invocation",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::InvocationPrepared { .. } => {
            if !has_takeover && !has_invocation_started {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed prepared ledger {} without takeover or invocation start",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::NotSubmittedProven { .. } => {
            if !has_claim {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed not-submitted ledger {} without next-epoch claim",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::InvocationStarted { .. }
        | store::SideEffectPhase::SubmissionUnknown { .. } => {
            if !has_submission_recovery {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed uncertain submission ledger {} without submission recovery evidence",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::Ambiguous { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run ambiguous ledger {}",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectPhase::Failed { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run failed ledger {} on the same attempt",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectPhase::IntentPersisted { .. }
        | store::SideEffectPhase::SubmissionObserved { .. }
        | store::SideEffectPhase::ReceiptObserved { .. }
        | store::SideEffectPhase::ConfirmationObserved { .. } => {}
    }

    if has_invocation_started
        && matches!(projection.phase, store::SideEffectPhase::Claimed { .. })
        && !has_takeover
        && !has_invocation_prepared
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect node {} started invocation from stale claim without takeover",
            node.node_id
        )));
    }

    Ok(())
}

fn validate_public_output(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    validate_public_output_render_node(
        runtime_spec,
        node,
        payload.public_schema_id.clone(),
        &payload.renderer_descriptor_id,
    )?;
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not certified as a public-output render node",
            node.node_id
        )));
    };
    if payload.output_spec_digest != render.output_spec_digest
        || payload.receipt_cell_id != node.output_cell
        || payload.cells.len() != render.required_cells.len()
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "public-output payload for node {} does not match certified render contract",
            node.node_id
        )));
    }
    for (actual, expected) in payload.cells.iter().zip(render.required_cells.iter()) {
        if actual.public_field_path != expected.public_field_path
            || actual.cell_id != expected.cell_id
            || actual.producer != expected.producer
            || actual.scope_id != expected.scope_id
            || actual.semantic_type_id != expected.semantic_type_id
            || actual.schema_id != expected.schema_id
            || actual.value_lineage != expected.value_lineage
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "public-output cell evidence for node {} does not match certified output",
                node.node_id
            )));
        }
    }
    Ok(())
}

fn validate_public_output_render_node(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    public_schema_id: SchemaId,
    renderer_descriptor_id: &DescriptorId,
) -> Result<()> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not certified as a public-output render node",
            node.node_id
        )));
    };
    if render.public_schema_id != public_schema_id
        || runtime_spec.spec().public_outputs.public_schema_id != public_schema_id
        || render.renderer_descriptor.descriptor_id != *renderer_descriptor_id
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "public-output render metadata for node {} does not match certified spec",
            node.node_id
        )));
    }
    Ok(())
}

fn require_attempt(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    actual_node_id: &NodeId,
    actual_attempt_id: &AttemptId,
) -> Result<()> {
    if actual_node_id == &node.node_id && actual_attempt_id == attempt_id {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "payload for node {} attempt {} was bound to node {} attempt {}",
            node.node_id, attempt_id, actual_node_id, actual_attempt_id
        )))
    }
}

fn require_capability(
    caps: &CertifiedRuntimeCapabilities,
    kind: &CapabilityKind,
    version: &CapabilityVersion,
    node_id: &NodeId,
) -> Result<()> {
    if caps.contains(kind, version) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {node_id} used uncertified capability {kind}:{version}"
        )))
    }
}

fn require_adapter(
    node: &spec::NodeSpec,
    kind: &AdapterKind,
    version: &AdapterVersion,
) -> Result<()> {
    if node
        .adapter_bindings
        .iter()
        .any(|adapter| adapter.adapter_kind == *kind && adapter.adapter_version == *version)
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} used uncertified adapter {}:{}",
            node.node_id, kind, version
        )))
    }
}

fn next_attempt_no(projections: &store::ProjectionSnapshot, node_id: &NodeId) -> Result<u32> {
    let count = projections
        .attempts()
        .filter(|((attempt_node_id, _), _)| attempt_node_id == node_id)
        .count();
    u32::try_from(count + 1).map_err(|_| {
        RuntimeError::InvalidRunStream(format!("attempt count overflow for {node_id}"))
    })
}

fn attempt_id(
    run_id: &RunId,
    spec_hash: &SpecHash,
    node_id: &NodeId,
    attempt_no: u32,
) -> Result<AttemptId> {
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

fn payload_spec_hash(payload: &events::KernelEventPayload) -> SpecHash {
    match payload {
        events::KernelEventPayload::RunStarted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptStarted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::FactRecorded(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::ArtifactReferenced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::CellProduced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::CellSkipped(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectClaimed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectAmbiguous(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::PublicOutputProduced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptCompleted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RunCompleted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RetentionRefsAppended(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RetentionManifestProjected(payload) => {
            payload.spec_hash.clone()
        }
    }
}

fn store_seed_artifact(seed: &events::SeedCellRef) -> store::ArtifactEvidenceRef {
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

fn config_artifact_reference_payloads(
    spec_hash: &SpecHash,
    artifacts: &[store::ArtifactEvidenceRef],
) -> Result<Vec<events::KernelEventPayload>> {
    artifacts
        .iter()
        .map(|artifact| {
            let schema_id = artifact.schema_id.clone().ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "config artifact {} is missing schema id",
                    artifact.artifact_id
                ))
            })?;
            Ok(events::KernelEventPayload::ArtifactReferenced(
                events::ArtifactReferenced {
                    spec_hash: spec_hash.clone(),
                    node_id: None,
                    attempt_id: None,
                    artifact_ref: events::ArtifactEvidenceRef {
                        artifact_id: artifact.artifact_id.clone(),
                        role: artifact.artifact_role,
                        schema_id,
                        semantic_type_id: artifact.semantic_type_id.clone(),
                        content_digest: artifact.digest.clone(),
                        byte_len: artifact.byte_len,
                        media_type: artifact.media_type.clone(),
                    },
                },
            ))
        })
        .collect()
}

fn retention_ref_for_artifact(artifact: &store::ArtifactEvidenceRef) -> events::RetentionRef {
    events::RetentionRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        content_digest: artifact.digest.clone(),
    }
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(&value)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))
}

fn content_digest_json(value: serde_json::Value) -> Result<ContentDigest> {
    Ok(canonical_json(value)?.content_digest())
}

fn config_ref_key(config_ref: &spec::ConfigRef) -> String {
    format!("{}:{}", config_ref.schema_id, config_ref.digest)
}

#[cfg(test)]
mod tests;
