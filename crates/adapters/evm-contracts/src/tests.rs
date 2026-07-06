use super::*;
use mfm_capabilities::CapabilitySpec;
use mfm_evm_capabilities::{
    EvmBlockReadResponse, EvmBlockSelector, EvmCallReadResponse, EvmCapabilityFuture,
    EvmChainIdentityResponse, EvmCodeReadResponse, EvmFeeReadResponse, EvmGasEstimateResponse,
    EvmLogsReadResponse, EvmNonceReadResponse, EvmReceiptReadResponse, EvmSourcePolicyId,
    EvmSourceRef, RedactedEvmSourceEvidence,
};
use mfm_evm_contract_model::{
    BlockSelector, BlockTag, ContractAddress, DeployProvenance, ImportFromMfmRun,
};
use mfm_evm_signing::{
    primitive_signature_from_bytes, recover_signing_address,
    EvmTransactionStyle as SigningTransactionStyle,
};
use mfm_ids::{
    ArtifactId, AttemptId, CellId, ContextRef, DescriptorId, DigestAlgorithm, DigestBytes, EventId,
    NodeId, RunId, SchemaId, ScopeId, SideEffectPairId, SpecHash, TrustScopeId,
};
use mfm_program::StateContext;
use mfm_replay::v1::SideEffectReplayVerifier;
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SigningError, SigningRequest, SigningResult,
};
use mfm_state_evm_contracts::ContextContractTransactionIntent;
use serde_json::json;
use std::sync::{Arc, Mutex};

const TEST_TRANSACTION_HASH: &str =
    "0x1111111111111111111111111111111111111111111111111111111111111111";
const MISMATCH_HASH: &str = TEST_TRANSACTION_HASH;

fn evidence_for_guard(guard: &EvmChainGuard) -> RedactedEvmSourceEvidence {
    RedactedEvmSourceEvidence {
        network_id: guard.network_id().clone(),
        expected_chain_id: guard.expected_chain_id(),
        observed_chain_id: guard.expected_chain_id(),
        source_ref: EvmSourceRef::new("local").expect("source"),
        policy_id: EvmSourcePolicyId::new("test").expect("policy"),
    }
}

fn mismatched_evidence_for_guard(guard: &EvmChainGuard) -> RedactedEvmSourceEvidence {
    RedactedEvmSourceEvidence {
        observed_chain_id: guard.expected_chain_id() + 1,
        ..evidence_for_guard(guard)
    }
}

