use super::*;

pub(super) fn validate_historical_retention_manifest_batches(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    history: &RuntimeCommittedHistory,
    artifact_bytes: &store::ArtifactByteAuthorityMap,
) -> Result<()> {
    for commit in history.commits() {
        validate_historical_retention_manifest_batch(
            runtime_spec,
            commit.prefix(stream),
            commit.events(stream),
            artifact_bytes,
        )?;
    }
    Ok(())
}

type RetentionRefKey = (ArtifactId, ContentDigest);
type TypedPayloadKey = (ArtifactId, ContentDigest, events::ArtifactRole);

pub(super) fn validate_historical_retention_ref_batches(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    history: &RuntimeCommittedHistory,
) -> Result<()> {
    for commit in history.commits() {
        validate_historical_retention_ref_batch(runtime_spec, commit.events(stream))?;
    }
    Ok(())
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
    if retention_refs.is_empty() {
        return Ok(());
    }

    let has_manifest_projection = commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::RetentionManifestProjected(_)
        )
    });
    let artifact_refs = same_commit_artifact_reference_keys(commit)?;
    let typed_payload_refs = same_commit_typed_artifact_keys(commit)?;

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
            events::RetentionReason::RunAdmitted => {
                return Err(RuntimeError::InvalidRunStream(
                    "RunAdmitted retention refs are projection-derived and must not be appended"
                        .to_owned(),
                ));
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
                    true,
                )?;
            }
            events::RetentionReason::PublicOutput => {
                validate_same_commit_retention_ref_evidence(
                    payload,
                    &artifact_refs,
                    &typed_payload_refs,
                    false,
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
    typed_payload_refs: &BTreeSet<TypedPayloadKey>,
    allow_fact_query_returned_refs: bool,
) -> Result<()> {
    let has_same_commit_fact_query_evidence = payload.refs.iter().any(|retention_ref| {
        retention_ref.role == events::ArtifactRole::FactQueryEvidence
            && artifact_refs.contains(&retention_ref_key(retention_ref))
            && typed_payload_refs.contains(&typed_payload_key(retention_ref))
    });
    for retention_ref in &payload.refs {
        if allow_fact_query_returned_refs
            && has_same_commit_fact_query_evidence
            && matches!(
                retention_ref.role,
                events::ArtifactRole::FactDescriptor | events::ArtifactRole::FactResponse
            )
        {
            continue;
        }
        let key = retention_ref_key(retention_ref);
        if staged_artifact_binding_kind(retention_ref.role).is_some()
            && !artifact_refs.contains(&key)
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention ref for artifact {} lacks same-commit artifact reference evidence",
                retention_ref.artifact_id
            )));
        }
        if !typed_payload_refs.contains(&typed_payload_key(retention_ref)) {
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
                let key = commit
                    .iter()
                    .filter_map(|event| match event.payload() {
                        events::KernelEventPayload::ArtifactReferenced(reference)
                            if reference.node_id.as_ref() == Some(&payload.node_id)
                                && reference.attempt_id.as_ref() == Some(&payload.attempt_id)
                                && reference.artifact_ref.artifact_id == payload.artifact_id
                                && reference.artifact_ref.content_digest
                                    == payload.content_digest
                                && reference.artifact_ref.evidence_hash
                                    == payload.evidence_hash
                                && reference.artifact_ref.role
                                    == events::ArtifactRole::StateOutput =>
                        {
                            Some(event_artifact_ref_key(
                                &reference.artifact_ref,
                                reference.node_id.clone(),
                                None,
                            ))
                        }
                        _ => None,
                    })
                    .next()
                    .transpose()?;
                if let Some(key) = key {
                    allowed.insert(key);
                }
            }
        }
    }
    if let Some(artifact_id) = &public_output.rendered_artifact_id {
        let key = commit
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::ArtifactReferenced(reference)
                    if reference.node_id.as_ref() == Some(&public_output.node_id)
                        && reference.attempt_id.as_ref() == Some(&public_output.attempt_id)
                        && reference.artifact_ref.artifact_id == *artifact_id
                        && reference.artifact_ref.content_digest
                            == public_output.rendered_digest
                        && public_output.rendered_artifact_evidence_hash.as_ref()
                            == Some(&reference.artifact_ref.evidence_hash)
                        && reference.artifact_ref.role == events::ArtifactRole::PublicOutput =>
                {
                    Some(event_artifact_ref_key(
                        &reference.artifact_ref,
                        reference.node_id.clone(),
                        None,
                    ))
                }
                _ => None,
            })
            .next()
            .transpose()?;
        if let Some(key) = key {
            allowed.insert(key);
        }
    }
    if allowed.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "public-output retention refs lack same-commit framework receipt/render evidence"
                .to_owned(),
        ));
    }
    Ok(allowed)
}

fn retention_ref_key(retention_ref: &events::RetentionRef) -> RetentionRefKey {
    (
        retention_ref.artifact_id.clone(),
        retention_ref.evidence_hash.clone(),
    )
}

fn event_artifact_ref_key(
    artifact: &events::ArtifactEvidenceRef,
    producer_node_id: Option<NodeId>,
    producer_seed_id: Option<mfm_ids::SeedId>,
) -> Result<RetentionRefKey> {
    let evidence = store_artifact_from_event_ref(artifact, producer_node_id, producer_seed_id);
    Ok((evidence.artifact_id.clone(), evidence.evidence_hash()?))
}

