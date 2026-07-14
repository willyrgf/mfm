use super::*;

pub(super) fn replay_fact_stream_fixture() -> ReplayFactStreamFixture {
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

    let (response_artifact, response_bytes) = replay_stream_fact_response_artifact(&descriptor, 42);
    let response_artifact_key =
        replay_artifact_authority_key(&response_artifact).expect("response artifact key");
    let certified_spec = hashed_fact_replay_spec();
    let run_admitted = fact_run_admitted_for_stream(&certified_spec, descriptor_run_ref);
    let run_id = run_admitted.run_id.clone();
    let attempt_id = fact_attempt_id();
    let node = certified_spec
        .spec
        .nodes
        .iter()
        .find(|node| node.node_id == fact_node_id())
        .expect("fact node");
    let started = events::StateAttemptStarted {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: node.node_id.clone(),
        attempt_id: attempt_id.clone(),
        attempt_no: 1,
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
    };
    let fact = events::FactRecorded {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: fact_node_id(),
        attempt_id,
        claim: replay_stream_fact_claim(&descriptor, &response_artifact),
    };
    let stream = vec![
        persisted_envelope(
            &run_id,
            1,
            KernelEventPayload::RunAdmitted(Box::new(run_admitted)),
        ),
        persisted_envelope(&run_id, 2, KernelEventPayload::StateAttemptStarted(started)),
        persisted_envelope(&run_id, 3, KernelEventPayload::FactRecorded(fact)),
    ];
    let claim_id = mfm_facts::derive_fact_claim_id(run_id, 3, 0).expect("fact claim id");
    let response_artifact_id = response_artifact.artifact_id.clone();
    let config_artifact = replay_stream_config_artifact(&certified_spec);
    ReplayFactStreamFixture {
        certified_spec,
        stream,
        artifact_evidence: vec![
            stored_artifact_from_run_ref(&stream_run_admitted_spec_artifact()),
            stored_artifact_from_run_ref(&stream_run_admitted_certificate_artifact()),
            config_artifact,
            descriptor_artifact,
            response_artifact,
        ],
        artifact_bytes: BTreeMap::from([
            (descriptor_artifact_key, descriptor_bytes.to_vec()),
            (response_artifact_key, response_bytes),
        ]),
        run_id: claim_id.source_run_id().clone(),
        claim_id,
        response_artifact_id,
    }
}

pub(super) struct ReplayFactQueryEvidenceArtifact {
    pub(super) artifact: StoredArtifactEvidenceRef,
    pub(super) bytes: Vec<u8>,
    pub(super) event: events::ArtifactReferenced,
}