fn artifact_json() -> serde_json::Value {
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

fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn content_digest_str(byte: u8) -> String {
    content_digest(byte).to_string()
}

fn artifact_id_str(byte: u8) -> String {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn cell_id_str(byte: u8) -> String {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn descriptor_id_str(byte: u8) -> String {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn event_id_str(byte: u8) -> String {
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn run_id_str(byte: u8) -> String {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn spec_hash_str(byte: u8) -> String {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn artifact_evidence_ref(byte: u8, byte_len: u64) -> LifecycleArtifactEvidenceRef {
    let digest = content_digest(byte);
    LifecycleArtifactEvidenceRef::new(
        ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len,
        Some(<ContractArtifactConfig as MfmConfig>::schema_id().expect("artifact schema")),
        None,
    )
}

fn source_artifact_ref_json(
    byte: u8,
    content_digest: serde_json::Value,
    schema_id: Option<String>,
    semantic_type_id: Option<String>,
) -> serde_json::Value {
    json!({
        "artifact_id": artifact_id_str(byte),
        "content_digest": content_digest,
        "byte_len": 128,
        "schema_id": schema_id,
        "semantic_type_id": semantic_type_id,
    })
}

fn context_json(network_id: &str, expected_chain_id: u64) -> serde_json::Value {
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

fn certified_contract_context(
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

fn deployed_source_value(
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

fn source_run_import_source_json(
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

fn source_run_import_evidence_json<T>(
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

fn source_run_import_json<T>(
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

struct MissingRetainedArtifacts;

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

struct StaticRetainedArtifacts {
    artifacts: Vec<(LifecycleArtifactEvidenceRef, Vec<u8>)>,
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
                    evidence
                        .artifact_id()
                        .map(|artifact_id| artifact_id == requirement.artifact_id)
                        .unwrap_or(false)
                })
                .ok_or_else(|| store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                })?;
            store::VerifiedRunArtifactBytes::new(
                bytes.clone(),
                store_artifact_evidence(evidence),
                requirement,
            )
        })
    }
}

fn store_artifact_evidence(evidence: &LifecycleArtifactEvidenceRef) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id().expect("artifact id"),
        digest: evidence.content_digest().expect("content digest"),
        byte_len: evidence.byte_len(),
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: evidence.schema_id().expect("schema id"),
        semantic_type_id: evidence.semantic_type_id().expect("semantic type id"),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    }
}

fn canonical_value_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let json = serde_json::to_string(value).expect("json");
    PlainCanonicalJsonBytes::from_json_str(&json)
        .expect("canonical json")
        .as_bytes()
        .to_vec()
}

fn artifact_ref_for_bytes<T: MfmValue>(bytes: &[u8]) -> LifecycleArtifactEvidenceRef {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    LifecycleArtifactEvidenceRef::new(
        ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
        digest,
        bytes.len() as u64,
        Some(T::schema_id().expect("schema id")),
        Some(T::semantic_id().expect("semantic type id")),
    )
}

fn artifact_ref_for_raw_bytes(bytes: &[u8]) -> LifecycleArtifactEvidenceRef {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    LifecycleArtifactEvidenceRef::new(
        ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
        digest,
        bytes.len() as u64,
        None,
        None,
    )
}

fn source_run_certification_registry() -> mfm_certify::CertificationRegistry {
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

fn source_run_state_registry() -> mfm_program::StateRegistrySnapshot {
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

struct SourceRunPublicOutputs<'program, 'scope> {
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

struct CertifiedSourceRunAuthority {
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

fn certified_source_run_authority(
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

fn source_run_program_draft(
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

fn certified_lifecycle_source_cell(
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

fn source_run_identity_material(spec_hash: &SpecHash) -> events::RunIdentityMaterialV1 {
    events::RunIdentityMaterialV1 {
        certified_spec_hash: spec_hash.clone(),
        trust_scope_id: TrustScopeId::new("mfm.trust_scope.v1:30303030303030303030303030303030")
            .expect("trust scope"),
        distinct_run_key_digest: Some(content_digest(0x30)),
    }
}

fn store_artifact_evidence_for_bytes(
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

fn source_spec_artifact_evidence(bytes: &[u8]) -> store::ArtifactEvidenceRef {
    store_artifact_evidence_for_bytes(
        bytes,
        events::ArtifactRole::TypedExecutionSpec,
        spec::MediaType::new(spec::MEDIA_TYPE).expect("spec media type"),
        Some(spec::typed_execution_spec_schema_id().expect("typed spec schema")),
        None,
        None,
    )
}

fn source_certificate_artifact_evidence(bytes: &[u8]) -> store::ArtifactEvidenceRef {
    store_artifact_evidence_for_bytes(
        bytes,
        events::ArtifactRole::TypedSpecCertificate,
        spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).expect("certificate media type"),
        Some(mfm_certify::typed_spec_certificate_schema_id().expect("certificate schema")),
        None,
        None,
    )
}

fn source_value_artifact_evidence<T>(
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

fn lifecycle_ref_from_store_artifact(
    evidence: &store::ArtifactEvidenceRef,
) -> LifecycleArtifactEvidenceRef {
    LifecycleArtifactEvidenceRef::new(
        evidence.artifact_id.clone(),
        evidence.digest.clone(),
        evidence.byte_len,
        evidence.schema_id.clone(),
        evidence.semantic_type_id.clone(),
    )
}

fn run_artifact_ref_from_store(
    evidence: &store::ArtifactEvidenceRef,
) -> events::RunArtifactEvidenceRef {
    events::RunArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        role: evidence.artifact_role,
        schema_id: evidence.schema_id.clone(),
        semantic_type_id: evidence.semantic_type_id.clone(),
        content_digest: evidence.digest.clone(),
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
    }
}

async fn source_run_stream_artifact<T>(
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
        entry_point: events::EntryPointLaunchEvidence {
            resolved_op_id: events::EntryPointOpId::new("mfm.test:source_run")
                .expect("entry point"),
            entry_point_registry_digest: content_digest(0x61),
        },
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
    (artifact_ref_for_raw_bytes(&bytes), bytes, terminal_event_id)
}

struct SourceRunCommitAppend<'a> {
    store: &'a store::AsyncInMemoryRunStore,
    run_id: &'a RunId,
    expected_next_seq: store::StreamSeq,
    commit_key: &'static str,
    payloads: Vec<events::KernelEventPayload>,
    required_artifacts: Vec<store::ArtifactEvidenceRef>,
    artifact_bytes: Vec<(Vec<u8>, store::ArtifactEvidenceRef)>,
    preconditions: store::CommitPreconditions,
}

async fn append_source_run_commit(input: SourceRunCommitAppend<'_>) {
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

async fn source_run_import_with_authority<T>(
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
    let value_ref = artifact_ref_for_bytes::<T>(&value_bytes);
    let source_value_store_artifact = source_value_artifact_evidence::<T>(
        &value_bytes,
        authority.source_producer_node.node_id.clone(),
    );
    let (stream_ref, stream_bytes, terminal_event_id) =
        source_run_stream_artifact::<T>(&authority, &value_bytes, source_value_store_artifact)
            .await;
    evidence.source_terminal_cell_or_output_event_ref =
        mfm_evm_contract_model::LifecycleEventIdRef::from(terminal_event_id);
    mutate_evidence(&mut evidence);
    let source_spec_ref = lifecycle_ref_from_store_artifact(&authority.source_spec_artifact);
    let certificate_ref = lifecycle_ref_from_store_artifact(&authority.certificate_artifact);
    let import = ImportDeployedSpec::FromMfmRun {
        source,
        evidence: ImportFromMfmRunEvidence {
            source_spec_artifact_ref: source_spec_ref.clone(),
            source_spec_certificate_ref: certificate_ref.clone(),
            source_run_stream_ref: stream_ref.clone(),
            source_value_artifact_ref_or_inline_canonical_value: value_ref.clone(),
            ..evidence
        },
    };
    (
        import,
        StaticRetainedArtifacts {
            artifacts: vec![
                (source_spec_ref, authority.source_spec_bytes),
                (certificate_ref, authority.certificate_bytes),
                (stream_ref, stream_bytes),
                (value_ref, value_bytes),
            ],
        },
    )
}

fn source_run_import_with_retargeted_source_value_payload(
    import: ImportDeployedSpec,
    mut artifacts: StaticRetainedArtifacts,
    payload: serde_json::Value,
) -> (ImportDeployedSpec, StaticRetainedArtifacts) {
    let raw_json = serde_json::to_string(&payload).expect("payload json");
    let raw_bytes = PlainCanonicalJsonBytes::from_json_str(&raw_json)
        .expect("payload canonical json")
        .to_vec();
    let raw_ref = artifact_ref_for_raw_bytes(&raw_bytes);
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
    evidence.source_value_artifact_ref_or_inline_canonical_value = raw_ref.clone();
    evidence.import_policy_digest = digest_for_value(&source).expect("import policy digest");

    artifacts.artifacts[3] = (raw_ref, raw_bytes);
    (
        ImportDeployedSpec::FromMfmRun { source, evidence },
        artifacts,
    )
}

fn deploy_action(chain_id: u64) -> ValidatedConfig<DeployAction> {
    deploy_action_for_style("eip1559", chain_id)
}

fn deploy_action_for_style(style: &str, chain_id: u64) -> ValidatedConfig<DeployAction> {
    ValidatedConfig::new(
        serde_json::from_value(json!({
            "signer": {
                "signer_ref": "deployer",
                "expected_signer_address": expected_test_signer_address_for_chain(style, chain_id),
            },
            "transaction": match style {
                "legacy" => json!({"style": "legacy", "gas_price": "7"}),
                _ => json!({"style": "eip1559", "max_fee_per_gas": "11", "max_priority_fee_per_gas": "3"}),
            },
        }))
        .expect("deploy action"),
    )
    .expect("valid deploy action")
}

fn configure_action(calls: serde_json::Value) -> ConfigureAction {
    serde_json::from_value(json!({
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": "0x0000000000000000000000000000000000000000",
        },
        "calls": calls,
    }))
    .expect("configure action")
}

fn contract_artifact_config() -> ContractArtifactConfig {
    serde_json::from_value(artifact_json()).expect("artifact config")
}

struct DeployPreparationFixture {
    providers: TestEvmProviders,
    action: ValidatedConfig<DeployAction>,
    context: mfm_program::CertifiedContext<EvmContractContext>,
    artifact: ContractArtifactConfig,
    intent: ContextContractDeployIntent,
}

type PreparedDeployFixture = (DeployPreparationFixture, PreparedContractMutation);

impl DeployPreparationFixture {
    fn new(style: &str) -> Self {
        let providers = TestEvmProviders::preparation();
        let action = deploy_action_for_style(style, 1);
        let context = certified_contract_context("ethereum-mainnet", 1);
        let artifact = contract_artifact_config();
        let intent = ContextBoundDeployContractState::new(action.clone())
            .expect("state")
            .prepare_intent(&(), &context)
            .expect("intent");
        Self {
            providers,
            action,
            context,
            artifact,
            intent,
        }
    }

    fn adapter(&self) -> EvmContractLifecycleAdapter<'_> {
        adapter(&self.providers)
    }

    async fn prepare(&self) -> PreparedContractMutation {
        self.adapter()
            .prepare_context_deploy_invocation(
                &self.action,
                &self.context,
                &self.artifact,
                &self.intent,
            )
            .await
            .expect("prepared")
    }

    fn reconstruct_with_hash(
        &self,
        prepared: &PreparedContractMutation,
        expected_transaction_hash: &str,
    ) -> PreparedContractMutation {
        let mut evidence = prepared.evidence().clone();
        evidence.transactions[0].expected_transaction_hash = expected_transaction_hash.to_owned();
        self.adapter()
            .reconstruct_context_deploy_invocation(
                &self.action,
                &self.context,
                &self.artifact,
                &self.intent,
                &evidence,
            )
            .expect("reconstructed")
    }
}

async fn prepared_deploy_fixture(style: &str) -> PreparedDeployFixture {
    let fixture = DeployPreparationFixture::new(style);
    let prepared = fixture.prepare().await;
    (fixture, prepared)
}

struct TestEvmProviders {
    mode: TestEvmProviderMode,
    submit_count: Arc<Mutex<u32>>,
    receipt_failure_reads: Arc<Mutex<u32>>,
}

#[derive(Clone, Copy)]
enum TestEvmProviderMode {
    Preparation,
    Recovery {
        receipt_mode: RecoveryReceiptMode,
        pending_nonce: u64,
        occupancy_mode: RecoveryOccupancyMode,
    },
    SubmitHashMismatch {
        returned_hash: B256,
    },
    ReceiptFailure,
    Finality,
    FinalityMismatchedEvidence,
}

impl TestEvmProviders {
    fn preparation() -> Self {
        Self::new(TestEvmProviderMode::Preparation)
    }

    fn recovery(
        receipt_mode: RecoveryReceiptMode,
        pending_nonce: u64,
        occupancy_mode: RecoveryOccupancyMode,
        submit_count: Arc<Mutex<u32>>,
    ) -> Self {
        Self {
            mode: TestEvmProviderMode::Recovery {
                receipt_mode,
                pending_nonce,
                occupancy_mode,
            },
            submit_count,
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }

    fn receipt_failure(reads: Arc<Mutex<u32>>) -> Self {
        Self {
            mode: TestEvmProviderMode::ReceiptFailure,
            submit_count: Arc::new(Mutex::new(0)),
            receipt_failure_reads: reads,
        }
    }

    fn finality() -> Self {
        Self::new(TestEvmProviderMode::Finality)
    }

    fn finality_mismatched_evidence() -> Self {
        Self::new(TestEvmProviderMode::FinalityMismatchedEvidence)
    }

    fn submit_hash_mismatch(returned_hash: B256, submit_count: Arc<Mutex<u32>>) -> Self {
        Self {
            mode: TestEvmProviderMode::SubmitHashMismatch { returned_hash },
            submit_count,
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }

    fn new(mode: TestEvmProviderMode) -> Self {
        Self {
            mode,
            submit_count: Arc::new(Mutex::new(0)),
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }
}

fn runtime_from_provider(provider: TestEvmProviders) -> EvmContractRuntime {
    let provider = Arc::new(provider);
    let evm: Arc<dyn EvmContractProvider> = provider.clone();
    let signer: Arc<dyn SigningProvider> = provider;
    EvmContractRuntime::new(evm, signer)
}

type ContractSubmissionDecision = SideEffectSubmissionDecision<
    ContractTransactionSubmissions,
    ContractTransactionSubmissions,
    ContractNotSubmittedProof,
    ContractTransactionSubmissions,
>;

async fn recover_with_provider(
    receipt_mode: RecoveryReceiptMode,
    pending_nonce: u64,
    occupancy_mode: RecoveryOccupancyMode,
    prepared: &PreparedContractMutation,
) -> (ContractSubmissionDecision, Arc<Mutex<u32>>) {
    let submit_count = Arc::new(Mutex::new(0));
    let runtime = runtime_from_provider(TestEvmProviders::recovery(
        receipt_mode,
        pending_nonce,
        occupancy_mode,
        Arc::clone(&submit_count),
    ));
    (recover_submission(&runtime, prepared).await, submit_count)
}

async fn recover_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
) -> ContractSubmissionDecision {
    submit_or_recover_contract_submission(
        runtime,
        prepared,
        SideEffectProtocolAction::SubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("recovered")
}

async fn start_with_submit_hash_mismatch(
    mismatched_hash: B256,
    prepared: &PreparedContractMutation,
) -> (ContractSubmissionDecision, Arc<Mutex<u32>>) {
    let submit_count = Arc::new(Mutex::new(0));
    let runtime = runtime_from_provider(TestEvmProviders::submit_hash_mismatch(
        mismatched_hash,
        Arc::clone(&submit_count),
    ));
    let decision = submit_or_recover_contract_submission(
        &runtime,
        prepared,
        SideEffectProtocolAction::StartPreparedAndSubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("submission decision");

    (decision, submit_count)
}

fn expected_transaction_hash(prepared: &PreparedContractMutation) -> &str {
    &prepared.evidence().transactions[0].expected_transaction_hash
}

fn assert_submit_count(submit_count: &Arc<Mutex<u32>>, expected: u32) {
    assert_eq!(*submit_count.lock().expect("submit count"), expected);
}

fn assert_observed_submission(
    decision: ContractSubmissionDecision,
    submit_count: &Arc<Mutex<u32>>,
    expected_count: u32,
    prepared: &PreparedContractMutation,
) {
    let SideEffectSubmissionDecision::Observed(submissions) = decision else {
        panic!("expected observed submission");
    };
    assert_submit_count(submit_count, expected_count);
    assert_eq!(
        submissions.transactions[0].transaction_hash,
        expected_transaction_hash(prepared)
    );
}

fn assert_unknown_submission(
    decision: ContractSubmissionDecision,
    submit_count: &Arc<Mutex<u32>>,
    prepared: &PreparedContractMutation,
) {
    let SideEffectSubmissionDecision::Unknown(evidence) = decision else {
        panic!("expected unknown submission");
    };
    assert_submit_count(submit_count, 0);
    assert_eq!(
        evidence.transactions[0].transaction_hash,
        expected_transaction_hash(prepared)
    );
}

fn assert_hash_mismatch_ambiguity(
    decision: ContractSubmissionDecision,
    submit_count: &Arc<Mutex<u32>>,
    expected_count: u32,
) -> ContractTransactionSubmissions {
    let SideEffectSubmissionDecision::Ambiguous {
        ambiguity_code,
        evidence,
    } = decision
    else {
        panic!("expected transaction-hash mismatch ambiguity");
    };
    assert_submit_count(submit_count, expected_count);
    assert_eq!(ambiguity_code.as_str(), "mfm.evm.transaction_hash_mismatch");
    evidence
}

async fn assert_recovery_observed(receipt_mode: RecoveryReceiptMode, expected_count: u32) {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let (decision, submit_count) =
        recover_with_provider(receipt_mode, 7, RecoveryOccupancyMode::Unknown, &prepared).await;
    assert_observed_submission(decision, &submit_count, expected_count, &prepared);
}

async fn assert_recovery_unknown(receipt_mode: RecoveryReceiptMode, pending_nonce: u64) {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let (decision, submit_count) = recover_with_provider(
        receipt_mode,
        pending_nonce,
        RecoveryOccupancyMode::Unknown,
        &prepared,
    )
    .await;
    assert_unknown_submission(decision, &submit_count, &prepared);
}

fn unexpected_evm_call<'a, T>(capability: &'static str) -> EvmCapabilityFuture<'a, T> {
    Box::pin(async move { panic!("unexpected test EVM capability call: {capability}") })
}

fn unexpected_signing_call<'a>() -> mfm_signing::SigningFuture<'a> {
    Box::pin(async { panic!("unexpected test signing provider call") })
}

impl EvmChainIdentityProvider for TestEvmProviders {
    fn chain_identity<'a>(
        &'a self,
        request: &'a EvmChainIdentityRequest,
    ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
        match self.mode {
            TestEvmProviderMode::Preparation
            | TestEvmProviderMode::Recovery { .. }
            | TestEvmProviderMode::SubmitHashMismatch { .. } => Box::pin(async {
                Ok(EvmChainIdentityResponse {
                    evidence: evidence_for_guard(&request.guard),
                    chain_id: request.guard.expected_chain_id(),
                    client_version: Some("test-client".to_owned()),
                })
            }),
            _ => unexpected_evm_call("chain_identity"),
        }
    }
}

impl EvmBlockReadProvider for TestEvmProviders {
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        match self.mode {
            TestEvmProviderMode::Finality | TestEvmProviderMode::FinalityMismatchedEvidence => {
                let evidence = match self.mode {
                    TestEvmProviderMode::FinalityMismatchedEvidence => {
                        mismatched_evidence_for_guard(&request.guard)
                    }
                    _ => evidence_for_guard(&request.guard),
                };
                Box::pin(async move {
                    Ok(EvmBlockReadResponse {
                        evidence,
                        block_number: 64,
                        block_hash: B256::from([0x64; 32]),
                    })
                })
            }
            _ => unexpected_evm_call("read_block"),
        }
    }
}

impl EvmNonceReadProvider for TestEvmProviders {
    fn read_nonce<'a>(
        &'a self,
        request: &'a EvmNonceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
        match self.mode {
            TestEvmProviderMode::Preparation => Box::pin(async {
                Ok(EvmNonceReadResponse {
                    evidence: evidence_for_guard(&request.guard),
                    nonce: 7,
                })
            }),
            TestEvmProviderMode::Recovery { pending_nonce, .. } => {
                assert_eq!(request.block, EvmBlockSelector::Pending);
                Box::pin(async move {
                    Ok(EvmNonceReadResponse {
                        evidence: evidence_for_guard(&request.guard),
                        nonce: pending_nonce,
                    })
                })
            }
            _ => unexpected_evm_call("read_nonce"),
        }
    }
}

impl EvmFeeReadProvider for TestEvmProviders {
    fn read_fee<'a>(
        &'a self,
        request: &'a EvmFeeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
        match self.mode {
            TestEvmProviderMode::Preparation => Box::pin(async {
                Ok(EvmFeeReadResponse {
                    evidence: evidence_for_guard(&request.guard),
                    base_fee_per_gas: Some(5),
                    priority_fee_per_gas: Some(3),
                    max_fee_per_gas: Some(11),
                    legacy_gas_price: Some(7),
                })
            }),
            _ => unexpected_evm_call("read_fee"),
        }
    }
}

impl EvmGasEstimateProvider for TestEvmProviders {
    fn estimate_gas<'a>(
        &'a self,
        request: &'a EvmGasEstimateRequest,
    ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
        match self.mode {
            TestEvmProviderMode::Preparation => Box::pin(async {
                Ok(EvmGasEstimateResponse {
                    evidence: evidence_for_guard(&request.guard),
                    gas_limit: 21_000,
                })
            }),
            _ => unexpected_evm_call("estimate_gas"),
        }
    }
}

impl EvmTransactionSubmitProvider for TestEvmProviders {
    fn submit_transaction<'a>(
        &'a self,
        request: &'a EvmTransactionSubmitRequest,
    ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmTransactionSubmitResponse> {
        match self.mode {
            TestEvmProviderMode::Recovery { .. } => {
                let submit_count = Arc::clone(&self.submit_count);
                let transaction_hash = request.signed_payload.transaction_hash();
                let evidence = evidence_for_guard(&request.guard);
                Box::pin(async move {
                    *submit_count.lock().expect("submit count") += 1;
                    Ok(mfm_evm_capabilities::EvmTransactionSubmitResponse {
                        evidence,
                        transaction_hash,
                    })
                })
            }
            TestEvmProviderMode::SubmitHashMismatch { returned_hash } => {
                let submit_count = Arc::clone(&self.submit_count);
                let evidence = evidence_for_guard(&request.guard);
                Box::pin(async move {
                    *submit_count.lock().expect("submit count") += 1;
                    Ok(mfm_evm_capabilities::EvmTransactionSubmitResponse {
                        evidence,
                        transaction_hash: returned_hash,
                    })
                })
            }
            _ => unexpected_evm_call("submit_transaction"),
        }
    }
}

impl EvmReceiptReadProvider for TestEvmProviders {
    fn read_receipt<'a>(
        &'a self,
        request: &'a EvmReceiptReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
        match self.mode {
            TestEvmProviderMode::Recovery { receipt_mode, .. } => {
                let transaction_hash = request.transaction_hash;
                let evidence = evidence_for_guard(&request.guard);
                Box::pin(async move {
                    match receipt_mode {
                        RecoveryReceiptMode::Landed => Ok(EvmReceiptReadResponse {
                            evidence,
                            transaction_hash,
                            block_number: 42,
                            status: true,
                        }),
                        RecoveryReceiptMode::Pending => Err(EvmCapabilityError::ReceiptPending),
                        RecoveryReceiptMode::ProviderFailure => {
                            Err(EvmCapabilityError::redacted_provider_failure("test rpc"))
                        }
                    }
                })
            }
            TestEvmProviderMode::ReceiptFailure => {
                let reads = Arc::clone(&self.receipt_failure_reads);
                Box::pin(async move {
                    let mut reads = reads
                        .lock()
                        .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
                    *reads += 1;
                    Err(EvmCapabilityError::redacted_provider_failure("test evm"))
                })
            }
            _ => unexpected_evm_call("read_receipt"),
        }
    }
}

impl EvmNonceOccupancyReadProvider for TestEvmProviders {
    fn read_nonce_occupancy<'a>(
        &'a self,
        request: &'a EvmNonceOccupancyReadRequest,
    ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmNonceOccupancyReadResponse> {
        match self.mode {
            TestEvmProviderMode::Recovery { occupancy_mode, .. } => {
                let nonce = request.nonce;
                let evidence = evidence_for_guard(&request.guard);
                Box::pin(async move {
                    match occupancy_mode {
                        RecoveryOccupancyMode::Unknown => {
                            Ok(mfm_evm_capabilities::EvmNonceOccupancyReadResponse {
                                evidence,
                                outcome: EvmNonceOccupancy::Unknown,
                            })
                        }
                        RecoveryOccupancyMode::Occupied { transaction_hash } => {
                            Ok(mfm_evm_capabilities::EvmNonceOccupancyReadResponse {
                                evidence,
                                outcome: EvmNonceOccupancy::Occupied {
                                    transaction_hash,
                                    block_number: Some(43 + nonce),
                                },
                            })
                        }
                    }
                })
            }
            _ => unexpected_evm_call("read_nonce_occupancy"),
        }
    }
}

impl EvmCallReadProvider for TestEvmProviders {
    fn read_call<'a>(
        &'a self,
        _request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
        unexpected_evm_call("read_call")
    }
}

impl EvmCodeReadProvider for TestEvmProviders {
    fn read_code<'a>(
        &'a self,
        request: &'a EvmCodeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCodeReadResponse> {
        Box::pin(async {
            let code = vec![0x60, 0x00];
            Ok(EvmCodeReadResponse {
                evidence: evidence_for_guard(&request.guard),
                code_hash: keccak256(&code),
                code,
            })
        })
    }
}

impl EvmLogsReadProvider for TestEvmProviders {
    fn read_logs<'a>(
        &'a self,
        _request: &'a EvmLogsReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
        unexpected_evm_call("read_logs")
    }
}

impl SigningProvider for TestEvmProviders {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> mfm_signing::SigningFuture<'a> {
        match self.mode {
            TestEvmProviderMode::Preparation
            | TestEvmProviderMode::Recovery { .. }
            | TestEvmProviderMode::SubmitHashMismatch { .. } => {
                let result = test_signing_result(request);
                Box::pin(async move { result })
            }
            _ => unexpected_signing_call(),
        }
    }
}

fn test_signature_bytes() -> SignatureBytes {
    SignatureBytes::new(
        hex_to_bytes(
            "0x48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353\
             efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b",
        )
        .expect("signature hex"),
    )
    .expect("signature bytes")
}

fn test_signing_result(request: &SigningRequest) -> mfm_signing::Result<SigningResult> {
    let signature = test_signature_bytes();
    let primitive = primitive_signature_from_bytes(&signature)
        .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
    let recovered = recover_signing_address(B256::from(*request.digest().as_bytes()), primitive)
        .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
    let identity = PublicSigningIdentity::new(
        request.algorithm().clone(),
        None,
        Some(format!("{recovered:?}")),
    )?;
    SigningResult::for_request(request, identity, signature)
}

fn expected_test_signer_address(style: &str) -> String {
    expected_test_signer_address_for_chain(style, 1)
}

fn expected_test_signer_address_for_chain(style: &str, chain_id: u64) -> String {
    let signer_ref = SignerRef::new("deployer").expect("signer ref");
    let expected_from = Address::from([0_u8; 20]);
    let data = vec![0x60, 0x00];
    let request = match style {
        "legacy" => EvmSigningRequest::legacy(
            signer_ref,
            LegacyTxToSign {
                to: None,
                value_wei: 0,
                chain_id,
                nonce: 7,
                gas_price_wei: 7,
                gas_limit: 21_000,
                data,
            },
            expected_from,
        ),
        _ => EvmSigningRequest::eip1559(
            signer_ref,
            Eip1559TxToSign {
                to: None,
                value_wei: 0,
                chain_id,
                nonce: 7,
                max_fee_per_gas: 11,
                max_priority_fee_per_gas: 3,
                gas_limit: 21_000,
                data,
            },
            expected_from,
        ),
    }
    .expect("signing request");
    let primitive = primitive_signature_from_bytes(&test_signature_bytes()).expect("signature");
    let recovered =
        recover_signing_address(request.signing_hash(), primitive).expect("recovered address");
    format!("{recovered:?}")
}

fn adapter(providers: &TestEvmProviders) -> EvmContractLifecycleAdapter<'_> {
    EvmContractLifecycleAdapter::new(
        EvmContractMutationProviders::from_evm_and_signer(providers, providers),
        EvmContractReadProviders::from_provider(providers),
    )
}

