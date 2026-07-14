use super::*;

pub(super) fn test_evm_provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(mfm_evm_capabilities::evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::TransportFailed,
    ))
}

pub(super) fn evidence_for_source(
    network_id: &EvmNetworkId,
    expected_chain_id: u64,
) -> RedactedEvmSourceEvidence {
    RedactedEvmSourceEvidence {
        network_id: network_id.clone(),
        expected_chain_id,
        observed_chain_id: expected_chain_id,
        source_ref: EvmSourceRef::new("local").expect("source"),
        policy_id: EvmSourcePolicyId::new("test").expect("policy"),
    }
}

pub(super) fn test_evm_evidence() -> RedactedEvmSourceEvidence {
    evidence_for_source(
        &EvmNetworkId::new("ethereum-mainnet").expect("test network"),
        1,
    )
}

pub(super) fn artifact_json() -> serde_json::Value {
    json!({
        "abi": {
            "json_text": json!([
                {
                    "type": "constructor",
                    "inputs": []
                },
                {
                    "type": "function",
                    "name": "configure",
                    "inputs": [],
                    "outputs": [],
                    "stateMutability": "nonpayable"
                },
                {
                    "type": "function",
                    "name": "owner",
                    "inputs": [],
                    "outputs": [{ "name": "", "type": "bool" }],
                    "stateMutability": "view"
                }
            ]).to_string()
        },
        "bytecode": {
            "json_text": json!({"object": "0x6000"}).to_string()
        }
    })
}

pub(super) fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

pub(super) fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

pub(super) fn content_digest_str(byte: u8) -> String {
    content_digest(byte).to_string()
}

