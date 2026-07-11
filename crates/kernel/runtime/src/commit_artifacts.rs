use super::*;

pub(super) fn required_artifacts_for_payloads(
    view: &RuntimeRunView,
    required_artifacts: Vec<store::ArtifactEvidenceRef>,
    payloads: &[events::KernelEventPayload],
) -> Result<Vec<store::ArtifactEvidenceRef>> {
    let mut required_artifacts = required_artifacts;
    for payload in payloads {
        for requirement in store::event_artifact_requirements(payload) {
            if required_artifacts.iter().any(|evidence| {
                evidence.artifact_id == requirement.artifact_id
                    && store::validate_artifact_requirement_against_evidence(&requirement, evidence)
                        .is_ok()
            }) {
                continue;
            }
            let Some(evidence) = committed_artifact_for_requirement(view, &requirement) else {
                if requirement.source.is_retention() {
                    if let Some(evidence) =
                        retained_fact_artifact_evidence_for_requirement(view, &requirement)?
                    {
                        required_artifacts.push(evidence);
                    }
                    continue;
                }
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "payload references artifact {} without required evidence",
                    requirement.artifact_id
                )));
            };
            required_artifacts.push(evidence);
        }
    }
    Ok(required_artifacts)
}

pub(super) fn retained_fact_artifact_evidence_for_requirement(
    view: &RuntimeRunView,
    requirement: &store::EventArtifactRequirement,
) -> Result<Option<store::ArtifactEvidenceRef>> {
    match requirement.artifact_role {
        Some(events::ArtifactRole::FactDescriptor) => {
            let descriptor_hash = requirement.digest.as_ref().ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "retention ref for artifact {} lacks descriptor digest",
                    requirement.artifact_id
                ))
            })?;
            let Some(descriptor) = view
                .projections
                .fact_descriptor(descriptor_hash)
                .filter(|descriptor| descriptor.descriptor_artifact_id == requirement.artifact_id)
                .filter(|descriptor| {
                    descriptor
                        .descriptor_artifact_evidence
                        .evidence_hash()
                        .is_ok_and(|actual| actual == requirement.evidence_hash)
                })
            else {
                return Ok(None);
            };
            Ok(Some(descriptor.descriptor_artifact_evidence.clone()))
        }
        Some(events::ArtifactRole::FactResponse) => Ok(view
            .projections
            .fact_index_entries()
            .find(|(_, index)| {
                index.artifact_id == requirement.artifact_id
                    && requirement
                        .digest
                        .as_ref()
                        .is_some_and(|digest| index.response_hash == *digest)
                    && index.artifact_evidence_hash == requirement.evidence_hash
            })
            .and_then(|(claim_id, _index)| {
                let record = view.projections.fact_record(claim_id)?;
                let evidence = record.response_artifact_evidence.as_ref()?;
                let evidence_hash = evidence.evidence_hash().ok()?;
                (evidence_hash == requirement.evidence_hash).then(|| evidence.clone())
            })),
        _ => Ok(None),
    }
}

pub(super) fn committed_artifact_for_requirement(
    view: &RuntimeRunView,
    requirement: &store::EventArtifactRequirement,
) -> Option<store::ArtifactEvidenceRef> {
    view.artifact_refs
        .get(&(
            requirement.artifact_id.clone(),
            requirement.evidence_hash.clone(),
        ))
        .filter(|reference| {
            store::validate_artifact_requirement_against_evidence(requirement, &reference.evidence)
                .is_ok()
        })
        .map(|reference| reference.evidence.clone())
}

pub(super) fn launch_artifacts_by_evidence(
    artifacts: Vec<RunLaunchArtifact>,
    kind: &'static str,
) -> Result<BTreeMap<(ArtifactId, ContentDigest), RunLaunchArtifact>> {
    let mut by_evidence = BTreeMap::new();
    for artifact in artifacts {
        verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
        let evidence_hash = artifact.evidence.evidence_hash()?;
        let key = (artifact.evidence.artifact_id.clone(), evidence_hash);
        if by_evidence.insert(key, artifact).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate staged {kind} launch artifact"
            )));
        }
    }
    Ok(by_evidence)
}