pub(super) fn fact_query_evidence_artifact(
    fixture: &ReplayFactStreamFixture,
    mutate_ref: impl FnOnce(mfm_facts::InternalFactRef) -> mfm_facts::InternalFactRef,
) -> ReplayFactQueryEvidenceArtifact {
    let plan = replay_fact_query_plan();
    let returned_ref = mutate_ref(internal_fact_ref_for_fixture(fixture));
    let rows = [mfm_facts::FactQueryResultRow::new(
        returned_ref,
        vec![mfm_facts::FactFieldValue::new(
            mfm_facts::FactFieldId::new("result.amount").expect("field"),
            mfm_facts::FactFieldValueType::UnsignedInteger,
            mfm_facts::FactCanonicalScalar::UnsignedInteger(42),
        )
        .expect("field summary")],
    )];
    let frontier = mfm_facts::StoreReadFrontier::new(
        mfm_facts::StoreScopeRef::new("default").expect("store scope"),
        mfm_facts::FactQueryScope::new(
            mfm_facts::FactAudience::Platform,
            mfm_facts::FactVisibilityScope::Default,
        ),
        mfm_facts::DescriptorCatalogWatermark::new(1),
        mfm_facts::StoreCommitOrder::new(3),
    );
    let receipt = mfm_facts::FactQueryReceipt::from_rows(
        frontier,
        mfm_facts::StoreReadFrontierType::Snapshot,
        &rows,
        true,
        None,
    )
    .expect("receipt");
    let selected_summaries_digest = mfm_facts::selected_returned_field_summaries_digest(
        receipt.returned_field_summaries().expect("summaries"),
        &[0],
    )
    .expect("selected summaries digest");
    let selection = mfm_facts::FactSelectionEvidence::new(
        content_digest(0x45),
        vec![0],
        Some(selected_summaries_digest),
    )
    .expect("selection");
    let evidence = mfm_facts::FactQueryEvidence::new(plan, receipt, selection);
    let bytes =
        mfm_facts::canonical_fact_query_evidence_bytes(&evidence).expect("query evidence bytes");
    let artifact = stored_artifact_ref(
        bytes.content_digest(),
        ArtifactRole::FactQueryEvidence,
        Some(mfm_facts::fact_query_evidence_schema_id().expect("query evidence schema")),
        Some(fact_node_id()),
        bytes.as_bytes().len() as u64,
    );
    let event = events::ArtifactReferenced {
        spec_hash: fixture.certified_spec.spec_hash.clone(),
        node_id: Some(fact_node_id()),
        attempt_id: Some(fact_attempt_id()),
        artifact_ref: events::ArtifactEvidenceRef {
            artifact_id: artifact.artifact_id.clone(),
            role: ArtifactRole::FactQueryEvidence,
            schema_id: artifact.schema_id.clone().expect("schema id"),
            semantic_type_id: None,
            content_digest: artifact.digest.clone(),
            evidence_hash: artifact.evidence_hash().expect("query evidence hash"),
            byte_len: artifact.byte_len,
            media_type: artifact.media_type.clone(),
        },
    };
    ReplayFactQueryEvidenceArtifact {
        artifact,
        bytes: bytes.to_vec(),
        event,
    }
}