pub(super) fn artifact_id_str(byte: u8) -> String {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn cell_id_str(byte: u8) -> String {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn descriptor_id_str(byte: u8) -> String {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn event_id_str(byte: u8) -> String {
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn run_id_str(byte: u8) -> String {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn spec_hash_str(byte: u8) -> String {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

pub(super) fn artifact_evidence_ref(byte: u8, byte_len: u64) -> LifecycleArtifactEvidenceRef {
    let digest = content_digest(byte);
    let schema_id = <ContractArtifactConfig as MfmConfig>::schema_id().expect("artifact schema");
    let store_evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest: digest.clone(),
        byte_len,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: Some(schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    lifecycle_ref_from_store_artifact(&store_evidence)
}

pub(super) fn source_artifact_ref_json(
    byte: u8,
    content_digest: serde_json::Value,
    schema_id: Option<String>,
    semantic_type_id: Option<String>,
) -> serde_json::Value {
    json!({
        "artifact_id": artifact_id_str(byte),
        "content_digest": content_digest,
        "evidence_hash": content_digest_str(byte.wrapping_add(0x80)),
        "byte_len": 128,
        "schema_id": schema_id,
        "semantic_type_id": semantic_type_id,
    })
}

pub(super) fn context_json(network_id: &str, expected_chain_id: u64) -> serde_json::Value {
    json!({
        "lifecycle_key": "adapter-test-lifecycle",
        "network": {
            "network_id": network_id,
            "expected_chain_id": expected_chain_id,
            "chain_fingerprint": null,
            "finality_or_observation_policy": null,
        },
        "contract_profile": {
            "profile_id": "adapter-test-profile",
            "artifact_digest": content_digest(0x42).to_string(),
            "artifact_ref": artifact_evidence_ref(0x42, 123),
            "interface_digest": null,
            "creation_bytecode_digest": null,
            "deployed_code_hash": null,
            "selector_event_policy_digest": null,
        },
    })
}

pub(super) fn certified_contract_context(
    network_id: &str,
    expected_chain_id: u64,
) -> mfm_program::CertifiedContext<EvmContractContext> {
    let value: EvmContractContext =
        serde_json::from_value(context_json(network_id, expected_chain_id)).expect("context");
    let mfm_program::StateContextDescriptorSpec::Required(requirement) =
        <EvmContractContext as StateContext>::descriptor().expect("descriptor")
    else {
        panic!("EVM contract context must require context");
    };
    let json = serde_json::to_string(&value).expect("context json");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical context json");
    let context_ref = mfm_program::CertifiedContextSpec::derive_context_ref(
        &requirement.context_descriptor_id,
        &requirement.schema_id,
        &requirement.semantic_type_id,
        &requirement.canonicalizer_identity,
        &canonical,
    )
    .expect("context ref");
    let spec = mfm_program::CertifiedContextSpec {
        context_ref,
        context_descriptor_id: requirement.context_descriptor_id,
        schema_id: requirement.schema_id,
        semantic_type_id: requirement.semantic_type_id,
        canonicalizer_identity: requirement.canonicalizer_identity,
        canonical_context_digest: canonical.content_digest(),
        canonical_context_byte_len: canonical.as_bytes().len() as u64,
        canonical_context: canonical,
    };
    mfm_program::CertifiedContext::from_certified_spec(&spec).expect("certified context")
}

pub(super) fn deployed_source_value(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> DeployedContractInstance {
    DeployedContractInstance {
        lifecycle_version: 1,
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        address: ContractAddress::new("0x000000000000000000000000000000000000beef")
            .expect("address"),
        deploy_provenance: DeployProvenance::MfmDeploy {
            deploy_tx_hash: TEST_TRANSACTION_HASH.to_owned(),
        },
        deploy_evidence: Vec::new(),
        external_adoption_evidence: None,
        deployed_block_number: Some(1),
    }
}

pub(super) fn source_run_import_source_json(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: &str,
    value_digest: &ContractProfileDigestRef,
) -> serde_json::Value {
    json!({
        "source_run_id": run_id_str(0x30),
        "source_spec_hash": spec_hash_str(0x31),
        "source_cell_or_output_id": {
            "kind": "cell",
            "cell_id": cell_id_str(0x32),
        },
        "source_value_digest": value_digest,
        "source_context_ref": context.context_ref().to_string(),
        "required_stage": required_stage,
    })
}

pub(super) fn source_run_import_evidence_json<T>(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    source: &ImportFromMfmRun,
    required_stage: &str,
    import_policy_digest: &ContractProfileDigestRef,
) -> serde_json::Value
where
    T: mfm_values::MfmValue,
{
    let schema_id = T::schema_id().expect("schema").to_string();
    let semantic_type_id = T::semantic_id().expect("semantic").to_string();
    json!({
        "source_spec_hash": &source.source_spec_hash,
        "source_spec_artifact_ref": source_artifact_ref_json(
            0x4e,
            json!(content_digest_str(0x4f)),
            None,
            None,
        ),
        "source_spec_certificate_ref": source_artifact_ref_json(
            0x50,
            json!(content_digest_str(0x51)),
            None,
            None,
        ),
        "source_run_stream_ref": source_artifact_ref_json(
            0x52,
            json!(content_digest_str(0x53)),
            None,
            None,
        ),
        "source_cell_or_output_id": &source.source_cell_or_output_id,
        "source_cell_schema_id": schema_id,
        "source_cell_semantic_type_id": semantic_type_id,
        "source_producer_descriptor_id": descriptor_id_str(0x54),
        "source_stage": required_stage,
        "source_context_ref": &source.source_context_ref,
        "source_context_descriptor_id": context.context_descriptor_id().to_string(),
        "source_value_digest": &source.source_value_digest,
        "source_value_artifact_ref_or_inline_canonical_value": source_artifact_ref_json(
            0x56,
            serde_json::to_value(&source.source_value_digest).expect("digest json"),
            Some(schema_id),
            Some(semantic_type_id),
        ),
        "source_terminal_cell_or_output_event_ref": event_id_str(0x57),
        "import_policy_digest": import_policy_digest,
    })
}

pub(super) fn source_run_import_json<T>(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: &str,
    value_digest: &ContractProfileDigestRef,
) -> serde_json::Value
where
    T: mfm_values::MfmValue,
{
    let source_json = source_run_import_source_json(context, required_stage, value_digest);
    let source: ImportFromMfmRun = serde_json::from_value(source_json).expect("source");
    let import_policy_digest = digest_for_value(&source).expect("policy digest");
    let evidence_json = source_run_import_evidence_json::<T>(
        context,
        &source,
        required_stage,
        &import_policy_digest,
    );
    json!({
        "kind": "from_mfm_run",
        "source": source,
        "evidence": evidence_json,
    })
}

pub(super) struct MissingRetainedArtifacts;

impl store::RetainedArtifactReadProvider for MissingRetainedArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            Err(store::StoreError::MissingArtifact {
                artifact_id: requirement.artifact_id.clone(),
            })
        })
    }
}

pub(super) struct StaticRetainedArtifacts {
    pub(super) artifacts: Vec<(store::ArtifactEvidenceRef, Vec<u8>)>,
}

impl store::RetainedArtifactReadProvider for StaticRetainedArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let (evidence, bytes) = self
                .artifacts
                .iter()
                .find(|(evidence, _)| {
                    evidence.artifact_id == requirement.artifact_id
                        && evidence
                            .evidence_hash()
                            .map(|hash| hash == requirement.evidence_hash)
                            .unwrap_or(false)
                })
                .ok_or_else(|| store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                })?;
            store::VerifiedRunArtifactBytes::new(bytes.clone(), evidence.clone(), requirement)
        })
    }
}