#[test]
fn executable_identity_summary_matches_golden() {
    assert_eq!(
        executable_identity_summary([SIDE_EFFECT_FACTORY, READ_FACTORY]),
        [
            "factory=apply_side_effect;cargo_digest=content:sha256-jcs-v1:685edb3e7cd5decfb0f17613a76568a2c5c572024d6db20bf0803d61ee0657e4;binary_digest=content:sha256-jcs-v1:4a583ef5eb9bb2ce3f01767ddffc34e0744290d029bde3d69967b272a74f302f;nix_derivation=false;nix_output=false",
            "factory=read_external;cargo_digest=content:sha256-jcs-v1:685edb3e7cd5decfb0f17613a76568a2c5c572024d6db20bf0803d61ee0657e4;binary_digest=content:sha256-jcs-v1:4a583ef5eb9bb2ce3f01767ddffc34e0744290d029bde3d69967b272a74f302f;nix_derivation=false;nix_output=false",
        ]
    );
}

#[test]
fn event_block_selector_preserves_supported_tags() {
    assert_eq!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Earliest,
            }),
            false
        )
        .expect("earliest selector"),
        EvmBlockSelector::Number(0)
    );
    assert_eq!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Latest,
            }),
            true
        )
        .expect("latest selector"),
        EvmBlockSelector::Latest
    );
    assert_eq!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Pending,
            }),
            true
        )
        .expect("pending selector"),
        EvmBlockSelector::Pending
    );
    assert_eq!(
        block_selector(Some(&BlockSelector::Number { number: 42 }), false)
            .expect("number selector"),
        EvmBlockSelector::Number(42)
    );
}

