use super::*;

pub(super) fn replay_broker_with_facts(
    facts: Vec<(
        mfm_facts::FactClaimId,
        events::FactRecorded,
        StoredArtifactEvidenceRef,
    )>,
) -> ReplayBroker {
    let mut fact_map = BTreeMap::new();
    let mut artifact_map = BTreeMap::new();
    for (claim_id, fact, artifact) in facts {
        fact_map.insert(claim_id, fact);
        artifact_map.insert(
            (
                artifact.artifact_id.clone(),
                artifact.evidence_hash().expect("artifact evidence hash"),
            ),
            artifact,
        );
    }
    ReplayBroker {
        certified_spec: fact_replay_spec(),
        stream: Vec::new(),
        run_id: fact_run_admitted(),
        projection: ProjectionSnapshot::default(),
        retained_artifacts: artifact_map.clone(),
        artifact_bytes: BTreeMap::new(),
        artifact_byte_authority: BTreeMap::new(),
        artifacts: artifact_map,
        facts: fact_map,
        fact_events: BTreeMap::new(),
        intents: BTreeMap::new(),
        prepared_invocations: BTreeMap::new(),
        submissions: BTreeMap::new(),
        not_submitted: BTreeMap::new(),
        receipts: BTreeMap::new(),
        confirmations: BTreeMap::new(),
        ambiguities: BTreeMap::new(),
        manual_resolutions: BTreeMap::new(),
    }
}