pub(super) fn canonical_value_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let json = serde_json::to_string(value).expect("json");
    PlainCanonicalJsonBytes::from_json_str(&json)
        .expect("canonical json")
        .as_bytes()
        .to_vec()
}

pub(super) fn artifact_ref_for_raw_bytes(
    bytes: &[u8],
) -> (LifecycleArtifactEvidenceRef, store::ArtifactEvidenceRef) {
    let store_evidence = store_artifact_evidence_for_bytes(
        bytes,
        events::ArtifactRole::StateOutput,
        spec::MediaType::new("application/json").expect("media type"),
        None,
        None,
        None,
    );
    (
        lifecycle_ref_from_store_artifact(&store_evidence),
        store_evidence,
    )
}

pub(super) fn source_run_certification_registry() -> mfm_certify::CertificationRegistry {
    let mut registry = mfm_certify::CertificationRegistry::new();
    registry
        .register_state::<ContextBoundDeployContractState>()
        .expect("deploy certifier");
    registry
        .register_state::<ContextBoundConfigureContractState>()
        .expect("configure certifier");
    registry
        .register_state::<ContextBoundValidateContractState>()
        .expect("validate certifier");
    registry
}

pub(super) fn source_run_state_registry() -> mfm_program::StateRegistrySnapshot {
    let mut states = mfm_program::StateRegistryBuilder::new();
    states
        .register::<ContextBoundDeployContractState>()
        .expect("deploy state descriptor");
    states
        .register::<ContextBoundConfigureContractState>()
        .expect("configure state descriptor");
    states
        .register::<ContextBoundValidateContractState>()
        .expect("validate state descriptor");
    states.into_snapshot()
}

pub(super) struct SourceRunPublicOutputs<'program, 'scope> {
    deployed: mfm_program::Handle<'program, 'scope, DeployedContractInstance>,
    configured: mfm_program::Handle<'program, 'scope, ConfiguredContractInstance>,
}

