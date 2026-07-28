use super::*;
use mfm_canonical::PlainCanonicalJsonBytes;

type RetentionRefKey = (ArtifactId, ContentDigest);
type TypedPayloadKey = (ArtifactId, ContentDigest, events::ArtifactRole);

/// Minimal retention frontier needed to validate a strict journal suffix.
#[derive(Debug)]
pub(super) struct RetentionHistoryFold {
    refs: BTreeMap<RetentionRefKey, events::RetentionRef>,
    event_schema_ids: BTreeSet<SchemaId>,
    latest_manifest: Option<(u64, ContentDigest)>,
    started_attempts: BTreeSet<(NodeId, AttemptId)>,
    last_sequence: Option<StreamSeq>,
}

impl RetentionHistoryFold {
    pub(super) fn new() -> Self {
        Self {
            refs: BTreeMap::new(),
            event_schema_ids: BTreeSet::new(),
            latest_manifest: None,
            started_attempts: BTreeSet::new(),
            last_sequence: None,
        }
    }

    pub(super) fn apply_suffix(
        &mut self,
        spec: &HistorySpec<'_>,
        history: &JournalHistory<'_>,
        suffix_start: usize,
    ) -> Result<()> {
        for batch in history.suffix_batches(suffix_start)? {
            validate_retention_ref_batch(spec, batch.records())?;
            self.validate_manifest_batch(spec, history, &batch)?;
            self.apply_batch(batch.records())?;
        }
        Ok(())
    }

