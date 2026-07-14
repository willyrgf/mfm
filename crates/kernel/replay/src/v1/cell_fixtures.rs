use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) enum CellReplayTerminal {
    Produced,
    Skipped,
}

pub(super) fn cell_replay_authority(
    terminal: CellReplayTerminal,
    event_context: spec::CellContextSpec,
) -> ReplayReadAuthority {
    let descriptor = replay_stream_fact_descriptor();
    let descriptor_bytes =
        mfm_facts::canonical_fact_descriptor_bytes(&descriptor).expect("descriptor bytes");
    let descriptor_digest = mfm_facts::fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let descriptor_artifact = stored_artifact_ref(
        descriptor_digest.clone(),
        ArtifactRole::FactDescriptor,
        Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
        None,
        descriptor_bytes.as_bytes().len() as u64,
    );
    let descriptor_artifact_key =
        replay_artifact_authority_key(&descriptor_artifact).expect("descriptor artifact key");
    let descriptor_run_ref = run_artifact_ref_from_store_artifact_for_test(&descriptor_artifact);

    let certified_spec = hashed_cell_replay_spec();
    let run_admitted = fact_run_admitted_for_stream(&certified_spec, descriptor_run_ref);
    let run_id = run_admitted.run_id.clone();
    let node = certified_spec
        .spec
        .nodes
        .iter()
        .find(|node| node.node_id == fact_node_id())
        .expect("cell node");
    let cell = certified_spec
        .spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == node.output_cell)
        .expect("output cell");
    let attempt_id = fact_attempt_id();
    let started = events::StateAttemptStarted {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: node.node_id.clone(),
        attempt_id: attempt_id.clone(),
        attempt_no: 1,
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
    };
    let mut artifact_evidence = vec![
        stored_artifact_from_run_ref(&stream_run_admitted_spec_artifact_for_hash(
            &certified_spec.spec_hash,
        )),
        stored_artifact_from_run_ref(&stream_run_admitted_certificate_artifact()),
        replay_stream_config_artifact(&certified_spec),
        descriptor_artifact,
    ];
    let terminal_payload = match terminal {
        CellReplayTerminal::Produced => {
            let output_digest = content_digest(0xe1);
            let output_artifact = state_output_artifact_ref(cell, &output_digest);
            let evidence_hash = output_artifact
                .evidence_hash()
                .expect("state output evidence hash");
            artifact_evidence.push(output_artifact);
            KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: certified_spec.spec_hash.clone(),
                node_id: node.node_id.clone(),
                cell_id: cell.cell_id.clone(),
                scope_id: cell.scope_id.clone(),
                attempt_id: attempt_id.clone(),
                semantic_type_id: cell.semantic_type_id.clone(),
                schema_id: cell.schema_id.clone(),
                value_lineage: cell.value_lineage.clone(),
                context: event_context,
                artifact_id: ArtifactId::from_digest(
                    output_digest.algorithm(),
                    *output_digest.digest(),
                ),
                content_digest: output_digest,
                evidence_hash,
                producer_state_kind: Some(node.state_kind.clone()),
                producer_state_version: Some(node.state_version.clone()),
            })
        }
        CellReplayTerminal::Skipped => KernelEventPayload::CellSkipped(events::CellSkipped {
            spec_hash: certified_spec.spec_hash.clone(),
            node_id: node.node_id.clone(),
            cell_id: cell.cell_id.clone(),
            scope_id: cell.scope_id.clone(),
            attempt_id: attempt_id.clone(),
            semantic_type_id: cell.semantic_type_id.clone(),
            schema_id: cell.schema_id.clone(),
            value_lineage: cell.value_lineage.clone(),
            context: event_context,
            skip_reason: events::SkipReason {
                code: events::ErrorCode::new("mfm.replay.test.skipped").expect("skip code"),
                safe_message: "skipped for replay context test".to_owned(),
            },
        }),
    };
    let completed = KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: node.node_id.clone(),
        attempt_id: attempt_id.clone(),
        output_cell_id: cell.cell_id.clone(),
    });
    let terminal_commit_key = store::CommitKey::new("replay-test:3").expect("terminal commit key");
    ReplayReadAuthority {
        certified_spec: certified_spec.clone(),
        stream: vec![
            persisted_envelope(
                &run_id,
                1,
                KernelEventPayload::RunAdmitted(Box::new(run_admitted)),
            ),
            persisted_envelope(&run_id, 2, KernelEventPayload::StateAttemptStarted(started)),
            persisted_envelope_with_ordinal(&run_id, 3, 0, terminal_commit_key.clone(), completed),
            persisted_envelope_with_ordinal(&run_id, 3, 1, terminal_commit_key, terminal_payload),
        ],
        canonicalizer_identity: certified_spec
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        artifact_evidence,
        artifact_bytes: BTreeMap::from([(descriptor_artifact_key, descriptor_bytes.to_vec())]),
        additional_artifact_evidence: Vec::new(),
        source_fact_events: Vec::new(),
    }
}

pub(super) fn hashed_cell_replay_spec() -> HashedSpecEnvelope {
    let mut envelope = fact_replay_spec();
    let node = envelope
        .spec
        .nodes
        .iter()
        .find(|node| node.node_id == fact_node_id())
        .expect("cell node")
        .clone();
    let lineage = spec::ValueLineageRef {
        lineage_digest: content_digest(0xe0),
    };
    envelope.spec.cells.push(spec::CellSpec {
        cell_id: node.output_cell.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        scope_id: node.scope_id.clone(),
        semantic_type_id: semantic_type_id(0xd6),
        schema_id: schema_id("mfm.replay.test.output", 0xd5),
        value_lineage: lineage.clone(),
        terminal_policy: spec::CellTerminalPolicy::MaybeSkipped,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    envelope.spec.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage,
        scope_id: node.scope_id.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        input_cells: Vec::new(),
        config_ref_digest: None,
        planning_lineage: node.planning_lineage,
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("hashed cell replay spec")
}

pub(super) fn state_output_artifact_ref(
    cell: &spec::CellSpec,
    digest: &ContentDigest,
) -> StoredArtifactEvidenceRef {
    StoredArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest: digest.clone(),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: Some(cell.schema_id.clone()),
        semantic_type_id: Some(cell.semantic_type_id.clone()),
        producer_node_id: match &cell.producer {
            spec::CellProducer::Node(node_id) => Some(node_id.clone()),
            spec::CellProducer::Seed(_) => None,
        },
        producer_seed_id: None,
        artifact_role: ArtifactRole::StateOutput,
    }
}

pub(super) fn mismatched_cell_context() -> spec::CellContextSpec {
    spec::CellContextSpec::Bound {
        context_ref: mfm_ids::ContextRef::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(0xe2),
        ),
        resource_kind: mfm_ids::ContextResourceKind::new("mfm.replay.test.resource")
            .expect("resource kind"),
        stage: mfm_ids::ContextStage::new("wrong_stage").expect("context stage"),
        producer: Box::new(spec::ContextProducerSpec {
            producer_descriptor_ids: vec![descriptor_id(0xe3)],
            seed_producers_allowed: false,
        }),
    }
}