impl<'program, 'scope> mfm_program::PublicOutputs<'program, 'scope>
    for SourceRunPublicOutputs<'program, 'scope>
{
    fn public_schema_id(&self) -> mfm_program::Result<SchemaId> {
        SchemaId::new(
            "mfm.evm.contract.test.source_run_public_outputs",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.evm.contract.test.source_run_public_outputs"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn output_cells(&self) -> mfm_program::Result<Vec<mfm_program::PublicOutputCellSpec>> {
        Ok(vec![
            mfm_program::PublicOutputCellSpec::from_handle(
                mfm_program::PublicFieldPath::new("deployed")?,
                &self.deployed,
            ),
            mfm_program::PublicOutputCellSpec::from_handle(
                mfm_program::PublicFieldPath::new("configured")?,
                &self.configured,
            ),
        ])
    }
}

pub(super) struct CertifiedSourceRunAuthority {
    source_run_id: RunId,
    source_spec_hash: mfm_evm_contract_model::LifecycleSpecHashRef,
    source_cell_or_output_id: SourceCellOrOutputRef,
    source_cell_id: CellId,
    source_scope_id: ScopeId,
    source_value_lineage: spec::ValueLineageRef,
    source_cell_context: spec::CellContextSpec,
    source_producer_node: spec::NodeSpec,
    source_producer_descriptor_id: mfm_evm_contract_model::LifecycleDescriptorIdRef,
    source_spec_bytes: Vec<u8>,
    source_spec_artifact: store::ArtifactEvidenceRef,
    certificate_bytes: Vec<u8>,
    certificate_artifact: store::ArtifactEvidenceRef,
    source_spec: spec::TypedExecutionSpec,
}

pub(super) fn certified_source_run_authority(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
) -> CertifiedSourceRunAuthority {
    let chain_id = context.value().network.expected_chain_id();
    let signer = json!({
        "signer_ref": "deployer",
        "expected_signer_address": expected_test_signer_address_for_chain("eip1559", chain_id),
    });
    let deploy: DeployAction = serde_json::from_value(json!({
        "signer": signer.clone(),
    }))
    .expect("source deploy action");
    let configure: ConfigureAction = serde_json::from_value(json!({
        "signer": signer,
        "calls": [],
    }))
    .expect("source configure action");
    let draft = source_run_program_draft(context.value().clone(), deploy, configure)
        .expect("source lifecycle draft");
    let registry = source_run_certification_registry();
    let lowered = mfm_certify::lower_program_draft(&draft).expect("lowered source spec");
    let scoped = registry
        .scoped_for_spec(lowered.spec())
        .expect("scoped source registry");
    let certified = mfm_certify::certify_typed_spec(lowered, &scoped).expect("certified source");
    let (cell, producer_node, producer_descriptor_id) =
        certified_lifecycle_source_cell(&certified, context, required_stage);
    let persisted = certified
        .to_persisted_parts()
        .expect("persisted source spec certificate");
    let source_spec_bytes = persisted.spec_bytes().to_vec();
    let certificate_bytes = persisted.certificate_bytes().to_vec();
    let source_spec_artifact = source_spec_artifact_evidence(&source_spec_bytes);
    let certificate_artifact = source_certificate_artifact_evidence(&certificate_bytes);
    let identity_material = source_run_identity_material(certified.spec_hash());
    let source_run_id = identity_material.derive_run_id().expect("source run id");
    CertifiedSourceRunAuthority {
        source_run_id,
        source_spec_hash: mfm_evm_contract_model::LifecycleSpecHashRef::from(
            certified.spec_hash().clone(),
        ),
        source_cell_or_output_id: SourceCellOrOutputRef::Cell {
            cell_id: mfm_evm_contract_model::LifecycleCellIdRef::from(cell.cell_id.clone()),
        },
        source_cell_id: cell.cell_id,
        source_scope_id: cell.scope_id,
        source_value_lineage: cell.value_lineage,
        source_cell_context: cell.context,
        source_producer_node: producer_node,
        source_producer_descriptor_id: mfm_evm_contract_model::LifecycleDescriptorIdRef::from(
            producer_descriptor_id,
        ),
        source_spec_bytes,
        source_spec_artifact,
        certificate_bytes,
        certificate_artifact,
        source_spec: certified.envelope().spec.clone(),
    }
}

pub(super) fn source_run_program_draft(
    context: EvmContractContext,
    deploy: DeployAction,
    configure: ConfigureAction,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    mfm_program::build_root_with_registries(
        mfm_program::ScopeKey::new("source_run")?,
        source_run_state_registry(),
        mfm_program::OperationRegistryBuilder::new().into_snapshot(),
        |root| {
            root.set_saga_policy(mfm_program::SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let context = root.scope().declare_context(context)?;
            let deployed = root
                .scope()
                .side_effect::<ContextBoundDeployContractState, _>(
                    mfm_program::StateKey::new("deploy")?,
                    &context,
                    deploy,
                    (),
                    mfm_state_evm_contracts::account_nonce_resource_claim()?,
                    mfm_program::SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            let configured = root
                .scope()
                .side_effect::<ContextBoundConfigureContractState, _>(
                    mfm_program::StateKey::new("configure")?,
                    &context,
                    configure,
                    mfm_state_evm_contracts::ContextConfigureContractInputHandles {
                        deployed: deployed.clone(),
                    },
                    mfm_state_evm_contracts::account_nonce_resource_claim()?,
                    mfm_program::SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            root.bind_public_outputs(
                mfm_program::PublicOutputKey::new("contract")?,
                &SourceRunPublicOutputs {
                    deployed,
                    configured,
                },
            )
        },
    )
}

pub(super) fn certified_lifecycle_source_cell(
    certified: &mfm_certify::CertifiedTypedSpec,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
) -> (spec::CellSpec, spec::NodeSpec, DescriptorId) {
    let expected_stage = context_stage_for_lifecycle_stage(required_stage);
    let (_, cell) = certified
        .validated_spec()
        .graph()
        .cells()
        .find(|(_, cell)| {
            matches!(
                &cell.context,
                spec::CellContextSpec::Bound {
                    context_ref,
                    resource_kind,
                    stage,
                    ..
                } if context_ref == context.context_ref()
                    && resource_kind == mfm_evm_contract_model::contract_instance_resource_kind()
                    && stage == expected_stage
            )
        })
        .expect("certified lifecycle source cell");
    let spec::CellProducer::Node(node_id) = &cell.producer else {
        panic!("source lifecycle cell must be node-produced");
    };
    let node = certified
        .validated_spec()
        .graph()
        .forward_node(node_id)
        .expect("source producer node")
        .clone();
    let spec::CellContextSpec::Bound { producer, .. } = &cell.context else {
        panic!("source lifecycle cell must be context-bound");
    };
    let producer_descriptor_id = producer
        .producer_descriptor_ids
        .first()
        .cloned()
        .expect("source producer descriptor");
    (cell.clone(), node, producer_descriptor_id)
}

pub(super) fn source_run_identity_material(spec_hash: &SpecHash) -> events::RunIdentityMaterialV1 {
    events::RunIdentityMaterialV1 {
        certified_spec_hash: spec_hash.clone(),
        store_scope_id: StoreScopeId::new("mfm.store_scope.v1:30303030303030303030303030303030")
            .expect("store scope"),
        invocation_key_digest: content_digest(0x30),
    }
}

pub(super) fn store_artifact_evidence_for_bytes(
    bytes: &[u8],
    role: events::ArtifactRole,
    media_type: spec::MediaType,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<mfm_ids::SemanticTypeId>,
    producer_node_id: Option<NodeId>,
) -> store::ArtifactEvidenceRef {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type,
        schema_id,
        semantic_type_id,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    }
}

pub(super) fn source_spec_artifact_evidence(bytes: &[u8]) -> store::ArtifactEvidenceRef {
    store_artifact_evidence_for_bytes(
        bytes,
        events::ArtifactRole::TypedExecutionSpec,
        spec::MediaType::new(spec::MEDIA_TYPE).expect("spec media type"),
        Some(spec::typed_execution_spec_schema_id().expect("typed spec schema")),
        None,
        None,
    )
}

pub(super) fn source_certificate_artifact_evidence(bytes: &[u8]) -> store::ArtifactEvidenceRef {
    store_artifact_evidence_for_bytes(
        bytes,
        events::ArtifactRole::TypedSpecCertificate,
        spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).expect("certificate media type"),
        Some(mfm_certify::typed_spec_certificate_schema_id().expect("certificate schema")),
        None,
        None,
    )
}

pub(super) fn source_value_artifact_evidence<T>(
    bytes: &[u8],
    producer_node_id: NodeId,
) -> store::ArtifactEvidenceRef
where
    T: MfmValue,
{
    store_artifact_evidence_for_bytes(
        bytes,
        events::ArtifactRole::StateOutput,
        spec::MediaType::new("application/json").expect("json media type"),
        Some(T::schema_id().expect("source value schema")),
        Some(T::semantic_id().expect("source value semantic")),
        Some(producer_node_id),
    )
}

pub(super) fn lifecycle_ref_from_store_artifact(
    evidence: &store::ArtifactEvidenceRef,
) -> LifecycleArtifactEvidenceRef {
    LifecycleArtifactEvidenceRef::new(
        evidence.artifact_id.clone(),
        evidence.digest.clone(),
        evidence
            .evidence_hash()
            .expect("lifecycle artifact evidence hash"),
        evidence.byte_len,
        evidence.schema_id.clone(),
        evidence.semantic_type_id.clone(),
    )
}

pub(super) fn run_artifact_ref_from_store(
    evidence: &store::ArtifactEvidenceRef,
) -> events::RunArtifactEvidenceRef {
    events::RunArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        role: evidence.artifact_role,
        schema_id: evidence.schema_id.clone(),
        semantic_type_id: evidence.semantic_type_id.clone(),
        content_digest: evidence.digest.clone(),
        evidence_hash: evidence
            .evidence_hash()
            .expect("run artifact evidence hash"),
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
    }
}

pub(super) async fn source_run_stream_artifact<T>(
    authority: &CertifiedSourceRunAuthority,
    source_value_bytes: &[u8],
    source_value_artifact: store::ArtifactEvidenceRef,
) -> (LifecycleArtifactEvidenceRef, Vec<u8>, EventId)
where
    T: MfmValue,
{
    let store = store::AsyncInMemoryRunStore::new();
    let run_authority = store::CertifiedRunStoreAuthority::from_spec(
        authority.source_run_id.clone(),
        &authority.source_spec,
    )
    .expect("source run store authority");
    let run_admission = events::KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id: authority.source_run_id.clone(),
        identity_material: source_run_identity_material(
            &authority
                .source_spec_hash
                .typed()
                .expect("source spec hash"),
        ),
        entry_point: events::EntryPointLaunchEvidence::new("mfm.test/source_run@1", Vec::new())
            .expect("entry point evidence"),
        spec_hash: authority
            .source_spec_hash
            .typed()
            .expect("source spec hash"),
        spec_artifact: run_artifact_ref_from_store(&authority.source_spec_artifact),
        certificate_artifact: run_artifact_ref_from_store(&authority.certificate_artifact),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: Vec::new(),
        spec_version: authority.source_spec.spec_version.clone(),
        lowering_version: authority.source_spec.lowering_version.clone(),
        public_output_schema_id: authority
            .source_spec
            .public_outputs
            .public_schema_id
            .clone(),
        saga_policy_digest: authority
            .source_spec
            .saga
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: authority.source_spec.descriptor_identities.clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(0x62),
        canonicalizer_identity: authority
            .source_spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        seed_cells: Vec::new(),
    }));
    append_source_run_commit(SourceRunCommitAppend {
        store: &store,
        run_id: &authority.source_run_id,
        expected_next_seq: store::StreamSeq::FIRST,
        commit_key: "source-run-admission",
        payloads: vec![run_admission],
        required_artifacts: vec![
            authority.source_spec_artifact.clone(),
            authority.certificate_artifact.clone(),
        ],
        artifact_bytes: vec![
            (
                authority.source_spec_bytes.clone(),
                authority.source_spec_artifact.clone(),
            ),
            (
                authority.certificate_bytes.clone(),
                authority.certificate_artifact.clone(),
            ),
        ],
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::Absent,
            certified_run_authority: Some(run_authority),
            ..store::CommitPreconditions::default()
        },
    })
    .await;

    let attempt_id = AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x63));
    append_source_run_commit(SourceRunCommitAppend {
        store: &store,
        run_id: &authority.source_run_id,
        expected_next_seq: store::StreamSeq::new(2).expect("seq 2"),
        commit_key: "source-run-attempt-start",
        payloads: vec![events::KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: authority
                    .source_spec_hash
                    .typed()
                    .expect("source spec hash"),
                node_id: authority.source_producer_node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no: 1,
                state_kind: authority.source_producer_node.state_kind.clone(),
                state_version: authority.source_producer_node.state_version.clone(),
            },
        )],
        required_artifacts: Vec::new(),
        artifact_bytes: Vec::new(),
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            ..store::CommitPreconditions::default()
        },
    })
    .await;

    append_source_run_commit(SourceRunCommitAppend {
        store: &store,
        run_id: &authority.source_run_id,
        expected_next_seq: store::StreamSeq::new(3).expect("seq 3"),
        commit_key: "source-run-terminal",
        payloads: vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: authority
                    .source_spec_hash
                    .typed()
                    .expect("source spec hash"),
                node_id: authority.source_producer_node.node_id.clone(),
                cell_id: authority.source_cell_id.clone(),
                scope_id: authority.source_scope_id.clone(),
                attempt_id: attempt_id.clone(),
                semantic_type_id: T::semantic_id().expect("source value semantic"),
                schema_id: T::schema_id().expect("source value schema"),
                value_lineage: authority.source_value_lineage.clone(),
                context: authority.source_cell_context.clone(),
                artifact_id: source_value_artifact.artifact_id.clone(),
                content_digest: source_value_artifact.digest.clone(),
                evidence_hash: source_value_artifact
                    .evidence_hash()
                    .expect("source value evidence hash"),
                producer_state_kind: Some(authority.source_producer_node.state_kind.clone()),
                producer_state_version: Some(authority.source_producer_node.state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: authority
                    .source_spec_hash
                    .typed()
                    .expect("source spec hash"),
                node_id: authority.source_producer_node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: authority.source_cell_id.clone(),
            }),
        ],
        required_artifacts: vec![source_value_artifact.clone()],
        artifact_bytes: vec![(source_value_bytes.to_vec(), source_value_artifact)],
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                authority.source_producer_node.node_id, attempt_id
            ))
            .expect("attempt logical key")],
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: authority.source_cell_id.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    })
    .await;

    let committed =
        store::RunEventStore::load_committed_run_stream(&store, &authority.source_run_id)
            .await
            .expect("source committed stream");
    let terminal_event_id = committed
        .events()
        .iter()
        .find_map(|event| {
            matches!(event.payload(), events::KernelEventPayload::CellProduced(_))
                .then(|| event.event_id().clone())
        })
        .expect("terminal cell event");
    let bytes = store::committed_run_stream_canonical_json(&committed)
        .expect("committed stream json")
        .to_vec();
    let (lifecycle_ref, _) = artifact_ref_for_raw_bytes(&bytes);
    (lifecycle_ref, bytes, terminal_event_id)
}