#[test]
fn event_block_selector_rejects_unsupported_tags() {
    assert!(matches!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Safe,
            }),
            true
        ),
        Err(EvmContractAdapterError::UnsupportedBlockTag)
    ));
    assert!(matches!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Finalized,
            }),
            false
        ),
        Err(EvmContractAdapterError::UnsupportedBlockTag)
    ));
}

#[test]
fn configure_action_transaction_inputs_accepts_empty_calls_without_artifact() {
    let action = configure_action(json!([]));

    let inputs = configure_action_transaction_inputs(
        &action,
        None,
        "0x000000000000000000000000000000000000beef",
    )
    .expect("empty configure calls should not require artifact");

    assert!(inputs.is_empty());
}

#[test]
fn configure_action_transaction_inputs_rejects_calls_without_artifact() {
    let action = configure_action(json!([
        {
            "function": "configure",
        }
    ]));

    let error = match configure_action_transaction_inputs(
        &action,
        None,
        "0x000000000000000000000000000000000000beef",
    ) {
        Ok(_) => panic!("non-empty configure calls require artifact"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        EvmContractAdapterError::MissingContractArtifact
    ));
}

#[tokio::test]
async fn deploy_preparation_defaults_to_eip1559_contract_creation() {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;

    assert_eq!(prepared.evidence().transactions.len(), 1);
    assert_eq!(
        prepared.evidence().transactions[0].style,
        PreparedContractTransactionStyle::Eip1559
    );
    assert_eq!(prepared.evidence().transactions[0].to_address, None);
    assert_eq!(
        prepared.signing_requests()[0].style(),
        SigningTransactionStyle::Eip1559
    );
}

#[tokio::test]
async fn deploy_preparation_supports_legacy_contract_creation() {
    let (_fixture, prepared) = prepared_deploy_fixture("legacy").await;

    assert_eq!(
        prepared.evidence().transactions[0].style,
        PreparedContractTransactionStyle::Legacy
    );
    assert_eq!(
        prepared.signing_requests()[0].style(),
        SigningTransactionStyle::Legacy
    );
}

#[tokio::test]
async fn context_deploy_preparation_routes_from_certified_context() {
    let providers = TestEvmProviders::preparation();
    let adapter = adapter(&providers);
    let context = certified_contract_context("reth-dev", 31337);
    let action = deploy_action(31337);
    let state = ContextBoundDeployContractState::new(action.clone()).expect("state");
    let intent = state.prepare_intent(&(), &context).expect("intent");
    let artifact = contract_artifact_config();

    let prepared = adapter
        .prepare_context_deploy_invocation(&action, &context, &artifact, &intent)
        .await
        .expect("prepared");

    assert_eq!(prepared.evidence().network_id, "reth-dev");
    assert_eq!(prepared.evidence().expected_chain_id, 31337);
    assert_eq!(
        prepared.evidence().context_ref.as_context_ref(),
        context.context_ref()
    );
    assert_eq!(
        prepared.evidence().resource_stage,
        ContractLifecycleStage::Deployed
    );
    assert_eq!(
        prepared.evidence().transactions[0].chain_id,
        31337,
        "transaction signing chain id must come from certified context"
    );
    assert!(prepared
        .evidence()
        .evm_network_context_ref
        .starts_with("content:sha256-jcs-v1:"));
}

#[tokio::test]
async fn source_run_import_rejects_missing_retained_evidence() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let value_digest = digest_for_value(&source_value).expect("value digest");
    let import: ImportDeployedSpec = serde_json::from_value(source_run_import_json::<
        DeployedContractInstance,
    >(
        &context, "deployed", &value_digest
    ))
    .expect("source-run import");
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
        evm,
        source_run_certification_registry(),
    );
    let error = runtime
        .import_deployed(&import, &context, &MissingRetainedArtifacts)
        .await
        .expect_err("missing proof must fail closed");

    assert!(matches!(
        error,
        EvmContractAdapterError::SourceRunImportEvidence(_)
    ));
}

#[tokio::test]
async fn source_run_import_accepts_retained_export_authority() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let (import, artifacts) = source_run_import_with_authority(
        &context,
        ContractLifecycleStage::Deployed,
        &source_value,
        |_| {},
    )
    .await;
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
        evm,
        source_run_certification_registry(),
    );

    let imported = runtime
        .import_deployed(&import, &context, &artifacts)
        .await
        .expect("source-run import");

    assert_eq!(imported.context_ref.as_context_ref(), context.context_ref());
    assert_eq!(imported.address, source_value.address);
    assert_eq!(imported.deploy_evidence.len(), 4);
}

#[tokio::test]
async fn source_run_import_without_registry_authority_fails_closed() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let (import, artifacts) = source_run_import_with_authority(
        &context,
        ContractLifecycleStage::Deployed,
        &source_value,
        |_| {},
    )
    .await;
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new(evm);

    let error = runtime
        .import_deployed(&import, &context, &artifacts)
        .await
        .expect_err("source-run import without registry authority");

    assert!(matches!(
        error,
        EvmContractAdapterError::SourceRunImportEvidence(_)
    ));
}

#[tokio::test]
async fn source_run_import_rejects_domain_local_export_certificate() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let (import, mut artifacts) = source_run_import_with_authority(
        &context,
        ContractLifecycleStage::Deployed,
        &source_value,
        |_| {},
    )
    .await;
    let ImportDeployedSpec::FromMfmRun {
        source,
        mut evidence,
    } = import
    else {
        panic!("expected source-run import");
    };
    let old_certificate = json!({
        "certificate_version": 1,
        "source_run_id": source.source_run_id.clone(),
        "source_spec_hash": source.source_spec_hash.clone(),
        "committed_stream_digest": content_digest_str(0x5c),
        "allowed_producer_descriptor_ids": [evidence.source_producer_descriptor_id.clone()],
        "allowed_context_descriptor_ids": [evidence.source_context_descriptor_id.clone()],
        "allowed_schema_ids": [evidence.source_cell_schema_id.clone()],
        "allowed_semantic_type_ids": [evidence.source_cell_semantic_type_id.clone()],
    });
    let old_certificate_bytes = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&old_certificate).expect("old certificate json"),
    )
    .expect("old certificate canonical")
    .to_vec();
    let old_certificate_ref = artifact_ref_for_raw_bytes(&old_certificate_bytes);
    artifacts.artifacts[1] = (old_certificate_ref.clone(), old_certificate_bytes);
    evidence.source_spec_certificate_ref = old_certificate_ref;
    let import = ImportDeployedSpec::FromMfmRun { source, evidence };
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
        evm,
        source_run_certification_registry(),
    );

    let error = runtime
        .import_deployed(&import, &context, &artifacts)
        .await
        .expect_err("domain-local certificate must not authorize imports");

    assert!(matches!(
        error,
        EvmContractAdapterError::SourceRunImportEvidence(_)
    ));
}

#[tokio::test]
async fn source_run_import_rejects_public_projection_and_raw_value_payloads() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let payloads = [
        (
            "public json",
            json!({
                "deployed": serde_json::to_value(&source_value).expect("source value json"),
            }),
        ),
        (
            "projection row",
            json!({
                "cell_id": cell_id_str(0x32),
                "content_digest": content_digest_str(0x33),
                "public_field_path": "deployed",
            }),
        ),
        (
            "raw payload",
            json!({
                "address": "0x1111111111111111111111111111111111111111",
                "context_ref": context.context_ref().to_string(),
            }),
        ),
    ];

    for (label, payload) in payloads {
        let (import, artifacts) = source_run_import_with_authority(
            &context,
            ContractLifecycleStage::Deployed,
            &source_value,
            |_| {},
        )
        .await;
        let (import, artifacts) =
            source_run_import_with_retargeted_source_value_payload(import, artifacts, payload);
        let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
        let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
            evm,
            source_run_certification_registry(),
        );

        let error = runtime
            .import_deployed(&import, &context, &artifacts)
            .await
            .expect_err(label);

        assert!(
            matches!(error, EvmContractAdapterError::SourceRunImportEvidence(_)),
            "{label} rejected with unexpected error: {error:?}"
        );
    }
}