    fn apply_batch(&mut self, records: &[KernelEventEnvelope]) -> Result<()> {
        for event in records {
            self.last_sequence = Some(event.seq());
            self.event_schema_ids
                .insert(event.event_schema_id().clone());
            match event.payload() {
                events::KernelEventPayload::StateAttemptStarted(started) => {
                    self.started_attempts
                        .insert((started.node_id.clone(), started.attempt_id.clone()));
                }
                events::KernelEventPayload::RunAdmitted(admitted) => {
                    for artifact in std::iter::once(&admitted.spec_artifact)
                        .chain(std::iter::once(&admitted.certificate_artifact))
                        .chain(admitted.config_artifacts.iter())
                    {
                        self.insert_evidence(ArtifactEvidenceRef::from_run_artifact(artifact))?;
                    }
                    for seed in &admitted.seed_cells {
                        self.insert_evidence(ArtifactEvidenceRef {
                            artifact_id: seed.seed_artifact.artifact_id.clone(),
                            digest: seed.seed_artifact.content_digest.clone(),
                            byte_len: seed.seed_artifact.byte_len,
                            media_type: seed.seed_artifact.media_type.clone(),
                            schema_id: Some(seed.seed_artifact.schema_id.clone()),
                            semantic_type_id: seed.seed_artifact.semantic_type_id.clone(),
                            producer_node_id: None,
                            producer_seed_id: Some(seed.seed_id.clone()),
                            artifact_role: seed.seed_artifact.role,
                        })?;
                    }
                }
                events::KernelEventPayload::RetentionRefsAppended(appended) => {
                    for retained in &appended.refs {
                        self.refs.insert(
                            (retained.artifact_id.clone(), retained.evidence_hash.clone()),
                            retained.clone(),
                        );
                    }
                }
                events::KernelEventPayload::RetentionManifestProjected(manifest) => {
                    self.latest_manifest =
                        Some((manifest.manifest_seq, manifest.manifest_digest.clone()));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn insert_evidence(&mut self, evidence: ArtifactEvidenceRef) -> Result<()> {
        let retained = evidence.retention_ref()?;
        self.refs.insert(
            (retained.artifact_id.clone(), retained.evidence_hash.clone()),
            retained,
        );
        Ok(())
    }

    fn validate_manifest_batch(
        &self,
        spec: &HistorySpec<'_>,
        history: &JournalHistory<'_>,
        batch: &JournalBatch<'_>,
    ) -> Result<()> {
        let projections = batch
            .records()
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
            return Err(invalid_history(
                "retention projection batch contains multiple manifest records",
            ));
        }
        let projected = projections[0];
        let retention_node = certified_framework_node(spec, FrameworkRole::Retention)?;
        if projected.spec_hash != *spec.spec_hash() {
            return Err(invalid_history(
                "retention manifest spec hash does not match certified spec",
            ));
        }
        let expected =
            self.build_manifest(spec, projected.run_id.clone(), history.admission_root()?)?;
        if projected.manifest_seq != expected.manifest_seq
            || projected.manifest_digest != expected.evidence.digest
            || projected.previous_manifest_digest != expected.previous_manifest_digest
            || projected.manifest_artifact_id != expected.evidence.artifact_id
        {
            return Err(invalid_history(
                "retention manifest projection does not match its verified prefix frontier",
            ));
        }
        let manifest_evidence_hash = expected.evidence.evidence_hash()?;
        let manifest_object = history
            .object(&expected.evidence.artifact_id, &manifest_evidence_hash)
            .ok_or_else(|| {
                invalid_history("retention manifest object is absent from journal authority")
            })?;
        if manifest_object.bytes.as_slice() != expected.bytes.as_bytes()
            || manifest_object.evidence != expected.evidence
        {
            return Err(invalid_history(
                "retention manifest object does not match generated canonical evidence",
            ));
        }
        let receipt_bytes = retention_manifest_receipt_json(&expected, self.last_sequence)?;
        let receipt_digest = receipt_bytes.content_digest();
        let receipt_artifact_id =
            ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
        let produced = batch
            .records()
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
            return Err(invalid_history(
                "retention manifest batch lacks exactly one framework receipt cell",
            ));
        }
        let produced = produced[0];
        if produced.spec_hash != *spec.spec_hash()
            || produced.cell_id != retention_node.output_cell
            || produced.artifact_id != receipt_artifact_id
            || produced.content_digest != receipt_digest
        {
            return Err(invalid_history(
                "retention manifest receipt cell does not match projected manifest",
            ));
        }
        let receipt_object = history
            .object(&receipt_artifact_id, &produced.evidence_hash)
            .ok_or_else(|| invalid_history("retention receipt object is absent"))?;
        if receipt_object.bytes.as_slice() != receipt_bytes.as_bytes()
            || receipt_object.evidence.digest != receipt_digest
        {
            return Err(invalid_history(
                "retention receipt object does not match generated canonical receipt",
            ));
        }
        if !self
            .started_attempts
            .contains(&(retention_node.node_id.clone(), produced.attempt_id.clone()))
        {
            return Err(invalid_history(
                "retention manifest batch lacks its started framework attempt",
            ));
        }
        validate_manifest_refs(
            spec,
            batch.records(),
            projected,
            produced,
            &expected,
            &manifest_evidence_hash,
            &receipt_artifact_id,
            &receipt_digest,
        )?;
        validate_manifest_payload_set(
            batch.records(),
            &retention_node.node_id,
            &retention_node.output_cell,
            produced,
            &expected,
            &manifest_evidence_hash,
            &receipt_artifact_id,
            &receipt_digest,
        )
    }

    fn build_manifest(
        &self,
        spec: &HistorySpec<'_>,
        run_id: RunId,
        admitted: &events::RunAdmitted,
    ) -> Result<RetentionManifestArtifact> {
        if admitted.run_id != run_id || admitted.spec_hash != *spec.spec_hash() {
            return Err(invalid_history(
                "retention manifest admission root does not match certified run",
            ));
        }
        let (manifest_seq, previous_manifest_digest) = self
            .latest_manifest
            .as_ref()
            .map(|(seq, digest)| {
                seq.checked_add(1)
                    .map(|next| (next, Some(digest.clone())))
                    .ok_or_else(|| invalid_history("retention manifest sequence overflow"))
            })
            .transpose()?
            .unwrap_or((1, None));
        let bytes = retention_manifest_json(
            spec,
            &run_id,
            admitted,
            manifest_seq,
            previous_manifest_digest.as_ref(),
            self.refs.values().collect::<Vec<_>>().as_slice(),
            &self.event_schema_ids,
        )?;
        let digest = bytes.content_digest();
        let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
        Ok(RetentionManifestArtifact {
            evidence: ArtifactEvidenceRef {
                artifact_id,
                digest,
                byte_len: bytes.as_bytes().len() as u64,
                media_type: spec::MediaType::new(
                    "application/vnd.mfm.retention-manifest+json;version=1",
                )
                .map_err(|error| invalid_history(error.to_string()))?,
                schema_id: None,
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::RetentionManifest,
            },
            bytes,
            manifest_seq,
            previous_manifest_digest,
        })
    }
}

struct RetentionManifestArtifact {
    bytes: PlainCanonicalJsonBytes,
    evidence: ArtifactEvidenceRef,
    manifest_seq: u64,
    previous_manifest_digest: Option<ContentDigest>,
}

fn validate_retention_ref_batch(
    spec: &HistorySpec<'_>,
    batch: &[KernelEventEnvelope],
) -> Result<()> {
    let appended = batch
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload) => Some((event, payload)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if appended.is_empty() {
        return Ok(());
    }
    let has_manifest = batch.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::RetentionManifestProjected(_)
        )
    });
    let artifact_refs = same_batch_artifact_reference_keys(batch)?;
    let typed_refs = same_batch_typed_artifact_keys(batch)?;
    for (event, payload) in appended {
        if event.run_id() != &payload.run_id || payload.spec_hash != *spec.spec_hash() {
            return Err(invalid_history(
                "retention refs identity does not match certified run",
            ));
        }
        if payload.refs.is_empty() {
            return Err(invalid_history("retention refs append cannot be empty"));
        }
        match payload.reason {
            events::RetentionReason::RunAdmitted => {
                return Err(invalid_history(
                    "RunAdmitted retention refs must be projection-derived",
                ));
            }
            events::RetentionReason::ManifestProjection if !has_manifest => {
                return Err(invalid_history(
                    "manifest retention refs lack same-batch manifest projection",
                ));
            }
            events::RetentionReason::ManifestProjection => {}
            events::RetentionReason::RuntimeEvidence => {
                validate_same_batch_ref_evidence(payload, &artifact_refs, &typed_refs, true)?
            }
            events::RetentionReason::PublicOutput => {
                validate_same_batch_ref_evidence(payload, &artifact_refs, &typed_refs, false)?;
                validate_public_output_refs(spec, batch, payload)?;
            }
        }
    }
    Ok(())
}