pub(super) struct SourceRunCommitAppend<'a> {
    store: &'a store::AsyncInMemoryRunStore,
    run_id: &'a RunId,
    expected_next_seq: store::StreamSeq,
    commit_key: &'static str,
    payloads: Vec<events::KernelEventPayload>,
    required_artifacts: Vec<store::ArtifactEvidenceRef>,
    artifact_bytes: Vec<(Vec<u8>, store::ArtifactEvidenceRef)>,
    preconditions: store::CommitPreconditions,
}

pub(super) async fn append_source_run_commit(input: SourceRunCommitAppend<'_>) {
    let SourceRunCommitAppend {
        store,
        run_id,
        expected_next_seq,
        commit_key,
        payloads,
        required_artifacts,
        artifact_bytes,
        preconditions,
    } = input;
    let request = store::CommitRequest::from_payloads(
        run_id.clone(),
        expected_next_seq,
        store::CommitKey::new(commit_key).expect("commit key"),
        payloads,
        required_artifacts.clone(),
        preconditions,
    )
    .expect("commit request");
    let artifacts =
        store::CommitArtifactEvidenceSet::new(required_artifacts.clone(), required_artifacts)
            .expect("artifact evidence set");
    let plan = match commit_key {
        "source-run-admission" => {
            store::PreparedCommit::<store::RunAdmission>::new(request, artifacts)
                .expect("run admission commit")
                .into()
        }
        "source-run-attempt-start" => {
            store::PreparedCommit::<store::StateAttemptStarted>::new(request, artifacts)
                .expect("attempt start commit")
                .into()
        }
        "source-run-terminal" => {
            store::PreparedCommit::<store::AttemptTerminal>::new(request, artifacts)
                .expect("terminal commit")
                .into()
        }
        _ => panic!("unknown source run commit"),
    };
    let artifact_bytes = artifact_bytes
        .into_iter()
        .map(|(bytes, evidence)| store::PreparedArtifactBytes::new(bytes, evidence))
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("prepared artifacts");
    let bundle = store::PreparedCommitBundle::new(plan, artifact_bytes, Vec::new())
        .expect("prepared bundle");
    store::RunEventStore::append_prepared_commit_bundle(store, bundle)
        .await
        .expect("append source commit");
}