#[tokio::test]
async fn source_run_import_rejects_stream_without_claimed_terminal_event() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let (import, artifacts) = source_run_import_with_authority(
        &context,
        ContractLifecycleStage::Deployed,
        &source_value,
        |evidence| {
            evidence.source_terminal_cell_or_output_event_ref =
                mfm_evm_contract_model::LifecycleEventIdRef::from(EventId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_with(0x7f),
                ));
        },
    )
    .await;
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
        evm,
        source_run_certification_registry(),
    );

    let error = runtime
        .import_deployed(&import, &context, &artifacts)
        .await
        .expect_err("tampered terminal event must fail");

    assert!(matches!(
        error,
        EvmContractAdapterError::SourceRunImportEvidence(_)
    ));
}

#[tokio::test]
async fn external_adoption_records_replayable_code_evidence() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let import: ImportDeployedSpec = serde_json::from_value(json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000beef",
            "provenance_label": "audited-external"
        }
    }))
    .expect("external adoption import");
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new(evm);

    let imported = runtime
        .import_deployed(&import, &context, &MissingRetainedArtifacts)
        .await
        .expect("external adoption");
    let evidence = imported
        .external_adoption_evidence
        .expect("external adoption evidence");

    assert_eq!(evidence.context_ref.as_context_ref(), context.context_ref());
    assert_eq!(evidence.resource_stage, ContractLifecycleStage::Deployed);
    assert_eq!(evidence.observed_chain_id, 1);
    let code = evidence.code_read_evidence.expect("code-read evidence");
    assert_eq!(
        code.address.as_str(),
        "0x000000000000000000000000000000000000beef"
    );
    assert_eq!(code.observed_code_byte_len, 2);
    assert_eq!(code.source.observed_chain_id, 1);
    assert_eq!(code.source.network_id, "ethereum-mainnet");
}

#[tokio::test]
async fn context_prepared_reconstruction_rejects_mismatched_context_ref() {
    let providers = TestEvmProviders::preparation();
    let adapter = adapter(&providers);
    let context = certified_contract_context("reth-dev", 31337);
    let action = deploy_action(31337);
    let state = ContextBoundDeployContractState::new(action.clone()).expect("state");
    let intent = state.prepare_intent(&(), &context).expect("intent");
    let artifact = contract_artifact_config();
    let prepared = adapter
        .prepare_context_deploy_invocation(&action, &context, &artifact, &intent)
        .await
        .expect("prepared");
    let mut evidence = prepared.evidence().clone();
    evidence.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(0x99),
    ));

    assert!(matches!(
        adapter.reconstruct_context_deploy_invocation(
            &action, &context, &artifact, &intent, &evidence
        ),
        Err(EvmContractAdapterError::InvalidPreparedInvocation)
    ));
}

#[tokio::test]
async fn prepared_invocation_evidence_excludes_live_and_secret_surfaces() {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;

    ensure_prepared_invocation_public(prepared.evidence()).expect("public evidence");
    let evidence = prepared.evidence();
    let rendered_value = serde_json::to_value(evidence).expect("json value");
    assert_eq!(
        sorted_json_keys(&rendered_value),
        vec![
            "context_ref",
            "evm_network_context_ref",
            "expected_chain_id",
            "expected_signer_address",
            "max_receipt_polls",
            "network_id",
            "phase",
            "poll_interval_ms",
            "prepared_version",
            "resource_stage",
            "signer_ref",
            "transactions",
        ]
    );
    assert_eq!(evidence.signer_ref, "deployer");
    assert_eq!(
        evidence.expected_signer_address,
        expected_test_signer_address("eip1559")
    );
    let transaction = evidence.transactions.first().expect("transaction");
    let rendered_transaction = rendered_value["transactions"][0].clone();
    assert_eq!(
        sorted_json_keys(&rendered_transaction),
        vec![
            "chain_id",
            "data_digest",
            "data_len",
            "expected_transaction_hash",
            "gas_limit",
            "gas_price",
            "index",
            "max_fee_per_gas",
            "max_priority_fee_per_gas",
            "nonce",
            "signing_digest",
            "style",
            "to_address",
            "value_wei",
        ]
    );
    assert_eq!(transaction.nonce, 7);
    assert_eq!(transaction.gas_limit, 21_000);
    assert_eq!(transaction.max_fee_per_gas.as_deref(), Some("11"));
    assert_eq!(transaction.max_priority_fee_per_gas.as_deref(), Some("3"));
    assert!(transaction
        .data_digest
        .starts_with("content:sha256-jcs-v1:"));
    assert_eq!(transaction.data_len, 2);
    assert!(transaction.signing_digest.starts_with("0x"));
    assert_eq!(transaction.signing_digest.len(), 66);
    assert!(transaction.expected_transaction_hash.starts_with("0x"));
    assert_eq!(transaction.expected_transaction_hash.len(), 66);
}

#[tokio::test]
async fn prepared_invocation_reconstruction_preserves_submission_anchor() {
    let (fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let reconstructed = fixture
        .adapter()
        .reconstruct_context_deploy_invocation(
            &fixture.action,
            &fixture.context,
            &fixture.artifact,
            &fixture.intent,
            prepared.evidence(),
        )
        .expect("reconstructed");

    assert_eq!(reconstructed.evidence(), prepared.evidence());
    assert_eq!(
        reconstructed.evidence().transactions[0].expected_transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
}

#[tokio::test]
async fn submit_rejects_resigned_hash_mismatch_before_broadcast() {
    let (fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let reconstructed = fixture.reconstruct_with_hash(&prepared, MISMATCH_HASH);
    let adapter = fixture.adapter();

    assert!(matches!(
        adapter.submit_prepared(&reconstructed).await,
        Err(EvmContractAdapterError::TransactionHashMismatch)
    ));
}

#[tokio::test]
async fn recovery_observes_landed_anchor_without_resubmitting() {
    assert_recovery_observed(RecoveryReceiptMode::Landed, 0).await;
}

#[tokio::test]
async fn recovery_rebroadcasts_unlanded_anchor_after_resign_match() {
    assert_recovery_observed(RecoveryReceiptMode::Pending, 1).await;
}

#[tokio::test]
async fn recovery_does_not_rebroadcast_when_resign_hash_mismatches_anchor() {
    let (fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let reconstructed = fixture.reconstruct_with_hash(&prepared, MISMATCH_HASH);
    let (decision, submit_count) = recover_with_provider(
        RecoveryReceiptMode::Pending,
        7,
        RecoveryOccupancyMode::Unknown,
        &reconstructed,
    )
    .await;

    let evidence = assert_hash_mismatch_ambiguity(decision, &submit_count, 0);
    assert_eq!(evidence.transactions[0].transaction_hash, MISMATCH_HASH);
}

#[tokio::test]
async fn fresh_submit_records_ambiguity_when_provider_returns_mismatched_hash() {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let mismatched_hash = B256::from([0x11; 32]);
    let (decision, submit_count) =
        start_with_submit_hash_mismatch(mismatched_hash, &prepared).await;

    let evidence = assert_hash_mismatch_ambiguity(decision, &submit_count, 1);
    assert_eq!(
        evidence.transactions[0].transaction_hash,
        expected_transaction_hash(&prepared)
    );
    assert_ne!(
        evidence.transactions[0].transaction_hash,
        format!("{mismatched_hash:?}")
    );
}

#[tokio::test]
async fn recovery_keeps_advanced_nonce_unknown_without_occupancy_proof() {
    assert_recovery_unknown(RecoveryReceiptMode::Pending, 8).await;
}

#[tokio::test]
async fn recovery_proves_not_submitted_when_foreign_transaction_occupies_nonce() {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let occupying_hash = "0x2222222222222222222222222222222222222222222222222222222222222222"
        .parse::<B256>()
        .expect("occupying hash");
    let (decision, submit_count) = recover_with_provider(
        RecoveryReceiptMode::Pending,
        8,
        RecoveryOccupancyMode::Occupied {
            transaction_hash: occupying_hash,
        },
        &prepared,
    )
    .await;

    let SideEffectSubmissionDecision::NotSubmitted(proof) = decision else {
        panic!("expected not-submitted proof");
    };
    assert_submit_count(&submit_count, 0);
    assert_eq!(
        proof.expected_transaction_hash,
        expected_transaction_hash(&prepared)
    );
    assert_eq!(
        proof.occupying_transaction_hash,
        "0x2222222222222222222222222222222222222222222222222222222222222222"
    );
    assert_eq!(proof.nonce, prepared.evidence().transactions[0].nonce);
    assert_eq!(proof.evidence_chain_id, 1);
}

#[tokio::test]
async fn recovery_records_unknown_on_transient_anchor_read_failure() {
    assert_recovery_unknown(RecoveryReceiptMode::ProviderFailure, 7).await;
}

type PreparedInvocationMutation = Box<dyn FnOnce(&mut PreparedContractInvocation)>;

#[test]
fn prepared_invocation_guard_rejects_malformed_public_contract() {
    let cases: Vec<PreparedInvocationMutation> = vec![
        Box::new(|prepared| prepared.prepared_version = 2),
        Box::new(|prepared| {
            prepared.expected_signer_address =
                "0x0F65FE9276BC9A24AE7083AE28E2660EF72DF99E".to_owned();
        }),
        Box::new(|prepared| prepared.transactions[0].index = 7),
        Box::new(|prepared| prepared.transactions[0].chain_id = 2),
        Box::new(|prepared| prepared.transactions[0].value_wei = "0x1".to_owned()),
        Box::new(|prepared| prepared.transactions[0].gas_price = Some("7".to_owned())),
        Box::new(|prepared| prepared.transactions[0].data_digest = "not-a-digest".to_owned()),
        Box::new(|prepared| prepared.transactions[0].signing_digest = "not-a-hash".to_owned()),
    ];

    for mutate in cases {
        let mut prepared = prepared_invocation_fixture();
        mutate(&mut prepared);
        assert!(matches!(
            ensure_prepared_invocation_public(&prepared),
            Err(EvmContractAdapterError::InvalidPreparedInvocation)
        ));
    }
}

#[test]
fn prepared_invocation_deserialization_rejects_unknown_public_fields() {
    let mut top_level = serde_json::to_value(prepared_invocation_fixture()).expect("json");
    top_level
        .as_object_mut()
        .expect("object")
        .insert("raw_transaction".to_owned(), serde_json::json!("0x01"));
    assert!(serde_json::from_value::<PreparedContractInvocation>(top_level).is_err());

    let mut nested = serde_json::to_value(prepared_invocation_fixture()).expect("json");
    nested["transactions"][0]
        .as_object_mut()
        .expect("transaction object")
        .insert("signature".to_owned(), serde_json::json!("0x01"));
    assert!(serde_json::from_value::<PreparedContractInvocation>(nested).is_err());
}

#[test]
fn context_nonce_resource_key_uses_network_context_ref_and_account() {
    let key = account_nonce_resource_key(
        &ContractNonceResourceScope {
            evm_network_context_ref:
                "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
        },
        "0x000000000000000000000000000000000000dead",
    )
    .expect("resource key");

    assert_eq!(
        key.as_str(),
        r#"{"account":"0x000000000000000000000000000000000000dead","evm_network_context_ref":"content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
    );
    assert!(!key.as_str().contains("\"chain_id\""));
}

fn prepared_invocation_fixture() -> PreparedContractInvocation {
    PreparedContractInvocation {
        prepared_version: 1,
        phase: ContractMutationPhase::Deploy,
        context_ref: mfm_values::ContextRefValue::from(ContextRef::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_with(0x44),
        )),
        evm_network_context_ref:
            "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
        resource_stage: ContractLifecycleStage::Deployed,
        network_id: "ethereum-mainnet".to_owned(),
        expected_chain_id: 1,
        signer_ref: "deployer".to_owned(),
        expected_signer_address: "0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e".to_owned(),
        transactions: vec![PreparedContractTransactionEvidence {
            index: 0,
            style: PreparedContractTransactionStyle::Eip1559,
            chain_id: 1,
            nonce: 7,
            to_address: None,
            value_wei: "0".to_owned(),
            gas_limit: 21_000,
            max_fee_per_gas: Some("11".to_owned()),
            max_priority_fee_per_gas: Some("3".to_owned()),
            gas_price: None,
            data_digest:
                "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            data_len: 2,
            signing_digest:
                "0x0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            expected_transaction_hash:
                "0x1111111111111111111111111111111111111111111111111111111111111111"
                    .to_owned(),
        }],
        poll_interval_ms: 1_000,
        max_receipt_polls: 10,
    }
}

fn wrong_context_prepared_invocation_fixture() -> PreparedContractInvocation {
    let mut prepared = prepared_invocation_fixture();
    prepared.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(0x92),
    ));
    prepared
}

fn certified_side_effect_context_for_prepared(
    prepared: &PreparedContractInvocation,
) -> replay::CertifiedSideEffectContext {
    replay::CertifiedSideEffectContext {
        node_context: spec::NodeContextSpec::Required {
            context_ref: prepared.context_ref.as_context_ref().clone(),
        },
        output_context: spec::CellContextSpec::Bound {
            context_ref: prepared.context_ref.as_context_ref().clone(),
            resource_kind: mfm_evm_contract_model::contract_instance_resource_kind().clone(),
            stage: context_stage_for_lifecycle_stage(prepared.resource_stage).clone(),
            producer: Box::new(spec::ContextProducerSpec {
                producer_descriptor_ids: vec![DescriptorId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_with(0x45),
                )],
                seed_producers_allowed: false,
            }),
        },
    }
}