pub(super) fn validate_fact_descriptor_launch_artifacts(
    runtime_spec: &CertifiedRuntimeSpec,
    artifacts: Vec<RunLaunchArtifact>,
) -> Result<Vec<RunLaunchArtifact>> {
    let required = runtime_spec.fact_descriptor_hashes();
    let schema_id = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
    let media_type = spec::MediaType::new("application/json")?;
    let mut by_hash = BTreeMap::new();

    for artifact in artifacts {
        verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
        let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(&artifact.bytes)
            .map_err(runtime_fact_error)?;
        let canonical =
            mfm_facts::canonical_fact_descriptor_bytes(&descriptor).map_err(runtime_fact_error)?;
        let descriptor_hash =
            mfm_facts::fact_descriptor_hash(&descriptor).map_err(runtime_fact_error)?;
        let expected_artifact_id =
            ArtifactId::from_digest(descriptor_hash.algorithm(), *descriptor_hash.digest());
        if artifact.evidence.artifact_id != expected_artifact_id
            || artifact.evidence.digest != descriptor_hash
            || artifact.evidence.byte_len != canonical.as_bytes().len() as u64
            || artifact.evidence.media_type != media_type
            || artifact.evidence.schema_id.as_ref() != Some(&schema_id)
            || artifact.evidence.semantic_type_id.is_some()
            || artifact.evidence.producer_node_id.is_some()
            || artifact.evidence.producer_seed_id.is_some()
            || artifact.evidence.artifact_role != events::ArtifactRole::FactDescriptor
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "fact descriptor artifact evidence does not match descriptor {}",
                descriptor_hash
            )));
        }
        if by_hash.insert(descriptor_hash.clone(), artifact).is_some() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "duplicate fact descriptor artifact for {}",
                descriptor_hash
            )));
        }
    }

    for required_hash in &required {
        if !by_hash.contains_key(required_hash) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "missing fact descriptor artifact for certified descriptor {}",
                required_hash
            )));
        }
    }
    for admitted_hash in by_hash.keys() {
        if !required.contains(admitted_hash) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "fact descriptor artifact {} is not certified by the runtime spec",
                admitted_hash
            )));
        }
    }

    Ok(by_hash.into_values().collect())
}

pub(super) fn validate_launch_seed_artifacts<'a>(
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

pub(super) fn runner_output_commit_key(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<store::CommitKey> {
    let mut fragments = BTreeSet::new();
    for (payload_ordinal, payload) in payloads.iter().enumerate() {
        fragments.insert(runner_output_commit_fragment(payload_ordinal, payload)?);
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

pub(super) fn runner_output_commit_fragment(
    payload_ordinal: usize,
    payload: &events::KernelEventPayload,
) -> Result<String> {
    Ok(match payload {
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
        events::KernelEventPayload::FactRecorded(payload) => {
            let payload_hash = store::payload_canonical_json(
                &events::KernelEventPayload::FactRecorded(payload.clone()),
            )?
            .content_digest();
            format!("fact-payload:{}:{}", payload_ordinal, payload_hash)
        }
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
            format!("sidefx-intent:{}", payload.pair_id)
        }
        events::KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx-claim:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx-claim-takeover:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::ResourceLaneClaimed(payload) => format!(
            "resource-lane-claimed:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.claim_id
        ),
        events::KernelEventPayload::ResourceLaneClaimIntent(payload) => format!(
            "resource-lane-claim-intent:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx-prepared:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx-started:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx-not-submitted:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx-submission:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx-submission-unknown:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx-receipt:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx-confirmation:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!("sidefx-ambiguous:{}", payload.pair_id)
        }
        events::KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx-failed:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::ResourceLaneReleased(payload) => format!(
            "resource-lane-released:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.release_id
        ),
        events::KernelEventPayload::ResourceLaneReleaseIntent(payload) => format!(
            "resource-lane-release-intent:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.claim_id
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
        events::KernelEventPayload::RunAdmitted(_)
        | events::KernelEventPayload::ManualResolutionRecorded(_)
        | events::KernelEventPayload::RunCompleted(_)
        | events::KernelEventPayload::StateAttemptStarted(_)
        | events::KernelEventPayload::StateAttemptInterrupted(_) => "scheduler-owned".to_owned(),
    })
}

pub(super) fn attempt_failure_commit_fragment(
    payload: &events::StateAttemptFailed,
) -> Result<String> {
    Ok(content_digest_json(serde_json::json!({
        "code": payload.error.code.as_str(),
        "retryable": payload.retryable,
    }))?
    .to_string())
}

pub(super) fn validate_attempt_failure_diagnostic_artifact(
    node: &spec::NodeSpec,
    artifact: PreparedStagedArtifact,
) -> Result<PreparedStagedArtifact> {
    verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
    if artifact.evidence.artifact_role != events::ArtifactRole::RedactedDiagnostic {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} failure diagnostic artifact has role {}",
            node.node_id,
            artifact_role_name(artifact.evidence.artifact_role)
        )));
    }
    if artifact.evidence.producer_node_id.as_ref() != Some(&node.node_id)
        || artifact.evidence.producer_seed_id.is_some()
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} failure diagnostic artifact {} has invalid producer evidence",
            node.node_id, artifact.evidence.artifact_id
        )));
    }
    if artifact.evidence.schema_id.is_none() || artifact.evidence.semantic_type_id.is_some() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} failure diagnostic artifact {} has invalid typing evidence",
            node.node_id, artifact.evidence.artifact_id
        )));
    }
    if artifact.evidence.media_type != spec::MediaType::new("application/json")? {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} failure diagnostic artifact {} has unsupported media type",
            node.node_id, artifact.evidence.artifact_id
        )));
    }
    Ok(artifact)
}