fn validate_same_batch_ref_evidence(
    payload: &events::RetentionRefsAppended,
    artifact_refs: &BTreeSet<RetentionRefKey>,
    typed_refs: &BTreeSet<TypedPayloadKey>,
    allow_fact_query_returns: bool,
) -> Result<()> {
    let has_fact_query = payload.refs.iter().any(|retained| {
        retained.role == events::ArtifactRole::FactQueryEvidence
            && artifact_refs.contains(&retention_ref_key(retained))
            && typed_refs.contains(&typed_payload_key(retained))
    });
    for retained in &payload.refs {
        if allow_fact_query_returns
            && has_fact_query
            && matches!(
                retained.role,
                events::ArtifactRole::FactDescriptor | events::ArtifactRole::FactResponse
            )
        {
            continue;
        }
        if role_requires_artifact_reference(retained.role)
            && !artifact_refs.contains(&retention_ref_key(retained))
        {
            return Err(invalid_history(format!(
                "retention ref {} lacks same-batch ArtifactReferenced evidence",
                retained.artifact_id
            )));
        }
        if !typed_refs.contains(&typed_payload_key(retained)) {
            return Err(invalid_history(format!(
                "retention ref {} lacks same-batch typed payload evidence",
                retained.artifact_id
            )));
        }
    }
    Ok(())
}

fn role_requires_artifact_reference(role: events::ArtifactRole) -> bool {
    matches!(
        role.contract().staging,
        events::ArtifactStagingClass::AttemptStateOutput
            | events::ArtifactStagingClass::AttemptFactResponse
            | events::ArtifactStagingClass::AttemptExternalReadEvidence
            | events::ArtifactStagingClass::AttemptFactQueryEvidence
            | events::ArtifactStagingClass::AttemptPublicOutput
            | events::ArtifactStagingClass::AttemptRedactedDiagnostic
    )
}