fn same_commit_artifact_reference_keys(
    commit: &[store::KernelEventEnvelope],
) -> Result<BTreeSet<RetentionRefKey>> {
    commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                Some((&payload.artifact_ref, payload.node_id.clone(), None))
            }
            _ => None,
        })
        .map(|(artifact, producer_node_id, producer_seed_id)| {
            event_artifact_ref_key(artifact, producer_node_id, producer_seed_id)
        })
        .collect()
}

fn same_commit_typed_artifact_keys(
    commit: &[store::KernelEventEnvelope],
) -> Result<BTreeSet<TypedPayloadKey>> {
    Ok(commit
        .iter()
        .flat_map(|event| store::event_artifact_requirements(event.payload()))
        .filter(|requirement| {
            requirement.source.is_same_commit_payload_evidence()
                || (requirement.source == store::EventArtifactReferenceSource::ArtifactReferenced
                    && matches!(
                        requirement.artifact_role,
                        Some(
                            events::ArtifactRole::FactQueryEvidence
                                | events::ArtifactRole::ExternalReadEvidence
                        )
                    ))
        })
        .filter_map(|requirement| {
            Some((
                requirement.artifact_id,
                requirement.digest?,
                requirement.artifact_role?,
            ))
        })
        .collect())
}

fn typed_payload_key(retention_ref: &events::RetentionRef) -> TypedPayloadKey {
    (
        retention_ref.artifact_id.clone(),
        retention_ref.content_digest.clone(),
        retention_ref.role,
    )
}

fn validate_historical_retention_manifest_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    pre_projection_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
    artifact_bytes: &store::ArtifactByteAuthorityMap,
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
    let expected = build_retention_manifest_artifact(
        runtime_spec,
        &projection.run_id,
        pre_projection_stream,
        artifact_bytes,
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
    let expected_manifest_evidence_hash = expected.evidence.evidence_hash()?;

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

    let pre_projection = store::ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
        pre_projection_stream,
        artifact_bytes,
    )?;
    if !matches!(
        pre_projection
            .attempt(&retention_node.node_id, &produced.attempt_id)
            .map(|attempt| &attempt.status),
        Some(store::AttemptStatus::Started { .. })
    ) {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection lacks matching started framework attempt in the prefix"
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
        || manifest_ref.evidence_hash != expected_manifest_evidence_hash
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
        || receipt_ref.evidence_hash != produced.evidence_hash
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
        &RetentionProjectionCommitEvidence {
            receipt_artifact_id: &receipt_artifact_id,
            receipt_digest: &receipt_digest,
            receipt_evidence_hash: &produced.evidence_hash,
            manifest_artifact_id: &expected.evidence.artifact_id,
            manifest_evidence_hash: &expected_manifest_evidence_hash,
        },
    )?;
    Ok(())
}

struct RetentionProjectionCommitEvidence<'a> {
    receipt_artifact_id: &'a ArtifactId,
    receipt_digest: &'a ContentDigest,
    receipt_evidence_hash: &'a ContentDigest,
    manifest_artifact_id: &'a ArtifactId,
    manifest_evidence_hash: &'a ContentDigest,
}

fn validate_retention_projection_commit_payload_set(
    commit: &[store::KernelEventEnvelope],
    retention_node_id: &NodeId,
    attempt_id: &AttemptId,
    receipt_cell_id: &CellId,
    evidence: &RetentionProjectionCommitEvidence<'_>,
) -> Result<()> {
    let mut produced = 0_usize;
    let mut completed = 0_usize;
    let mut artifact_referenced = 0_usize;
    let mut manifest_projected = 0_usize;
    let mut retention_refs = 0_usize;

    for event in commit {
        match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == *retention_node_id
                    && payload.attempt_id == *attempt_id
                    && payload.cell_id == *receipt_cell_id
                    && payload.artifact_id == *evidence.receipt_artifact_id
                    && payload.content_digest == *evidence.receipt_digest
                    && payload.evidence_hash == *evidence.receipt_evidence_hash =>
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
                    && payload.artifact_ref.artifact_id == *evidence.receipt_artifact_id
                    && payload.artifact_ref.content_digest == *evidence.receipt_digest
                    && payload.artifact_ref.evidence_hash == *evidence.receipt_evidence_hash
                    && payload.artifact_ref.role == events::ArtifactRole::StateOutput =>
            {
                artifact_referenced += 1;
            }
            events::KernelEventPayload::RetentionManifestProjected(payload)
                if payload.manifest_artifact_id == *evidence.manifest_artifact_id =>
            {
                manifest_projected += 1;
            }
            events::KernelEventPayload::RetentionRefsAppended(payload) => match payload.reason {
                events::RetentionReason::ManifestProjection
                    if payload.refs.iter().all(|reference| {
                        reference.artifact_id == *evidence.manifest_artifact_id
                            && reference.evidence_hash == *evidence.manifest_evidence_hash
                    }) =>
                {
                    retention_refs += 1;
                }
                events::RetentionReason::RuntimeEvidence
                    if payload.refs.iter().all(|reference| {
                        reference.artifact_id == *evidence.receipt_artifact_id
                            && reference.evidence_hash == *evidence.receipt_evidence_hash
                    }) =>
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

    if produced == 1
        && completed == 1
        && artifact_referenced == 1
        && manifest_projected == 1
        && retention_refs == 2
        && commit.len() == 6
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit does not match the sealed framework batch"
                .to_owned(),
        ))
    }
}