pub(super) fn append_fact_query_evidence(
    authority: &mut ReplayReadAuthority,
    run_id: &RunId,
    seq: u64,
    query: &ReplayFactQueryEvidenceArtifact,
) {
    authority.stream.push(persisted_envelope(
        run_id,
        seq,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority.artifact_bytes.insert(
        replay_artifact_authority_key(&query.artifact).expect("query artifact key"),
        query.bytes.clone(),
    );
}

pub(super) fn replay_fact_query_plan() -> mfm_facts::CanonicalFactQueryPlan {
    let input = mfm_facts::FactQueryInput::new(
        mfm_facts::StoreScopeRef::new("default").expect("store scope"),
        mfm_facts::FactQueryScope::new(
            mfm_facts::FactAudience::Platform,
            mfm_facts::FactVisibilityScope::Default,
        ),
        mfm_facts::ScopeDecisionEvidence::new(content_digest(0x46)),
        vec![mfm_facts::FactQueryPredicate::new(
            mfm_facts::FactFieldId::new("subject.account").expect("field"),
            mfm_facts::FactQueryOperator::Equal,
            mfm_facts::FactCanonicalScalar::string("same-subject"),
        )],
        vec![mfm_facts::FactFieldId::new("result.amount").expect("field")],
        mfm_facts::FactOrderingName::new("result.amount.desc").expect("ordering"),
        Some(1),
    )
    .expect("query input");
    mfm_facts::compile_fact_query_plan(&replay_stream_fact_descriptor(), input).expect("query plan")
}

pub(super) fn internal_fact_ref_for_fixture(
    fixture: &ReplayFactStreamFixture,
) -> mfm_facts::InternalFactRef {
    let event = fixture.stream.get(2).expect("fact event");
    let KernelEventPayload::FactRecorded(fact) = event.payload() else {
        panic!("expected fact event");
    };
    mfm_facts::InternalFactRef::from_claim(
        fixture.claim_id.clone(),
        event.event_id().clone(),
        "2026-07-01T00:00:00Z".to_owned(),
        fact_node_id(),
        &fact.claim,
    )
    .expect("internal fact ref")
    .expect("indexed fact ref")
}

pub(super) fn internal_fact_ref_parts_from_ref(
    fact_ref: &mfm_facts::InternalFactRef,
) -> mfm_facts::InternalFactRefParts {
    mfm_facts::InternalFactRefParts {
        fact_claim_id: fact_ref.fact_claim_id().clone(),
        source_event_id: fact_ref.source_event_id().clone(),
        recorded_at: fact_ref.recorded_at().to_owned(),
        producer_node_id: fact_ref.producer_node_id().clone(),
        observed_at: fact_ref.observed_at().map(ToOwned::to_owned),
        visibility: fact_ref.visibility().clone(),
        fact_kind: fact_ref.fact_kind().clone(),
        fact_descriptor_hash: fact_ref.fact_descriptor_hash().clone(),
        subject: mfm_facts::FactSubjectRef::new(
            fact_ref.fact_subject_namespace_hash().clone(),
            fact_ref.fact_key().clone(),
            fact_ref.subject_material_hash().clone(),
        ),
        request: match (fact_ref.request_schema_id(), fact_ref.request_hash()) {
            (Some(schema_id), Some(hash)) => Some(mfm_facts::FactRequestEvidence::new(
                schema_id.clone(),
                hash.clone(),
            )),
            (None, None) => None,
            _ => unreachable!("internal fact refs cannot carry partial request evidence"),
        },
        response: mfm_facts::FactResponseEvidence::new(
            fact_ref.response_schema_id().clone(),
            fact_ref.response_hash().clone(),
            fact_ref.artifact_id().clone(),
            fact_ref.artifact_evidence_hash().clone(),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            fact_ref.capability_kind().clone(),
            fact_ref.capability_version().clone(),
            fact_ref.adapter_kind().clone(),
            fact_ref.adapter_version().clone(),
        ),
    }
}

pub(super) fn replay_stream_fact_descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.replay.test.fact").expect("fact kind"),
        mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
        schema_id("mfm.replay.test.fact_subject", 0xe0),
        fact_response_schema_id(),
        vec![
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("subject.account").expect("field id"),
                mfm_facts::FactFieldValueType::String,
                mfm_facts::FactFieldExtraction::Subject(
                    mfm_facts::CanonicalValuePath::new("account").expect("subject path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .required(),
            )
            .expect("subject field"),
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("result.amount").expect("field id"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldExtraction::Response(
                    mfm_facts::CanonicalValuePath::new("amount").expect("response path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .sortable()
                .required(),
            )
            .expect("result field"),
        ],
        vec![mfm_facts::FactOrderingPolicy::new(
            mfm_facts::FactOrderingName::new("result.amount.desc").expect("ordering"),
            vec![mfm_facts::FactOrderingTerm::new(
                mfm_facts::FactFieldId::new("result.amount").expect("field"),
                mfm_facts::SortDirection::Descending,
                mfm_facts::NullOrdering::Last,
                false,
            )],
        )
        .expect("ordering")],
    )
    .expect("fact descriptor")
}

pub(super) fn replay_stream_other_fact_descriptor() -> mfm_facts::FactDescriptor {
    let descriptor = replay_stream_fact_descriptor();
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.replay.test.other_fact").expect("fact kind"),
        descriptor.descriptor_schema_id().clone(),
        descriptor.subject_schema_id().clone(),
        descriptor.response_schema_id().clone(),
        descriptor.fields().to_vec(),
        descriptor.orderings().to_vec(),
    )
    .expect("other fact descriptor")
}

pub(super) fn replay_stream_fact_subject_evidence(
    descriptor: &mfm_facts::FactDescriptor,
) -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactFieldValue::new(
        mfm_facts::FactFieldId::new("subject.account").expect("field id"),
        mfm_facts::FactFieldValueType::String,
        mfm_facts::FactCanonicalScalar::string("same-subject"),
    )
    .expect("subject value")])
    .expect("subject material");
    let namespace_hash =
        mfm_facts::fact_subject_namespace_hash(descriptor).expect("subject namespace hash");
    mfm_facts::FactSubjectEvidence::from_material(namespace_hash, &material)
        .expect("subject evidence")
}