fn replay_artifact_for_value<T>(
    value: &T,
    role: events::ArtifactRole,
) -> (store::ArtifactEvidenceRef, Vec<u8>)
where
    T: MfmValue + Serialize,
{
    let bytes = canonical_value_bytes(value);
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    (
        store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
            digest,
            byte_len: bytes.len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media type"),
            schema_id: Some(T::schema_id().expect("schema id")),
            semantic_type_id: Some(T::semantic_id().expect("semantic id")),
            producer_node_id: Some(node_id(0x46)),
            producer_seed_id: None,
            artifact_role: role,
        },
        bytes,
    )
}

fn replay_attempt_id() -> AttemptId {
    AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x47))
}

fn replay_scope_id() -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x48))
}

fn replay_pair_id() -> SideEffectPairId {
    SideEffectPairId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x49))
}

fn replay_ledger_key() -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new("evm-contract-replay-test").expect("ledger key")
}

fn replay_deploy_intent(prepared: &PreparedContractInvocation) -> ContextContractDeployIntent {
    ContextContractDeployIntent {
        intent_version: 1,
        transaction: ContextContractTransactionIntent {
            intent_version: 1,
            context_ref: prepared.context_ref.clone(),
            network_id: prepared.network_id.clone(),
            expected_chain_id: prepared.expected_chain_id,
            signer_ref: prepared.signer_ref.clone(),
            expected_signer_address: prepared.expected_signer_address.clone(),
            to_address: None,
            value_wei: Some("0".to_owned()),
            data_ref: None,
            transaction: EvmTransactionPolicy::new(
                ConfigTransactionStyle::Eip1559,
                Some(prepared.transactions[0].gas_limit),
                prepared.transactions[0].max_fee_per_gas.clone(),
                prepared.transactions[0].max_priority_fee_per_gas.clone(),
                None,
            )
            .expect("transaction policy"),
        },
        constructor_args_len: 0,
        contract_profile_id: "test-profile".to_owned(),
    }
}

fn replay_intent_evidence(
    prepared: &PreparedContractInvocation,
) -> replay::SideEffectIntentReplayEvidence {
    let intent = replay_deploy_intent(prepared);
    let (artifact, artifact_bytes) =
        replay_artifact_for_value(&intent, events::ArtifactRole::SideEffectIntent);
    let binding = evm_contract_lifecycle_adapter_binding().expect("adapter binding");
    replay::SideEffectIntentReplayEvidence {
        intent: side_effect::IntentPersisted {
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x4a)),
            node_id: node_id(0x46),
            scope_id: replay_scope_id(),
            attempt_id: replay_attempt_id(),
            ledger_key: replay_ledger_key(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: replay_pair_id(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            intent_schema_id: ContextContractDeployIntent::schema_id().expect("intent schema"),
            intent_hash: artifact.digest.clone(),
            intent_artifact_id: artifact.artifact_id.clone(),
            idempotency_input_schema_id: ContractTransactionIdempotency::schema_id()
                .expect("idempotency schema"),
            idempotency_input_hash: content_digest(0x4b),
            idempotency_key: events::IdempotencyKeyRef::new("evm-contract-replay-test")
                .expect("idempotency key"),
            capability_kind: EvmTransactionSubmitCapability::kind().expect("capability kind"),
            capability_version: EvmTransactionSubmitCapability::version()
                .expect("capability version"),
            adapter_kind: binding.adapter_kind().clone(),
            adapter_version: binding.adapter_version().clone(),
        },
        artifact,
        artifact_bytes,
    }
}

#[test]
fn replay_intent_rejects_prepared_transaction_count_mismatch() {
    let original = prepared_invocation_fixture();
    let intent = replay_intent_evidence(&original);
    let mut prepared = original.clone();
    let mut extra = prepared.transactions[0].clone();
    extra.index = 1;
    prepared.transactions.push(extra);

    assert!(verify_replay_intent_matches_prepared(&intent, &prepared).is_err());
}

#[test]
fn replay_intent_rejects_prepared_destination_mismatch() {
    let original = prepared_invocation_fixture();
    let intent = replay_intent_evidence(&original);
    let mut prepared = original;
    prepared.transactions[0].to_address =
        Some("0x000000000000000000000000000000000000beef".to_owned());

    assert!(verify_replay_intent_matches_prepared(&intent, &prepared).is_err());
}

#[test]
fn replay_intent_rejects_prepared_policy_mismatch() {
    let original = prepared_invocation_fixture();
    let intent = replay_intent_evidence(&original);
    let mut prepared = original;
    prepared.transactions[0].style = PreparedContractTransactionStyle::Legacy;
    prepared.transactions[0].gas_price = Some("9".to_owned());
    prepared.transactions[0].max_fee_per_gas = None;
    prepared.transactions[0].max_priority_fee_per_gas = None;

    assert!(verify_replay_intent_matches_prepared(&intent, &prepared).is_err());
}