fn validate_public_output_refs(
    spec: &HistorySpec<'_>,
    batch: &[KernelEventEnvelope],
    appended: &events::RetentionRefsAppended,
) -> Result<()> {
    let outputs = batch
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::PublicOutputProduced(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if outputs.len() != 1 {
        return Err(invalid_history(
            "public-output retention refs require exactly one same-batch output record",
        ));
    }
    let output = outputs[0];
    let node = spec.node(&output.node_id).ok_or_else(|| {
        invalid_history(format!(
            "public-output retention references uncertified node {}",
            output.node_id
        ))
    })?;
    if !matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    ) {
        return Err(invalid_history(format!(
            "public-output retention was appended by non-render node {}",
            output.node_id
        )));
    }
    let allowed = public_output_ref_keys(batch, output)?;
    for retained in &appended.refs {
        if !allowed.contains(&retention_ref_key(retained)) {
            return Err(invalid_history(format!(
                "public-output retention ref {} is not sealed to output evidence",
                retained.artifact_id
            )));
        }
    }
    Ok(())
}

fn public_output_ref_keys(
    batch: &[KernelEventEnvelope],
    output: &events::PublicOutputProduced,
) -> Result<BTreeSet<RetentionRefKey>> {
    let mut allowed = BTreeSet::new();
    for event in batch {
        if let events::KernelEventPayload::CellProduced(produced) = event.payload() {
            if produced.node_id == output.node_id
                && produced.attempt_id == output.attempt_id
                && produced.cell_id == output.receipt_cell_id
            {
                if let Some(key) = batch
                    .iter()
                    .find_map(|candidate| match candidate.payload() {
                        events::KernelEventPayload::ArtifactReferenced(reference)
                            if reference.node_id.as_ref() == Some(&produced.node_id)
                                && reference.attempt_id.as_ref() == Some(&produced.attempt_id)
                                && reference.artifact_ref.artifact_id == produced.artifact_id
                                && reference.artifact_ref.content_digest
                                    == produced.content_digest
                                && reference.artifact_ref.evidence_hash
                                    == produced.evidence_hash
                                && reference.artifact_ref.role
                                    == events::ArtifactRole::StateOutput =>
                        {
                            Some(event_artifact_ref_key(
                                &reference.artifact_ref,
                                reference.node_id.clone(),
                            ))
                        }
                        _ => None,
                    })
                    .transpose()?
                {
                    allowed.insert(key);
                }
            }
        }
    }
    if let Some(artifact_id) = &output.rendered_artifact_id {
        if let Some(key) = batch
            .iter()
            .find_map(|candidate| match candidate.payload() {
                events::KernelEventPayload::ArtifactReferenced(reference)
                    if reference.node_id.as_ref() == Some(&output.node_id)
                        && reference.attempt_id.as_ref() == Some(&output.attempt_id)
                        && reference.artifact_ref.artifact_id == *artifact_id
                        && reference.artifact_ref.content_digest == output.rendered_digest
                        && output.rendered_artifact_evidence_hash.as_ref()
                            == Some(&reference.artifact_ref.evidence_hash)
                        && reference.artifact_ref.role == events::ArtifactRole::PublicOutput =>
                {
                    Some(event_artifact_ref_key(
                        &reference.artifact_ref,
                        reference.node_id.clone(),
                    ))
                }
                _ => None,
            })
            .transpose()?
        {
            allowed.insert(key);
        }
    }
    if allowed.is_empty() {
        return Err(invalid_history(
            "public-output retention lacks same-batch receipt or render evidence",
        ));
    }
    Ok(allowed)
}

fn retention_ref_key(retained: &events::RetentionRef) -> RetentionRefKey {
    (retained.artifact_id.clone(), retained.evidence_hash.clone())
}

fn typed_payload_key(retained: &events::RetentionRef) -> TypedPayloadKey {
    (
        retained.artifact_id.clone(),
        retained.content_digest.clone(),
        retained.role,
    )
}

fn event_artifact_ref_key(
    artifact: &events::ArtifactEvidenceRef,
    producer_node_id: Option<NodeId>,
) -> Result<RetentionRefKey> {
    let evidence = ArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        digest: artifact.content_digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
        schema_id: Some(artifact.schema_id.clone()),
        semantic_type_id: artifact.semantic_type_id.clone(),
        producer_node_id,
        producer_seed_id: None,
        artifact_role: artifact.role,
    };
    Ok((evidence.artifact_id.clone(), evidence.evidence_hash()?))
}

fn same_batch_artifact_reference_keys(
    batch: &[KernelEventEnvelope],
) -> Result<BTreeSet<RetentionRefKey>> {
    batch
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                Some((&payload.artifact_ref, payload.node_id.clone()))
            }
            _ => None,
        })
        .map(|(artifact, producer)| event_artifact_ref_key(artifact, producer))
        .collect()
}