pub(super) fn fact_replay_spec() -> HashedSpecEnvelope {
    let descriptor_ref = spec::FactDescriptorRef {
        descriptor_hash: mfm_facts::fact_descriptor_hash(&replay_stream_fact_descriptor())
            .expect("descriptor hash"),
    };
    let capability = mfm_capabilities::CapabilityDescriptor::new(
        fact_capability_kind(),
        fact_capability_version(),
        mfm_capabilities::CapabilityRole::ReadExternal,
        "fact_read",
    )
    .expect("capability descriptor");
    let capability_bindings =
        CapabilitySetDescriptor::new(vec![capability]).expect("capability set");
    let config_ref = spec::ConfigRef {
        schema_id: schema_id("mfm.replay.test.config", 0xb0),
        artifact_id: artifact_id(0xb1),
        digest: content_digest(0xb2),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    };
    let input_bindings = spec::InputBindingSpec {
        input_schema_id: schema_id("mfm.replay.test.input", 0xb3),
        input_descriptor_id: descriptor_id(0xb4),
        root: spec::InputBindingNodeSpec::Unit,
        digest: content_digest(0xb5),
    };
    let public_schema_id = schema_id("mfm.replay.test.public", 0xb6);
    let renderer_descriptor = spec::RendererDescriptorIdentity {
        descriptor_id: descriptor_id(0xb7),
        renderer_kind: spec::RendererKind::new("replay-test-renderer").expect("renderer kind"),
        renderer_version: spec::RendererVersion::new("mfm.replay.test.renderer.v1")
            .expect("renderer version"),
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity: CanonicalizerIdentity::new("sha256-jcs-v1")
            .expect("canonicalizer identity"),
    };
    let node = spec::NodeSpec {
        node_id: fact_node_id(),
        stable_key: spec::StableAuthorKey::new("fact-node").expect("stable key"),
        scope_id: scope_id(0xb8),
        state_kind: state_kind(0xb9),
        state_version: mfm_ids::StateVersion::new("mfm.replay.test.fact_state.v1")
            .expect("state version"),
        descriptor_id: descriptor_id(0xba),
        context: spec::NodeContextSpec::no_context(),
        config_ref: config_ref.clone(),
        input_bindings,
        output_cell: cell_id(0xbb),
        effect_kind: effect_kind(0xbc),
        capability_bindings,
        adapter_bindings: vec![spec::AdapterBinding {
            adapter_kind: fact_adapter_kind(),
            adapter_version: fact_adapter_version(),
            binding_digest: None,
        }],
        fact_descriptor_allowlist: vec![descriptor_ref.clone()],
        side_effect: None,
        framework: None,
        planning_lineage: spec::PlanningLineage {
            active_operation_instances: Vec::new(),
            completed_operation_frames: Vec::new(),
            lineage_digest: content_digest(0xbd),
        },
        deterministic_predecessors: Vec::new(),
    };
    let state_identity = spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
        descriptor_id: node.descriptor_id.clone(),
        name: "mfm.replay.test.fact_state".to_owned(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        context: spec::StateContextDescriptorSpec::no_context(),
        input_context: spec::StateInputContextContractSpec::no_context(),
        output_context: spec::StateOutputContextContractSpec::no_context(),
        config_schema_id: node.config_ref.schema_id.clone(),
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        output_schema_id: schema_id("mfm.replay.test.output", 0xd5),
        output_semantic_type_id: semantic_type_id(0xd6),
        effect_kind: node.effect_kind.clone(),
        effect_class: "read".to_owned(),
        effect_name: "read".to_owned(),
        effect_version: mfm_ids::EffectVersion::new("mfm.replay.test.effect.v1")
            .expect("effect version"),
        capabilities: node.capability_bindings.clone(),
        emitted_fact_descriptors: vec![descriptor_ref],
        runner: "mfm.replay.test.runner".to_owned(),
        side_effect_contract_digest: None,
    }));
    let spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
        authoring: spec::AuthoringProvenance::StateComposition {
            descriptor: spec::CompositionDescriptor {
                descriptor_id: descriptor_id(0xbe),
                name: "mfm.replay.test.fact_composition".to_owned(),
                version: "mfm.replay.test.fact_composition.v1".to_owned(),
            },
            config_hash: content_digest(0xbf),
        },
        saga: spec::SagaPolicySpec::NoSideEffects,
        contexts: Vec::new(),
        scopes: Vec::new(),
        seeds: Vec::new(),
        descriptor_identities: vec![state_identity],
        config_refs: vec![config_ref],
        nodes: vec![node],
        remediations: BTreeMap::new(),
        cells: Vec::new(),
        value_lineages: Vec::new(),
        planning_lineage: Vec::new(),
        public_outputs: spec::PublicOutputSpec {
            public_schema_id,
            outputs: Vec::new(),
            renderer_descriptor,
        },
    })
    .expect("typed spec");
    HashedSpecEnvelope {
        spec_hash: spec_hash(0xc0),
        spec,
        audit: spec::TypedExecutionSpecAudit::default(),
    }
}

pub(super) fn fact_run_admitted() -> events::RunAdmitted {
    let spec_hash = spec_hash(0xc0);
    events::RunAdmitted {
        run_id: run_id(0xc1),
        identity_material: events::RunIdentityMaterialV1 {
            certified_spec_hash: spec_hash.clone(),
            store_scope_id: mfm_ids::StoreScopeId::new(
                "mfm.store_scope.v1:000000000000000000000000000000c1",
            )
            .expect("store scope"),
            invocation_key_digest: content_digest(0xc2),
        },
        entry_point: events::EntryPointLaunchEvidence {
            resolved_op_id: events::EntryPointOpId::new("mfm.replay.test.fact")
                .expect("entry point"),
            entry_point_registry_digest: content_digest(0xc2),
        },
        spec_hash,
        spec_artifact: run_artifact_ref(
            ArtifactRole::TypedExecutionSpec,
            schema_id("mfm.replay.test.spec", 0xc4),
            content_digest(0xc5),
        ),
        certificate_artifact: run_artifact_ref(
            ArtifactRole::TypedSpecCertificate,
            schema_id("mfm.replay.test.certificate", 0xc7),
            content_digest(0xc8),
        ),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: Vec::new(),
        spec_version: mfm_ids::SpecVersion::new("1").expect("spec version"),
        lowering_version: mfm_ids::LoweringVersion::new("mfm.lowering.v1")
            .expect("lowering version"),
        public_output_schema_id: schema_id("mfm.replay.test.public", 0xc9),
        saga_policy_digest: content_digest(0xca),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(0xcb),
        canonicalizer_identity: CanonicalizerIdentity::new("sha256-jcs-v1")
            .expect("canonicalizer identity"),
        seed_cells: Vec::new(),
    }
}