#[test]
fn replay_prepared_transaction_data_must_match_certified_inputs() {
    let tx_inputs = vec![PreparedTransactionInput {
        to: None,
        value_wei: 0,
        data: vec![0x60, 0x00],
    }];
    let mut prepared = prepared_invocation_fixture();
    prepared.transactions[0].data_digest = digest_bytes(&tx_inputs[0].data).to_string();
    prepared.transactions[0].data_len = tx_inputs[0].data.len() as u64;

    verify_prepared_transaction_data_matches_inputs(&prepared, &tx_inputs)
        .expect("matching prepared transaction data");

    let mut wrong_digest = prepared.clone();
    wrong_digest.transactions[0].data_digest = digest_bytes(&[0x61, 0x00]).to_string();
    let error = verify_prepared_transaction_data_matches_inputs(&wrong_digest, &tx_inputs)
        .expect_err("mismatched data digest rejects");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut wrong_len = prepared;
    wrong_len.transactions[0].data_len += 1;
    let error = verify_prepared_transaction_data_matches_inputs(&wrong_len, &tx_inputs)
        .expect_err("mismatched data length rejects");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

fn replay_prepared_evidence(
    prepared: &PreparedContractInvocation,
) -> replay::PreparedInvocationReplayEvidence {
    let (artifact, artifact_bytes) =
        replay_artifact_for_value(prepared, events::ArtifactRole::PreparedInvocation);
    replay::PreparedInvocationReplayEvidence {
        prepared: side_effect::InvocationPrepared {
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x4a)),
            node_id: node_id(0x46),
            attempt_id: replay_attempt_id(),
            ledger_key: replay_ledger_key(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: replay_pair_id(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            claim_generation: 1,
            claim_fencing_token: side_effect::ClaimFencingToken::new("claim-token")
                .expect("claim token"),
            resource_key: None,
            prepared_artifact_id: Some(artifact.artifact_id.clone()),
            prepared_hash: Some(artifact.digest.clone()),
        },
        artifact,
        artifact_bytes,
    }
}

fn replay_submission_evidence(
    prepared: &PreparedContractInvocation,
) -> replay::SubmissionReplayEvidence {
    let submissions = prepared_anchor_submissions(prepared).expect("submissions");
    let (artifact, artifact_bytes) =
        replay_artifact_for_value(&submissions, events::ArtifactRole::Submission);
    replay::SubmissionReplayEvidence {
        submission: side_effect::SubmissionObserved {
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x4a)),
            node_id: node_id(0x46),
            attempt_id: replay_attempt_id(),
            ledger_key: replay_ledger_key(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: replay_pair_id(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            submission_schema_id: ContractTransactionSubmissions::schema_id()
                .expect("submission schema"),
            submission_hash: artifact.digest.clone(),
            submission_artifact_id: artifact.artifact_id.clone(),
        },
        artifact,
        artifact_bytes,
    }
}

fn replay_receipt_evidence(prepared: &PreparedContractInvocation) -> replay::ReceiptReplayEvidence {
    let receipt = ContractDeployReceipt {
        receipt_version: 1,
        context_ref: prepared.context_ref.clone(),
        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
        resource_stage: ContractLifecycleStage::Deployed,
        contract_address: "0x000000000000000000000000000000000000beef".to_owned(),
        receipt: ContractTransactionReceipt {
            receipt_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: ContractLifecycleStage::Deployed,
            transaction_hash: prepared.transactions[0].expected_transaction_hash.clone(),
            block_number: 7,
            status: true,
            receipt_evidence: None,
        },
    };
    let (artifact, artifact_bytes) =
        replay_artifact_for_value(&receipt, events::ArtifactRole::Receipt);
    replay::ReceiptReplayEvidence {
        receipt: side_effect::ReceiptObserved {
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x4a)),
            node_id: node_id(0x46),
            attempt_id: replay_attempt_id(),
            ledger_key: replay_ledger_key(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: replay_pair_id(),
            pair_role: events::SideEffectPairRole::Verify,
            invocation_epoch: 1,
            receipt_schema_id: ContractDeployReceipt::schema_id().expect("receipt schema"),
            receipt_hash: artifact.digest.clone(),
            receipt_artifact_id: artifact.artifact_id.clone(),
            replay_verifier_id: replay_verifier_id().expect("replay verifier id"),
            resource_touched_set: None,
        },
        artifact,
        artifact_bytes,
    }
}

fn receipt_polling_invocation(transaction_hash: &str) -> PreparedContractInvocation {
    let mut prepared = prepared_invocation_fixture();
    prepared.network_id = "reth-dev".to_owned();
    prepared.expected_chain_id = 31337;
    prepared.expected_signer_address = "0x0000000000000000000000000000000000000001".to_owned();
    prepared.poll_interval_ms = 25;
    prepared.max_receipt_polls = 3;

    let transaction = &mut prepared.transactions[0];
    transaction.chain_id = 31337;
    transaction.data_len = 0;
    transaction.signing_digest = transaction_hash.to_owned();
    transaction.expected_transaction_hash = transaction_hash.to_owned();
    prepared
}

fn finality_receipt() -> ContractTransactionReceipt {
    ContractTransactionReceipt {
        receipt_version: 1,
        context_ref: prepared_invocation_fixture().context_ref,
        evm_network_context_ref: prepared_invocation_fixture().evm_network_context_ref,
        resource_stage: ContractLifecycleStage::Deployed,
        transaction_hash: TEST_TRANSACTION_HASH.to_owned(),
        block_number: 63,
        status: true,
        receipt_evidence: None,
    }
}