pub(super) fn validate_staged_artifacts(
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    staged_artifacts: &[StagedArtifact],
) -> Result<Vec<ValidatedStagedArtifact>> {
    let mut by_artifact = BTreeMap::<(ArtifactId, ContentDigest), ValidatedStagedArtifact>::new();
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
        let evidence_key = (
            handle.evidence().artifact_id.clone(),
            handle
                .evidence()
                .evidence_hash()
                .map_err(RuntimeError::from)?,
        );
        if let Some(existing) = by_artifact.get(&evidence_key) {
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
            evidence_key,
            ValidatedStagedArtifact {
                evidence: handle.evidence().clone(),
                binding: handle.binding().clone(),
                bytes: staged.bytes().map(ToOwned::to_owned),
            },
        );
    }
    Ok(by_artifact.into_values().collect())
}

pub(super) fn framework_retention_manifest_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    pre_projection_stream: &[store::KernelEventEnvelope],
    artifact_bytes: &store::ArtifactByteAuthorityMap,
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
    let expected = build_retention_manifest_artifact(
        runtime_spec,
        run_id,
        pre_projection_stream,
        artifact_bytes,
    )?;
    if staged.evidence != expected.evidence || bytes != expected.bytes.as_bytes() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged manifest outside authoritative stream",
            node.node_id
        )));
    }
    Ok(Some(expected))
}

pub(crate) fn retention_manifest_payloads(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    manifest: RetentionManifestArtifact,
) -> Result<Vec<events::KernelEventPayload>> {
    let manifest_ref = manifest.evidence.retention_ref()?;
    Ok(vec![
        events::KernelEventPayload::RetentionManifestProjected(
            events::RetentionManifestProjected {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                manifest_seq: manifest.manifest_seq,
                manifest_digest: manifest.evidence.digest.clone(),
                previous_manifest_digest: manifest.previous_manifest_digest,
                manifest_artifact_id: manifest.evidence.artifact_id.clone(),
                manifest_artifact_evidence_hash: manifest.evidence.evidence_hash()?,
            },
        ),
        events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            refs: vec![manifest_ref],
            reason: events::RetentionReason::ManifestProjection,
        }),
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidatedStagedArtifact {
    pub(super) evidence: store::ArtifactEvidenceRef,
    pub(super) binding: StagedArtifactBindingKind,
    pub(super) bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StagedArtifactRequirement {
    artifact_id: ArtifactId,
    evidence_hash: ContentDigest,
    digest: ContentDigest,
    byte_len: Option<u64>,
    media_type: Option<spec::MediaType>,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<SemanticTypeId>,
    role: events::ArtifactRole,
    binding: StagedArtifactBindingKind,
}

pub(super) fn validate_staged_artifact_payload_bindings(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Result<()> {
    let requirements = staged_payload_artifact_requirements(node, attempt_id, payloads)?;
    for staged in staged_artifacts {
        if matches!(
            staged.binding,
            StagedArtifactBindingKind::FactQueryEvidence
                | StagedArtifactBindingKind::ExternalReadEvidence
        ) {
            continue;
        }
        if !requirements
            .iter()
            .any(|requirement| staged_artifact_matches_requirement(node, staged, requirement))
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} without typed payload reference",
                node.node_id, staged.evidence.artifact_id
            )));
        }
    }

    for requirement in &requirements {
        if !staged_artifacts
            .iter()
            .any(|staged| staged_artifact_matches_requirement(node, staged, requirement))
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} referenced artifact {} without staged artifact",
                node.node_id, requirement.artifact_id
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_context_bound_output_artifacts(
    node: &spec::NodeSpec,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
    extractor: Option<&dyn ContextOutputExtractor>,
) -> Result<()> {
    let output_context = payloads.iter().find_map(|payload| match payload {
        events::KernelEventPayload::CellProduced(payload)
            if payload.cell_id == node.output_cell =>
        {
            Some((
                &payload.context,
                &payload.artifact_id,
                &payload.content_digest,
                &payload.evidence_hash,
            ))
        }
        _ => None,
    });
    let Some((context, artifact_id, digest, evidence_hash)) = output_context else {
        return Ok(());
    };
    if matches!(context, spec::CellContextSpec::NoContext) {
        return Ok(());
    }
    let extractor = extractor.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} produced context-bound output without a registered context output extractor",
            node.node_id
        ))
    })?;
    let staged = staged_artifacts
        .iter()
        .find(|artifact| {
            artifact.binding == StagedArtifactBindingKind::StateOutput
                && artifact.evidence.artifact_id == *artifact_id
                && artifact.evidence.digest == *digest
                && artifact
                    .evidence
                    .evidence_hash()
                    .is_ok_and(|actual| actual == *evidence_hash)
        })
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "node {} produced context-bound artifact {} without staged artifact evidence",
                node.node_id, artifact_id
            ))
        })?;
    let bytes = staged.bytes.as_deref().ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} produced context-bound artifact {} without staged bytes",
            node.node_id, artifact_id
        ))
    })?;
    extractor.validate_context_output(context, &staged.evidence, bytes)
}