fn same_batch_typed_artifact_keys(
    batch: &[KernelEventEnvelope],
) -> Result<BTreeSet<TypedPayloadKey>> {
    Ok(batch
        .iter()
        .flat_map(|event| event_artifact_requirements(event.payload()))
        .filter(|requirement| {
            requirement.source.is_same_commit_payload_evidence()
                || (requirement.source == EventArtifactReferenceSource::ArtifactReferenced
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

#[allow(clippy::too_many_arguments)]
fn validate_manifest_refs(
    spec: &HistorySpec<'_>,
    batch: &[KernelEventEnvelope],
    projected: &events::RetentionManifestProjected,
    produced: &events::CellProduced,
    expected: &RetentionManifestArtifact,
    manifest_evidence_hash: &ContentDigest,
    receipt_artifact_id: &ArtifactId,
    receipt_digest: &ContentDigest,
) -> Result<()> {
    let completed = batch
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptCompleted(payload)
                    if payload.node_id == produced.node_id
                        && payload.attempt_id == produced.attempt_id
                        && payload.output_cell_id == produced.cell_id
            )
        })
        .count();
    if completed != 1 {
        return Err(invalid_history(
            "retention manifest batch lacks matching StateAttemptCompleted",
        ));
    }
    let refs = batch
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
        return Err(invalid_history(
            "retention manifest batch must contain exactly one manifest ref append",
        ));
    }
    let manifest_refs = manifest_refs[0];
    if manifest_refs.run_id != projected.run_id
        || manifest_refs.spec_hash != *spec.spec_hash()
        || manifest_refs.refs.len() != 1
    {
        return Err(invalid_history(
            "manifest retention refs are not sealed to the projection",
        ));
    }
    let retained_manifest = &manifest_refs.refs[0];
    if retained_manifest.artifact_id != expected.evidence.artifact_id
        || retained_manifest.content_digest != expected.evidence.digest
        || retained_manifest.evidence_hash != *manifest_evidence_hash
        || retained_manifest.role != events::ArtifactRole::RetentionManifest
    {
        return Err(invalid_history(
            "manifest retention ref does not match generated manifest evidence",
        ));
    }
    let receipt_refs = refs
        .iter()
        .copied()
        .filter(|payload| payload.reason == events::RetentionReason::RuntimeEvidence)
        .collect::<Vec<_>>();
    if receipt_refs.len() != 1 {
        return Err(invalid_history(
            "retention manifest batch must retain its framework receipt",
        ));
    }
    let receipt_refs = receipt_refs[0];
    if receipt_refs.run_id != projected.run_id
        || receipt_refs.spec_hash != *spec.spec_hash()
        || receipt_refs.refs.len() != 1
    {
        return Err(invalid_history(
            "manifest receipt refs are not sealed to runtime evidence",
        ));
    }
    let receipt = &receipt_refs.refs[0];
    if receipt.artifact_id != *receipt_artifact_id
        || receipt.content_digest != *receipt_digest
        || receipt.evidence_hash != produced.evidence_hash
        || receipt.role != events::ArtifactRole::StateOutput
    {
        return Err(invalid_history(
            "manifest receipt ref does not match framework receipt evidence",
        ));
    }
    if refs.len() != 2 {
        return Err(invalid_history(
            "retention manifest batch contains unsupported retention refs",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_manifest_payload_set(
    batch: &[KernelEventEnvelope],
    node_id: &NodeId,
    output_cell: &CellId,
    produced: &events::CellProduced,
    expected: &RetentionManifestArtifact,
    manifest_evidence_hash: &ContentDigest,
    receipt_artifact_id: &ArtifactId,
    receipt_digest: &ContentDigest,
) -> Result<()> {
    let mut produced_count = 0;
    let mut completed = 0;
    let mut referenced = 0;
    let mut projected = 0;
    let mut refs = 0;
    for event in batch {
        match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == *node_id
                    && payload.attempt_id == produced.attempt_id
                    && payload.cell_id == *output_cell
                    && payload.artifact_id == *receipt_artifact_id
                    && payload.content_digest == *receipt_digest
                    && payload.evidence_hash == produced.evidence_hash =>
            {
                produced_count += 1;
            }
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.node_id == *node_id
                    && payload.attempt_id == produced.attempt_id
                    && payload.output_cell_id == *output_cell =>
            {
                completed += 1;
            }
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(node_id)
                    && payload.attempt_id.as_ref() == Some(&produced.attempt_id)
                    && payload.artifact_ref.artifact_id == *receipt_artifact_id
                    && payload.artifact_ref.content_digest == *receipt_digest
                    && payload.artifact_ref.evidence_hash == produced.evidence_hash
                    && payload.artifact_ref.role == events::ArtifactRole::StateOutput =>
            {
                referenced += 1;
            }
            events::KernelEventPayload::RetentionManifestProjected(payload)
                if payload.manifest_artifact_id == expected.evidence.artifact_id =>
            {
                projected += 1;
            }
            events::KernelEventPayload::RetentionRefsAppended(payload) => match payload.reason {
                events::RetentionReason::ManifestProjection
                    if payload.refs.iter().all(|reference| {
                        reference.artifact_id == expected.evidence.artifact_id
                            && reference.evidence_hash == *manifest_evidence_hash
                    }) =>
                {
                    refs += 1;
                }
                events::RetentionReason::RuntimeEvidence
                    if payload.refs.iter().all(|reference| {
                        reference.artifact_id == *receipt_artifact_id
                            && reference.evidence_hash == produced.evidence_hash
                    }) =>
                {
                    refs += 1;
                }
                _ => {
                    return Err(invalid_history(
                        "retention manifest batch contains unsupported payload",
                    ));
                }
            },
            _ => {
                return Err(invalid_history(
                    "retention manifest batch contains unsupported payload",
                ));
            }
        }
    }
    if produced_count == 1
        && completed == 1
        && referenced == 1
        && projected == 1
        && refs == 2
        && batch.len() == 6
    {
        Ok(())
    } else {
        Err(invalid_history(
            "retention manifest batch does not match the sealed framework shape",
        ))
    }
}

fn retention_manifest_receipt_json(
    manifest: &RetentionManifestArtifact,
    prefix_seq: Option<StreamSeq>,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "manifest_artifact_id": manifest.evidence.artifact_id.as_str(),
        "manifest_digest": manifest.evidence.digest.as_str(),
        "manifest_seq": manifest.manifest_seq,
        "pre_projection_stream_seq": prefix_seq.map(StreamSeq::as_u64),
        "previous_manifest_digest": manifest.previous_manifest_digest.as_ref().map(ContentDigest::as_str),
    }))
}

fn retention_manifest_json(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    admitted: &events::RunAdmitted,
    manifest_seq: u64,
    previous_manifest_digest: Option<&ContentDigest>,
    retained_refs: &[&events::RetentionRef],
    event_schema_ids: &BTreeSet<SchemaId>,
) -> Result<PlainCanonicalJsonBytes> {
    let spec_canonical = spec
        .spec()
        .canonical_json()
        .map_err(|error| invalid_history(error.to_string()))?;
    let spec_digest = spec_canonical.content_digest();
    let certificate = spec
        .certified()
        .certificate()
        .canonical_json()
        .map_err(|error| invalid_history(error.to_string()))?;
    let retained_by_role = retained_refs_by_role(retained_refs);
    canonical_json(serde_json::json!({
        "adapter_executables": admitted.adapter_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
        "canonicalizer_identity": admitted.canonicalizer_identity.as_str(),
        "certificate_artifact": {
            "artifact_id": admitted.certificate_artifact.artifact_id.as_str(),
            "byte_len": certificate.as_bytes().len(),
            "content_digest": admitted.certificate_artifact.content_digest.as_str(),
            "media_type": admitted.certificate_artifact.media_type.as_str(),
        },
        "config_artifacts": spec.spec().config_refs.iter().map(config_artifact_json).collect::<Vec<_>>(),
        "descriptor_digests": spec.spec().descriptor_identities.iter().map(descriptor_digest_json).collect::<Vec<_>>(),
        "descriptor_identities": spec.spec().descriptor_identities.iter().map(descriptor_identity_json).collect::<Vec<_>>(),
        "entry_point": entry_point_json(&admitted.entry_point),
        "event_schema_ids": event_schema_ids.iter().map(SchemaId::as_str).collect::<Vec<_>>(),
        "manifest_seq": manifest_seq,
        "previous_manifest_digest": previous_manifest_digest.map(ContentDigest::as_str),
        "public_output_artifacts": retained_by_role.public_output_artifacts,
        "receipt_artifacts": retained_by_role.receipt_artifacts,
        "confirmation_artifacts": retained_by_role.confirmation_artifacts,
        "retained_refs": retained_refs.iter().map(|retained| retention_ref_json(retained)).collect::<Vec<_>>(),
        "run_id": run_id.as_str(),
        "runner_executables": admitted.runner_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
        "spec_artifact": {
            "artifact_id": admitted.spec_artifact.artifact_id.as_str(),
            "byte_len": spec_canonical.as_bytes().len(),
            "content_digest": spec_digest.as_str(),
            "media_type": admitted.spec_artifact.media_type.as_str(),
        },
        "spec_hash": spec.spec_hash().as_str(),
        "value_artifacts": retained_by_role.value_artifacts,
    }))
}

fn executable_identity_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
    serde_json::json!({
        "binary_digest": identity.binary_digest.as_str(),
        "factory_id": identity.factory_id.as_str(),
    })
}