pub(super) async fn source_run_import_with_authority<T>(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
    source_value: &T,
    mutate_evidence: impl FnOnce(&mut ImportFromMfmRunEvidence),
) -> (ImportDeployedSpec, StaticRetainedArtifacts)
where
    T: MfmValue + Serialize,
{
    let source_value_digest = digest_for_value(source_value).expect("value digest");
    let authority = certified_source_run_authority(context, required_stage);
    let source_json = json!({
        "source_run_id": authority.source_run_id.to_string(),
        "source_spec_hash": &authority.source_spec_hash,
        "source_cell_or_output_id": &authority.source_cell_or_output_id,
        "source_value_digest": source_value_digest,
        "source_context_ref": context.context_ref().to_string(),
        "required_stage": match required_stage {
            ContractLifecycleStage::Deployed => "deployed",
            ContractLifecycleStage::Configured => "configured",
        },
    });
    let source: ImportFromMfmRun = serde_json::from_value(source_json).expect("source");
    let import_policy_digest = digest_for_value(&source).expect("policy digest");
    let evidence_json = source_run_import_evidence_json::<T>(
        context,
        &source,
        match required_stage {
            ContractLifecycleStage::Deployed => "deployed",
            ContractLifecycleStage::Configured => "configured",
        },
        &import_policy_digest,
    );
    let mut evidence: ImportFromMfmRunEvidence =
        serde_json::from_value(evidence_json).expect("evidence");
    evidence.source_producer_descriptor_id = authority.source_producer_descriptor_id.clone();
    let value_bytes = canonical_value_bytes(source_value);
    let source_value_store_artifact = source_value_artifact_evidence::<T>(
        &value_bytes,
        authority.source_producer_node.node_id.clone(),
    );
    let value_ref = lifecycle_ref_from_store_artifact(&source_value_store_artifact);
    let (stream_ref, stream_bytes, terminal_event_id) = source_run_stream_artifact::<T>(
        &authority,
        &value_bytes,
        source_value_store_artifact.clone(),
    )
    .await;
    let stream_store_artifact = store_artifact_evidence_for_bytes(
        &stream_bytes,
        events::ArtifactRole::StateOutput,
        spec::MediaType::new("application/json").expect("media type"),
        None,
        None,
        None,
    );
    evidence.source_terminal_cell_or_output_event_ref =
        mfm_evm_contract_model::LifecycleEventIdRef::from(terminal_event_id);
    mutate_evidence(&mut evidence);
    let source_spec_ref = lifecycle_ref_from_store_artifact(&authority.source_spec_artifact);
    let certificate_ref = lifecycle_ref_from_store_artifact(&authority.certificate_artifact);
    let import = ImportDeployedSpec::FromMfmRun {
        source,
        evidence: ImportFromMfmRunEvidence {
            source_spec_artifact_ref: source_spec_ref,
            source_spec_certificate_ref: certificate_ref,
            source_run_stream_ref: stream_ref,
            source_value_artifact_ref_or_inline_canonical_value: value_ref,
            ..evidence
        },
    };
    (
        import,
        StaticRetainedArtifacts {
            artifacts: vec![
                (
                    authority.source_spec_artifact.clone(),
                    authority.source_spec_bytes,
                ),
                (
                    authority.certificate_artifact.clone(),
                    authority.certificate_bytes,
                ),
                (stream_store_artifact, stream_bytes),
                (source_value_store_artifact, value_bytes),
            ],
        },
    )
}