fn staged_artifact_matches_requirement(
    node: &spec::NodeSpec,
    staged: &ValidatedStagedArtifact,
    requirement: &StagedArtifactRequirement,
) -> bool {
    validate_staged_artifact_requirement(node, &staged.evidence, &staged.binding, requirement)
        .is_ok()
}

fn validate_staged_artifact_requirement(
    node: &spec::NodeSpec,
    evidence: &store::ArtifactEvidenceRef,
    binding: &StagedArtifactBindingKind,
    requirement: &StagedArtifactRequirement,
) -> Result<()> {
    let actual_evidence_hash = evidence.evidence_hash()?;
    if evidence.artifact_id != requirement.artifact_id
        || actual_evidence_hash != requirement.evidence_hash
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

fn staged_payload_artifact_requirements(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<Vec<StagedArtifactRequirement>> {
    let mut requirements = Vec::new();
    for payload in payloads {
        for requirement in store::event_artifact_requirements(payload) {
            let Some(binding) =
                staged_payload_artifact_binding(node, attempt_id, payload, requirement.source)?
            else {
                continue;
            };
            requirements.push(staged_artifact_requirement_from_event_requirement(
                node,
                requirement,
                binding,
            )?);
        }
    }
    Ok(requirements)
}

pub(super) fn staged_payload_artifact_binding(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payload: &events::KernelEventPayload,
    source: store::EventArtifactReferenceSource,
) -> Result<Option<StagedArtifactBindingKind>> {
    if let Some(phase) = staged_side_effect_artifact_phase_for_source(source) {
        return match payload.side_effect_ref() {
            Some(side_effect) => {
                staged_side_effect_artifact_binding(node, attempt_id, side_effect, phase)
            }
            None => Ok(None),
        };
    }

    match (payload, source) {
        (
            events::KernelEventPayload::FactRecorded(payload),
            store::EventArtifactReferenceSource::FactResponse,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::FactResponse))
        }
        (
            events::KernelEventPayload::CellProduced(payload),
            store::EventArtifactReferenceSource::StateOutput,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::StateOutput))
        }
        (
            events::KernelEventPayload::PublicOutputProduced(payload),
            store::EventArtifactReferenceSource::PublicOutputRendered,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::PublicOutput))
        }
        (
            events::KernelEventPayload::PublicOutputRenderFailed(payload),
            store::EventArtifactReferenceSource::PublicOutputRenderFailureDiagnostic,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::RedactedDiagnostic))
        }
        (
            events::KernelEventPayload::StateAttemptFailed(payload),
            store::EventArtifactReferenceSource::StateAttemptFailureDiagnostic,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::RedactedDiagnostic))
        }
        (
            events::KernelEventPayload::ArtifactReferenced(payload),
            store::EventArtifactReferenceSource::ArtifactReferenced,
        ) if payload.artifact_ref.role == events::ArtifactRole::ExternalReadEvidence => {
            let node_id = payload.node_id.as_ref().ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "node {} external read evidence reference lacks a producer node",
                    node.node_id
                ))
            })?;
            let referenced_attempt_id = payload.attempt_id.as_ref().ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "node {} external read evidence reference lacks an attempt",
                    node.node_id
                ))
            })?;
            require_attempt(node, attempt_id, node_id, referenced_attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::ExternalReadEvidence))
        }
        _ => Ok(None),
    }
}