pub(super) fn replay_stream_fact_response_artifact(
    descriptor: &mfm_facts::FactDescriptor,
    amount: u64,
) -> (StoredArtifactEvidenceRef, Vec<u8>) {
    let json =
        serde_json::to_string(&serde_json::json!({ "amount": amount })).expect("response json");
    let bytes =
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical response");
    let digest = bytes.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    (
        StoredArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: bytes.as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media type"),
            schema_id: Some(descriptor.response_schema_id().clone()),
            semantic_type_id: None,
            producer_node_id: Some(fact_node_id()),
            producer_seed_id: None::<SeedId>,
            artifact_role: ArtifactRole::FactResponse,
        },
        bytes.to_vec(),
    )
}

pub(super) fn replay_stream_fact_claim(
    descriptor: &mfm_facts::FactDescriptor,
    response_artifact: &StoredArtifactEvidenceRef,
) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        fact_kind: descriptor.fact_kind().clone(),
        fact_descriptor_hash: mfm_facts::fact_descriptor_hash(descriptor).expect("descriptor hash"),
        subject: replay_stream_fact_subject_evidence(descriptor),
        observed_at: None,
        request: Some(mfm_facts::FactRequestEvidence::new(
            fact_request_schema_id(),
            fact_request_hash(),
        )),
        response: mfm_facts::FactResponseEvidence::new(
            descriptor.response_schema_id().clone(),
            response_artifact.digest.clone(),
            response_artifact.artifact_id.clone(),
            response_artifact
                .evidence_hash()
                .expect("response evidence hash"),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            fact_capability_kind(),
            fact_capability_version(),
            fact_adapter_kind(),
            fact_adapter_version(),
        ),
    })
    .expect("fact claim")
}

pub(super) fn hashed_fact_replay_spec() -> HashedSpecEnvelope {
    let envelope = fact_replay_spec();
    HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("hashed replay spec")
}

pub(super) fn hashed_fact_replay_spec_with_other_node_descriptor(
    other_descriptor: &mfm_facts::FactDescriptor,
) -> HashedSpecEnvelope {
    let mut envelope = fact_replay_spec();
    let other_ref = spec::FactDescriptorRef {
        descriptor_hash: mfm_facts::fact_descriptor_hash(other_descriptor)
            .expect("other descriptor hash"),
    };
    let template = envelope.spec.nodes[0].clone();
    let other_descriptor_id = descriptor_id(0xdd);
    let other_node = spec::NodeSpec {
        node_id: node_id(0xde),
        stable_key: spec::StableAuthorKey::new("other-fact-node").expect("stable key"),
        scope_id: template.scope_id.clone(),
        state_kind: template.state_kind.clone(),
        state_version: template.state_version.clone(),
        descriptor_id: other_descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: template.config_ref.clone(),
        input_bindings: template.input_bindings.clone(),
        output_cell: cell_id(0xdf),
        effect_kind: template.effect_kind.clone(),
        capability_bindings: template.capability_bindings.clone(),
        adapter_bindings: template.adapter_bindings.clone(),
        fact_descriptor_allowlist: vec![other_ref.clone()],
        side_effect: None,
        framework: None,
        planning_lineage: template.planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    };
    let mut other_state_identity = envelope
        .spec
        .descriptor_identities
        .iter()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state) => Some((**state).clone()),
            _ => None,
        })
        .expect("state descriptor identity");
    other_state_identity.descriptor_id = other_descriptor_id;
    other_state_identity.name = "mfm.replay.test.other_fact_state".to_owned();
    other_state_identity.emitted_fact_descriptors = vec![other_ref];
    envelope
        .spec
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            other_state_identity,
        )));
    envelope.spec.nodes.push(other_node);
    HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("hashed replay spec")
}