fn entry_point_json(evidence: &events::EntryPointLaunchEvidence) -> serde_json::Value {
    serde_json::json!({
        "configured_targets": evidence.configured_targets.iter().map(|source| {
            serde_json::json!({
                "digest": source.digest.as_str(),
                "target": source.target.as_str(),
                "schema_id": source.schema_id.as_str(),
            })
        }).collect::<Vec<_>>(),
        "entry_point_id": evidence.entry_point_id.as_str(),
    })
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
        "digest": ContentDigest::from_digest(
            descriptor_id.algorithm(),
            *descriptor_id.digest(),
        ).as_str(),
    })
}

fn retention_ref_json(retained: &events::RetentionRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": retained.artifact_id.as_str(),
        "content_digest": retained.content_digest.as_str(),
        "evidence_hash": retained.evidence_hash.as_str(),
        "role": retained.role.as_str(),
    })
}

struct RetainedRefsByRole {
    value_artifacts: Vec<String>,
    receipt_artifacts: Vec<String>,
    confirmation_artifacts: Vec<String>,
    public_output_artifacts: Vec<String>,
}

fn retained_refs_by_role(retained_refs: &[&events::RetentionRef]) -> RetainedRefsByRole {
    let mut grouped = RetainedRefsByRole {
        value_artifacts: Vec::new(),
        receipt_artifacts: Vec::new(),
        confirmation_artifacts: Vec::new(),
        public_output_artifacts: Vec::new(),
    };
    for retained in retained_refs {
        match retained.role.contract().retention {
            events::ArtifactRetentionClass::ValueArtifacts => {
                grouped
                    .value_artifacts
                    .push(retained.artifact_id.as_str().to_owned());
            }
            events::ArtifactRetentionClass::ReceiptArtifacts => {
                grouped
                    .receipt_artifacts
                    .push(retained.artifact_id.as_str().to_owned());
            }
            events::ArtifactRetentionClass::ConfirmationArtifacts => {
                grouped
                    .confirmation_artifacts
                    .push(retained.artifact_id.as_str().to_owned());
            }
            events::ArtifactRetentionClass::PublicOutputArtifacts => {
                grouped
                    .public_output_artifacts
                    .push(retained.artifact_id.as_str().to_owned());
            }
            events::ArtifactRetentionClass::FrameworkIgnored => {}
        }
    }
    grouped
}

#[derive(Clone, Copy)]
pub(super) enum FrameworkRole {
    Retention,
    Complete,
    Resolve,
}

pub(super) fn certified_framework_node<'a>(
    spec: &HistorySpec<'a>,
    role: FrameworkRole,
) -> Result<&'a spec::NodeSpec> {
    let role_id = match role {
        FrameworkRole::Retention => spec.certified().framework_lifecycle().retention().node_id(),
        FrameworkRole::Complete => spec.certified().framework_lifecycle().complete().node_id(),
        FrameworkRole::Resolve => spec.certified().framework_lifecycle().resolve().node_id(),
    };
    spec.node(role_id).ok_or_else(|| {
        invalid_history(format!(
            "certified framework lifecycle references missing node {}",
            role_id
        ))
    })
}