pub(super) fn staged_side_effect_artifact_binding(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    side_effect: events::SideEffectEventRef<'_>,
    phase: StagedSideEffectArtifactPhase,
) -> Result<Option<StagedArtifactBindingKind>> {
    require_attempt(
        node,
        attempt_id,
        side_effect.node_id,
        side_effect.attempt_id,
    )?;
    let invocation_epoch = side_effect.invocation_epoch.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} side-effect artifact payload lacks invocation epoch",
            node.node_id
        ))
    })?;
    Ok(Some(StagedArtifactBindingKind::SideEffectEvidence {
        ledger_key: side_effect.ledger_key.clone(),
        invocation_epoch,
        phase,
    }))
}

pub(super) fn staged_side_effect_artifact_phase_for_source(
    source: store::EventArtifactReferenceSource,
) -> Option<StagedSideEffectArtifactPhase> {
    let role = match source {
        store::EventArtifactReferenceSource::SideEffectIntent => {
            events::ArtifactRole::SideEffectIntent
        }
        store::EventArtifactReferenceSource::PreparedInvocation => {
            events::ArtifactRole::PreparedInvocation
        }
        store::EventArtifactReferenceSource::NotSubmittedProof => {
            events::ArtifactRole::NotSubmittedProof
        }
        store::EventArtifactReferenceSource::Submission => events::ArtifactRole::Submission,
        store::EventArtifactReferenceSource::SubmissionUnknownEvidence => {
            events::ArtifactRole::SubmissionUnknownEvidence
        }
        store::EventArtifactReferenceSource::Receipt => events::ArtifactRole::Receipt,
        store::EventArtifactReferenceSource::Confirmation => events::ArtifactRole::Confirmation,
        store::EventArtifactReferenceSource::AmbiguityEvidence => {
            events::ArtifactRole::AmbiguityEvidence
        }
        _ => return None,
    };
    staged_side_effect_artifact_phase(role)
}

fn staged_artifact_requirement_from_event_requirement(
    node: &spec::NodeSpec,
    requirement: store::EventArtifactRequirement,
    binding: StagedArtifactBindingKind,
) -> Result<StagedArtifactRequirement> {
    let role = requirement.artifact_role.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} typed payload requirement for artifact {} lacks artifact role",
            node.node_id, requirement.artifact_id
        ))
    })?;
    let digest = requirement.digest.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} typed payload requirement for artifact {} lacks digest",
            node.node_id, requirement.artifact_id
        ))
    })?;
    let expected_role = staged_artifact_binding_role(&binding);
    if role != expected_role {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} typed payload requirement role {} does not match staged binding",
            node.node_id,
            artifact_role_name(role)
        )));
    }
    Ok(StagedArtifactRequirement {
        artifact_id: requirement.artifact_id,
        evidence_hash: requirement.evidence_hash,
        digest,
        byte_len: requirement.byte_len,
        media_type: requirement.media_type,
        schema_id: requirement.schema_id,
        semantic_type_id: requirement.semantic_type_id,
        role,
        binding,
    })
}

pub(super) fn staged_artifact_reference_payloads(
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Result<Vec<events::KernelEventPayload>> {
    let existing_refs = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(&node.node_id)
                    && payload.attempt_id.as_ref() == Some(attempt_id) =>
            {
                Some((
                    payload.artifact_ref.artifact_id.clone(),
                    payload.artifact_ref.evidence_hash.clone(),
                ))
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut refs = Vec::new();
    for artifact in staged_artifacts {
        if staged_artifact_binding_kind(artifact.evidence.artifact_role).is_none() {
            continue;
        }
        let evidence_hash = artifact.evidence.evidence_hash()?;
        if existing_refs.contains(&(artifact.evidence.artifact_id.clone(), evidence_hash.clone())) {
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
                    evidence_hash,
                    byte_len: artifact.evidence.byte_len,
                    media_type: artifact.evidence.media_type.clone(),
                },
            },
        ));
    }
    Ok(refs)
}