pub(super) fn fact_run_admitted_for_stream(
    certified_spec: &HashedSpecEnvelope,
    descriptor: events::RunArtifactEvidenceRef,
) -> events::RunAdmitted {
    fact_run_admitted_for_stream_with_descriptors(certified_spec, vec![descriptor])
}

pub(super) fn fact_run_admitted_for_stream_with_descriptors(
    certified_spec: &HashedSpecEnvelope,
    descriptors: Vec<events::RunArtifactEvidenceRef>,
) -> events::RunAdmitted {
    let spec_hash = certified_spec.spec_hash.clone();
    let identity_material = events::RunIdentityMaterialV1 {
        certified_spec_hash: spec_hash.clone(),
        store_scope_id: mfm_ids::StoreScopeId::new(
            "mfm.store_scope.v1:000000000000000000000000000000c1",
        )
        .expect("store scope"),
        invocation_key_digest: content_digest(0xc2),
    };
    let run_id = identity_material.derive_run_id().expect("run id");
    events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: events::EntryPointLaunchEvidence {
            resolved_op_id: events::EntryPointOpId::new("mfm.replay.test.fact")
                .expect("entry point"),
            entry_point_registry_digest: content_digest(0xc2),
        },
        spec_hash,
        spec_artifact: stream_run_admitted_spec_artifact_for_hash(&certified_spec.spec_hash),
        certificate_artifact: stream_run_admitted_certificate_artifact(),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: descriptors,
        spec_version: certified_spec.spec.spec_version.clone(),
        lowering_version: certified_spec.spec.lowering_version.clone(),
        public_output_schema_id: certified_spec.spec.public_outputs.public_schema_id.clone(),
        saga_policy_digest: content_digest(0xca),
        descriptor_identities: certified_spec.spec.descriptor_identities.clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: admitted_binding_digest(&[], &[]).expect("binding digest"),
        canonicalizer_identity: certified_spec
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        seed_cells: Vec::new(),
    }
}

pub(super) fn stream_run_admitted_spec_artifact() -> events::RunArtifactEvidenceRef {
    stream_run_admitted_spec_artifact_for_hash(&hashed_fact_replay_spec().spec_hash)
}

pub(super) fn stream_run_admitted_spec_artifact_for_hash(
    spec_hash: &SpecHash,
) -> events::RunArtifactEvidenceRef {
    run_artifact_ref(
        ArtifactRole::TypedExecutionSpec,
        spec::typed_execution_spec_schema_id().expect("spec schema"),
        spec_digest(spec_hash),
    )
}

pub(super) fn stream_run_admitted_certificate_artifact() -> events::RunArtifactEvidenceRef {
    run_artifact_ref(
        ArtifactRole::TypedSpecCertificate,
        mfm_certify::typed_spec_certificate_schema_id().expect("cert schema"),
        content_digest(0xc8),
    )
}

pub(super) fn stored_artifact_from_run_ref(
    artifact: &events::RunArtifactEvidenceRef,
) -> StoredArtifactEvidenceRef {
    StoredArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        digest: artifact.content_digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: artifact.role,
    }
}

pub(super) fn replay_stream_config_artifact(
    certified_spec: &HashedSpecEnvelope,
) -> StoredArtifactEvidenceRef {
    let config = certified_spec.spec.config_refs.first().expect("config ref");
    StoredArtifactEvidenceRef {
        artifact_id: config.artifact_id.clone(),
        digest: config.digest.clone(),
        byte_len: config.byte_len,
        media_type: config.media_type.clone(),
        schema_id: Some(config.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedConfig,
    }
}

pub(super) fn stored_artifact_ref(
    digest: ContentDigest,
    role: ArtifactRole,
    schema_id: Option<SchemaId>,
    producer_node_id: Option<NodeId>,
    byte_len: u64,
) -> StoredArtifactEvidenceRef {
    StoredArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id,
        semantic_type_id: None,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    }
}