pub(super) fn source_run_import_with_retargeted_source_value_payload(
    import: ImportDeployedSpec,
    mut artifacts: StaticRetainedArtifacts,
    payload: serde_json::Value,
) -> (ImportDeployedSpec, StaticRetainedArtifacts) {
    let raw_json = serde_json::to_string(&payload).expect("payload json");
    let raw_bytes = PlainCanonicalJsonBytes::from_json_str(&raw_json)
        .expect("payload canonical json")
        .to_vec();
    let (raw_ref, raw_store_evidence) = artifact_ref_for_raw_bytes(&raw_bytes);
    let raw_digest = ContractProfileDigestRef::from(
        raw_ref
            .content_digest()
            .expect("raw payload content digest"),
    );
    let ImportDeployedSpec::FromMfmRun {
        mut source,
        mut evidence,
    } = import
    else {
        panic!("expected source-run import");
    };
    source.source_value_digest = raw_digest.clone();
    evidence.source_value_digest = raw_digest.clone();
    evidence.source_value_artifact_ref_or_inline_canonical_value = raw_ref;
    evidence.import_policy_digest = digest_for_value(&source).expect("import policy digest");

    artifacts.artifacts[3] = (raw_store_evidence, raw_bytes);
    (
        ImportDeployedSpec::FromMfmRun { source, evidence },
        artifacts,
    )
}