pub(super) fn bind_staged_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    projections: &store::ProjectionSnapshot,
    required_artifacts: &[store::ArtifactEvidenceRef],
    staged: Vec<StagedRetentionRefs>,
) -> Result<Vec<events::KernelEventPayload>> {
    let mut payloads = Vec::with_capacity(staged.len());
    for staged_refs in staged {
        let reason = staged_refs.reason;
        validate_staged_retention_reason(runtime_spec, node, required_artifacts, &staged_refs)?;
        if staged_refs.refs.is_empty() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged empty retention refs",
                node.node_id
            )));
        }
        validate_fact_query_evidence_retention_set(projections, &staged_refs)?;
        for retention_ref in &staged_refs.refs {
            if let Some(artifact) =
                artifact_evidence_for_retention_ref(required_artifacts, retention_ref)
            {
                if artifact.digest != retention_ref.content_digest
                    || artifact.artifact_role != retention_ref.role
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} staged retention evidence for artifact {} does not match artifact evidence",
                        node.node_id, retention_ref.artifact_id
                    )));
                }
                continue;
            }

            if !staged_retention_ref_authorized_by_existing_fact_query_evidence(
                projections,
                &staged_refs,
                retention_ref,
            ) {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged retention for artifact {} without staged artifact evidence",
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

pub(super) fn validate_fact_query_evidence_retention_set(
    projections: &store::ProjectionSnapshot,
    staged_refs: &StagedRetentionRefs,
) -> Result<()> {
    let StagedRetentionRefAuthority::FactQueryEvidence { returned_refs } = &staged_refs.authority
    else {
        return Ok(());
    };

    if !staged_refs
        .refs
        .iter()
        .any(|reference| reference.role == events::ArtifactRole::FactQueryEvidence)
    {
        return Err(RuntimeError::InvalidRunnerOutput(
            "fact query evidence retention missing query evidence artifact".to_owned(),
        ));
    }

    for fact_ref in returned_refs {
        let [descriptor_ref, response_ref] =
            fact_query_returned_ref_retention_refs(projections, fact_ref)?;
        if !staged_refs.refs.contains(&descriptor_ref) {
            return Err(RuntimeError::InvalidRunnerOutput(
                "fact query evidence retention missing descriptor artifact authority".to_owned(),
            ));
        }

        if !staged_refs.refs.contains(&response_ref) {
            return Err(RuntimeError::InvalidRunnerOutput(
                "fact query evidence retention missing response artifact authority".to_owned(),
            ));
        }
    }

    Ok(())
}

pub(super) fn staged_retention_ref_authorized_by_existing_fact_query_evidence(
    projections: &store::ProjectionSnapshot,
    staged_refs: &StagedRetentionRefs,
    retention_ref: &events::RetentionRef,
) -> bool {
    let StagedRetentionRefAuthority::FactQueryEvidence { returned_refs } = &staged_refs.authority
    else {
        return false;
    };

    match retention_ref.role {
        events::ArtifactRole::FactDescriptor | events::ArtifactRole::FactResponse => {
            returned_refs.iter().any(|fact_ref| {
                fact_query_returned_ref_retention_refs(projections, fact_ref)
                    .map(|refs| refs.contains(retention_ref))
                    .unwrap_or(false)
            })
        }
        _ => false,
    }
}

pub(super) fn validate_staged_retention_reason(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    artifact_evidence: &[store::ArtifactEvidenceRef],
    staged_refs: &StagedRetentionRefs,
) -> Result<()> {
    match staged_refs.reason {
        events::RetentionReason::RunAdmitted | events::RetentionReason::ManifestProjection => {
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
                let artifact = artifact_evidence_for_retention_ref(artifact_evidence, retention_ref)
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

pub(super) fn artifact_evidence_for_retention_ref<'a>(
    artifacts: &'a [store::ArtifactEvidenceRef],
    retention_ref: &events::RetentionRef,
) -> Option<&'a store::ArtifactEvidenceRef> {
    artifacts.iter().find(|artifact| {
        artifact.artifact_id == retention_ref.artifact_id
            && artifact.digest == retention_ref.content_digest
            && artifact.artifact_role == retention_ref.role
            && artifact
                .evidence_hash()
                .ok()
                .is_some_and(|evidence_hash| evidence_hash == retention_ref.evidence_hash)
    })
}