pub(super) fn run_artifact_ref(
    role: ArtifactRole,
    schema_id: SchemaId,
    content_digest: ContentDigest,
) -> events::RunArtifactEvidenceRef {
    let artifact_id = ArtifactId::from_digest(content_digest.algorithm(), *content_digest.digest());
    let store = StoredArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: content_digest.clone(),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: Some(schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: role,
    };
    events::RunArtifactEvidenceRef {
        artifact_id,
        role,
        schema_id: Some(schema_id),
        semantic_type_id: None,
        content_digest,
        evidence_hash: store.evidence_hash().expect("run artifact evidence hash"),
        byte_len: 2,
        media_type: store.media_type,
    }
}

pub(super) fn fact_recorded(artifact: &StoredArtifactEvidenceRef) -> events::FactRecorded {
    events::FactRecorded {
        spec_hash: spec_hash(0xc0),
        node_id: fact_node_id(),
        attempt_id: fact_attempt_id(),
        claim: fact_claim(artifact),
    }
}

pub(super) fn fact_claim(artifact: &StoredArtifactEvidenceRef) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        fact_kind: mfm_facts::FactKind::new("mfm.replay.test.fact").expect("fact kind"),
        fact_descriptor_hash: content_digest(0xcc),
        subject: fact_subject_evidence(),
        observed_at: None,
        request: Some(mfm_facts::FactRequestEvidence::new(
            fact_request_schema_id(),
            fact_request_hash(),
        )),
        response: mfm_facts::FactResponseEvidence::new(
            fact_response_schema_id(),
            artifact.digest.clone(),
            artifact.artifact_id.clone(),
            artifact.evidence_hash().expect("artifact evidence hash"),
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

pub(super) fn fact_subject_evidence() -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactFieldValue::new(
        mfm_facts::FactFieldId::new("subject.account").expect("field id"),
        mfm_facts::FactFieldValueType::String,
        mfm_facts::FactCanonicalScalar::string("same-subject"),
    )
    .expect("subject value")])
    .expect("subject material");
    mfm_facts::FactSubjectEvidence::from_material(content_digest(0xcd), &material)
        .expect("subject evidence")
}

#[derive(Clone)]
pub(super) struct ReplayFactStreamFixture {
    pub(super) certified_spec: HashedSpecEnvelope,
    pub(super) stream: Vec<KernelEventEnvelope>,
    pub(super) artifact_evidence: Vec<StoredArtifactEvidenceRef>,
    pub(super) artifact_bytes: BTreeMap<(ArtifactId, ContentDigest), Vec<u8>>,
    pub(super) run_id: RunId,
    pub(super) claim_id: mfm_facts::FactClaimId,
    pub(super) response_artifact_id: ArtifactId,
}

impl ReplayFactStreamFixture {
    pub(super) fn authority(&self) -> ReplayReadAuthority {
        ReplayReadAuthority {
            certified_spec: self.certified_spec.clone(),
            stream: self.stream.clone(),
            canonicalizer_identity: self
                .certified_spec
                .spec
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
                .clone(),
            runner_executables: Vec::new(),
            adapter_executables: Vec::new(),
            artifact_evidence: self.artifact_evidence.clone(),
            artifact_bytes: self.artifact_bytes.clone(),
            additional_artifact_evidence: Vec::new(),
            source_fact_events: Vec::new(),
        }
    }
}