fn external_source_evidence_for_context(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> ExternalEvmSourceEvidence {
    ExternalEvmSourceEvidence {
        network_id: context.value().network.network_id.as_str().to_owned(),
        expected_chain_id: context.value().network.expected_chain_id(),
        observed_chain_id: context.value().network.expected_chain_id(),
        source_ref: "adapter-test".to_owned(),
        policy_id: "adapter-test".to_owned(),
    }
}

fn expected_bool(value: bool) -> ExpectedValue {
    ExpectedValue::from_json_value(&json!(value)).expect("expected bool")
}

fn expected_bool_json(value: bool) -> serde_json::Value {
    json!({ "json_text": value.to_string() })
}

fn validation_read_result(passed: bool) -> ValidationReadResult {
    ValidationReadResult {
        function: "owner".to_owned(),
        args: Vec::new(),
        expected: expected_bool(true),
        actual: expected_bool(passed),
        passed,
    }
}

fn validation_event_result(passed: bool) -> ValidationEventResult {
    ValidationEventResult {
        event: "Configured".to_owned(),
        min_count: 1,
        observed_count: if passed { 1 } else { 0 },
        passed,
    }
}

fn external_adoption_for_replay(
    evidence_policy: serde_json::Value,
) -> mfm_evm_contract_model::AdoptExternalAddress {
    serde_json::from_value(json!({
        "address": "0x000000000000000000000000000000000000beef",
        "provenance_label": "adapter-test",
        "evidence_policy": evidence_policy,
    }))
    .expect("external adoption")
}

fn external_adoption_evidence_for_replay(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    stage: ContractLifecycleStage,
    adoption: &mfm_evm_contract_model::AdoptExternalAddress,
) -> ExternalAdoptionEvidence {
    ExternalAdoptionEvidence {
        evidence_policy_digest: digest_for_value(&adoption.evidence_policy)
            .expect("evidence policy digest"),
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        evm_network_context_ref: evm_network_context_ref(&context.value().network)
            .expect("network context ref"),
        resource_stage: stage,
        observed_chain_id: context.value().network.expected_chain_id(),
        code_read_evidence: None,
        read_assertion_evidence: Vec::new(),
        event_assertion_evidence: Vec::new(),
    }
}

async fn verified_test_finality(
    runtime: &EvmContractRuntime,
    receipt: &ContractTransactionReceipt,
    required_confirmations: u64,
) -> mfm_runtime::Result<u64> {
    verified_finality_confirmations(
        runtime,
        "ethereum-mainnet",
        1,
        std::slice::from_ref(receipt),
        required_confirmations,
    )
    .await
}

fn sorted_json_keys(value: &serde_json::Value) -> Vec<&str> {
    let mut keys = value
        .as_object()
        .expect("json object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

#[derive(Clone, Copy)]
enum RecoveryReceiptMode {
    Landed,
    Pending,
    ProviderFailure,
}

#[derive(Clone, Copy)]
enum RecoveryOccupancyMode {
    Unknown,
    Occupied { transaction_hash: B256 },
}

#[test]
fn replay_verifier_uses_contract_namespace() {
    let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");

    assert_eq!(verifier.verifier_id().as_str(), REPLAY_VERIFIER_ID);
    assert!(!verifier
        .verifier_id()
        .as_str()
        .contains(&["d", "cv"].concat()));
}

#[test]
fn replay_verifier_accepts_context_configure_schemas() {
    let receipt_schema = ContextContractConfigureReceipt::schema_id().expect("receipt schema");
    verify_contract_receipt_schema(&receipt_schema).expect("context configure receipt schema");

    let confirmation = ContextContractConfigureConfirmation {
        confirmation_version: 1,
        context_ref: prepared_invocation_fixture().context_ref,
        evm_network_context_ref: prepared_invocation_fixture().evm_network_context_ref,
        resource_stage: ContractLifecycleStage::Configured,
        confirmations: 3,
        configure_node: LifecycleNodeIdRef::from(node_id(0x44)),
        receipts: Vec::new(),
        call_evidence_refs: Vec::new(),
        confirmation_evidence_refs: Vec::new(),
        configured_block_number: None,
    };
    let confirmation_schema =
        ContextContractConfigureConfirmation::schema_id().expect("confirmation schema");
    let confirmation_bytes = serde_json::to_vec(&confirmation).expect("confirmation json");

    verify_contract_confirmation_schema(&confirmation_schema, &confirmation_bytes, 3)
        .expect("sufficient context configure confirmation depth");
    let error = verify_contract_confirmation_schema(&confirmation_schema, &confirmation_bytes, 4)
        .expect_err("insufficient context configure confirmation depth");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_validation_report_evidence_must_match_retained_read_and_event_results() {
    let context = certified_contract_context("reth-dev", 31337);
    let read_result = validation_read_result(true);
    let event_result = validation_event_result(true);
    let read_evidence = ExternalReadAssertionEvidence {
        source: external_source_evidence_for_context(&context),
        result: read_result.clone(),
    };
    let event_evidence = ExternalEventAssertionEvidence {
        source: external_source_evidence_for_context(&context),
        result: event_result.clone(),
    };

    verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&read_result),
        std::slice::from_ref(&read_evidence),
        std::slice::from_ref(&event_result),
        std::slice::from_ref(&event_evidence),
    )
    .expect("matching validation evidence");

    let missing = verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&read_result),
        &[],
        std::slice::from_ref(&event_result),
        std::slice::from_ref(&event_evidence),
    )
    .expect_err("missing validation read evidence must reject");
    assert_eq!(missing.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut mismatched = read_evidence.clone();
    mismatched.result = validation_read_result(false);
    let error = verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&read_result),
        std::slice::from_ref(&mismatched),
        std::slice::from_ref(&event_result),
        std::slice::from_ref(&event_evidence),
    )
    .expect_err("mismatched validation read evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut wrong_context = read_evidence;
    wrong_context.source.observed_chain_id += 1;
    let error = verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&read_result),
        std::slice::from_ref(&wrong_context),
        std::slice::from_ref(&event_result),
        std::slice::from_ref(&event_evidence),
    )
    .expect_err("wrong-context validation read evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_validation_report_results_must_match_certified_validate_action() {
    let action: ValidateAction = serde_json::from_value(json!({
        "read_assertions": [
            {
                "function": "owner",
                "expected": expected_bool_json(true)
            }
        ],
        "event_assertions": [
            {
                "event": "Configured",
                "min_count": 1
            }
        ]
    }))
    .expect("validate action");
    let context = certified_contract_context("reth-dev", 31337);
    let report = ContextBoundValidationReport {
        report_version: 1,
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        configured_instance: mfm_evm_contract_model::ConfiguredContractInstanceRef {
            context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
            address: ContractAddress::new("0x000000000000000000000000000000000000beef")
                .expect("address"),
            configuration_claim: ConfigurationClaim::ExternalClaimedConfigured {
                provenance_label: serde_json::from_value(json!("adapter-test"))
                    .expect("provenance label"),
                evidence_policy_digest: digest_for_value(&json!({"adapter": "test"}))
                    .expect("digest"),
            },
        },
        observed_chain_id: 31337,
        configuration_read_results: Vec::new(),
        configuration_event_results: Vec::new(),
        read_results: vec![validation_read_result(true)],
        event_results: vec![validation_event_result(true)],
        validation_read_evidence: Vec::new(),
        validation_event_evidence: Vec::new(),
        evidence_refs: Vec::new(),
        valid: true,
    };

    verify_validation_results_match_action(&report, &action)
        .expect("report results match validate action");

    let mut mismatched = report;
    mismatched.read_results[0].expected = expected_bool(false);
    let error = verify_validation_results_match_action(&mismatched, &action)
        .expect_err("mismatched read action evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_external_adoption_evidence_must_match_context_stage_and_policy() {
    let context = certified_contract_context("reth-dev", 31337);
    let adoption = external_adoption_for_replay(json!({
        "require_code": false,
        "allow_external_claimed_configured": true,
        "initial_read_assertions": [
            {
                "function": "owner",
                "expected": expected_bool_json(true)
            }
        ]
    }));
    let mut evidence = external_adoption_evidence_for_replay(
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    );
    evidence
        .read_assertion_evidence
        .push(ExternalReadAssertionEvidence {
            source: external_source_evidence_for_context(&context),
            result: validation_read_result(true),
        });

    verify_replayed_external_adoption(
        &evidence,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect("matching external adoption evidence");

    let mut missing = evidence.clone();
    missing.read_assertion_evidence.clear();
    let error = verify_replayed_external_adoption(
        &missing,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect_err("missing external adoption assertion evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut wrong_context = evidence.clone();
    wrong_context.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(0x92),
    ));
    let error = verify_replayed_external_adoption(
        &wrong_context,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect_err("wrong-context external adoption evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut wrong_stage = evidence;
    wrong_stage.resource_stage = ContractLifecycleStage::Deployed;
    let error = verify_replayed_external_adoption(
        &wrong_stage,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect_err("wrong-stage external adoption evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_external_adoption_requires_code_evidence_when_policy_requires_code() {
    let context = certified_contract_context("reth-dev", 31337);
    let adoption = external_adoption_for_replay(json!({
        "require_code": true,
        "allow_external_claimed_configured": true
    }));
    let evidence = external_adoption_evidence_for_replay(
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    );

    let error = verify_replayed_external_adoption(
        &evidence,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect_err("missing required code evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_submission_rejects_context_mismatch() {
    let prepared = prepared_invocation_fixture();
    let mut submissions = prepared_anchor_submissions(&prepared).expect("submissions");
    submissions.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(0x90),
    ));

    assert!(matches!(
        verify_prepared_submissions(&prepared, &submissions),
        Err(EvmContractAdapterError::InvalidPreparedInvocation)
    ));
}

#[test]
fn replay_receipt_rejects_context_mismatch() {
    let prepared = prepared_invocation_fixture();
    let mut receipt = ContractDeployReceipt {
        receipt_version: 1,
        context_ref: prepared.context_ref.clone(),
        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
        resource_stage: ContractLifecycleStage::Deployed,
        contract_address: "0x000000000000000000000000000000000000beef".to_owned(),
        receipt: ContractTransactionReceipt {
            receipt_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: ContractLifecycleStage::Deployed,
            transaction_hash: prepared.transactions[0].expected_transaction_hash.clone(),
            block_number: 7,
            status: true,
            receipt_evidence: None,
        },
    };
    receipt.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(0x91),
    ));
    let schema = ContractDeployReceipt::schema_id().expect("schema");
    let bytes = serde_json::to_vec(&receipt).expect("receipt json");

    let error = verify_contract_receipt_artifact(&schema, &bytes, &prepared)
        .expect_err("mismatched receipt context");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_verifier_accepts_prepared_context_matching_certified_node_context() {
    let prepared = prepared_invocation_fixture();
    let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");
    let input = replay::SideEffectSubmissionReplayInput {
        intent: replay_intent_evidence(&prepared),
        certified_context: certified_side_effect_context_for_prepared(&prepared),
        prepared_invocation: Some(replay_prepared_evidence(&prepared)),
        submission: replay_submission_evidence(&prepared),
    };

    verifier
        .verify_submission(&input)
        .expect("matching certified context");
}

#[test]
fn replay_verifier_rejects_consistently_wrong_prepared_submission_context() {
    let certified_prepared = prepared_invocation_fixture();
    let wrong_prepared = wrong_context_prepared_invocation_fixture();
    let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");
    let input = replay::SideEffectSubmissionReplayInput {
        intent: replay_intent_evidence(&wrong_prepared),
        certified_context: certified_side_effect_context_for_prepared(&certified_prepared),
        prepared_invocation: Some(replay_prepared_evidence(&wrong_prepared)),
        submission: replay_submission_evidence(&wrong_prepared),
    };

    let error = verifier
        .verify_submission(&input)
        .expect_err("prepared and submission artifacts agree with the wrong context");

    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_verifier_rejects_consistently_wrong_prepared_submission_receipt_context() {
    let certified_prepared = prepared_invocation_fixture();
    let wrong_prepared = wrong_context_prepared_invocation_fixture();
    let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");
    let input = replay::SideEffectReceiptReplayInput {
        intent: replay_intent_evidence(&wrong_prepared),
        certified_context: certified_side_effect_context_for_prepared(&certified_prepared),
        prepared_invocation: Some(replay_prepared_evidence(&wrong_prepared)),
        submission: Some(replay_submission_evidence(&wrong_prepared)),
        receipt: replay_receipt_evidence(&wrong_prepared),
    };

    let error = verifier
        .verify_receipt(&input)
        .expect_err("prepared, submission, and receipt artifacts agree with the wrong context");

    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[tokio::test]
async fn receipt_polling_does_not_retry_permanent_capability_failures() {
    let reads = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(TestEvmProviders::receipt_failure(Arc::clone(&reads)));
    let transaction_hash = TEST_TRANSACTION_HASH;
    let prepared = receipt_polling_invocation(transaction_hash);
    let submissions = ContractTransactionSubmissions {
        submissions_version: 1,
        context_ref: prepared.context_ref.clone(),
        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
        resource_stage: prepared.resource_stage,
        transactions: vec![ContractTransactionSubmission {
            submission_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: prepared.resource_stage,
            transaction_hash: transaction_hash.to_owned(),
            signer_public_key: None,
        }],
    };

    let error = read_receipts_with_poll(&runtime, &prepared, &submissions)
        .await
        .expect_err("permanent capability failure");

    assert!(error.to_string().contains("EVM capability failed"));
    assert_eq!(*reads.lock().expect("reads"), 1);
}

#[tokio::test]
async fn receipt_polling_rejects_tampered_submission_before_provider_read() {
    let reads = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(TestEvmProviders::receipt_failure(Arc::clone(&reads)));
    let prepared = receipt_polling_invocation(TEST_TRANSACTION_HASH);
    let submissions = ContractTransactionSubmissions {
        submissions_version: 1,
        context_ref: prepared.context_ref.clone(),
        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
        resource_stage: prepared.resource_stage,
        transactions: vec![ContractTransactionSubmission {
            submission_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: prepared.resource_stage,
            transaction_hash: "0x2222222222222222222222222222222222222222222222222222222222222222"
                .to_owned(),
            signer_public_key: None,
        }],
    };

    let error = read_receipts_with_poll(&runtime, &prepared, &submissions)
        .await
        .expect_err("tampered submission hash");

    assert!(error.to_string().contains("EVM transaction hash mismatch"));
    assert_eq!(*reads.lock().expect("reads"), 0);
}

#[tokio::test]
async fn finality_confirmation_requires_certified_depth() {
    let runtime = runtime_from_provider(TestEvmProviders::finality());
    let receipt = finality_receipt();

    let confirmations = verified_test_finality(&runtime, &receipt, 2)
        .await
        .expect("sufficient confirmations");
    assert_eq!(confirmations, 2);

    let error = verified_test_finality(&runtime, &receipt, 3)
        .await
        .expect_err("insufficient confirmations");
    assert!(matches!(error, mfm_runtime::RuntimeError::Blocked(_)));
}

#[tokio::test]
async fn finality_rejects_mismatched_guard_evidence() {
    let runtime = runtime_from_provider(TestEvmProviders::finality_mismatched_evidence());
    let receipt = finality_receipt();

    let error = verified_test_finality(&runtime, &receipt, 1)
        .await
        .expect_err("mismatched evidence");

    assert!(matches!(
        error,
        mfm_runtime::RuntimeError::InvalidRunnerOutputDiagnostic { .. }
    ));
    assert!(error.to_string().contains("EVM source chain id"));
}

#[test]
fn replay_confirmation_depth_rejects_insufficient_certified_depth() {
    assert!(ensure_replay_confirmation_depth(3, 3).is_ok());
    let error = ensure_replay_confirmation_depth(2, 3).expect_err("insufficient replay depth");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn adapter_source_has_no_concrete_store_or_signer_provider_coupling() {
    let source = include_str!("lib.rs");
    for forbidden in [
        ["artifact", "_store", "_fs"].concat(),
        ["Fs", "Typed", "Artifact", "Store"].concat(),
        ["Key", "store"].concat(),
        ["Runtime", "Secret", "Source"].concat(),
        ["Signer", "Provider", "Runtime", "Config"].concat(),
        ["MFM", "_EVM", "_RPC"].concat(),
    ] {
        assert!(
            !source.contains(&forbidden),
            "adapter source contains forbidden coupling {forbidden}"
        );
    }
}

fn executable_identity_summary(factories: [&str; 2]) -> Vec<String> {
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm-contracts",
        "evm-contract-lifecycle",
        env!("CARGO_PKG_VERSION"),
    )
    .expect("executable identity template");
    factories
        .into_iter()
        .map(|factory| {
            let identity = executable_identities
                .executable(events::RunnerFactoryId::new(factory).expect("factory id"));
            format!(
                "factory={};cargo_digest={};binary_digest={};nix_derivation={};nix_output={}",
                identity.factory_id,
                identity.cargo_package_digest,
                identity.binary_digest,
                identity.nix_derivation_hash.is_some(),
                identity.nix_output_hash.is_some()
            )
        })
        .collect()
}
