#![warn(missing_docs)]
//! Typed EVM deploy/configure/validate workflow operation.
//!
//! The workflow is authored through `mfm-program` and lowers to certified typed state programs.
//! It exposes no legacy dynamic planner or context-key surface. Deploy and configure are
//! side-effect states; validate is a read state that consumes a [`ConfiguredContract`] typestate
//! value.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_evm_deploy_configure_validate::{
//!     dcv_program_draft, DeployConfigureValidateCanonicalConfig,
//! };
//!
//! # fn demo(config: DeployConfigureValidateCanonicalConfig) -> mfm_program::Result<()> {
//! let draft = dcv_program_draft(config)?;
//! assert_eq!(draft.state_nodes().len(), 3);
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;

use mfm_certify::{certify_program_draft, CertifiedTypedSpec};
use mfm_evm_deploy_configure_validate_config::build_deploy_configure_validate_config;
pub use mfm_evm_deploy_configure_validate_config::{
    canonicalize_deploy_configure_validate_authored_config,
    decode_deploy_configure_validate_built_config,
    decode_deploy_configure_validate_canonical_config,
    parse_deploy_configure_validate_authored_config,
    parse_deploy_configure_validate_authored_config_with_hint,
    DeployConfigureValidateAuthoredConfig, DeployConfigureValidateBuildOutcome,
    DeployConfigureValidateBuildReport, DeployConfigureValidateBuiltConfig,
    DeployConfigureValidateCanonicalConfig, DeployConfigureValidateConfigError,
    DeployConfigureValidateConfigureConfig, DeployConfigureValidateDeployConfig,
    DeployConfigureValidateExecutionConfig, DeployConfigureValidateInput,
    DeployConfigureValidateValidateConfig, ExistingConfiguredContractValidationConfig,
};
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, OperationKind, OperationVersion, SchemaId,
};
use mfm_program::{
    Operation, OperationExpansion, OperationKey, OperationRegistryBuilder, PublicOutputKey,
    RootBuilder, ScopeKey, StateKey, StateRegistryBuilder,
};
use mfm_spec::v1 as spec;
pub use mfm_state_evm_dcv::{
    configure_intent_from_config, deploy_intent_from_config, evm_dcv_adapter_kind,
    evm_dcv_adapter_version, validate_configured_contract_with_backend, ConfigureContractState,
    ConfiguredContract, ConfiguredContractRef, DcvOperationOutputs, DcvPublicOutputs,
    DeployContractState, DeployedContract, EvmDcvConfigureConfirmation,
    EvmDcvConfigureIdempotencyInput, EvmDcvConfigureIntent, EvmDcvConfigureReceipt,
    EvmDcvConfigureReceiptEntry, EvmDcvConfigureSubmission, EvmDcvDeployConfirmation,
    EvmDcvDeployIdempotencyInput, EvmDcvDeployIntent, EvmDcvDeployReceipt, EvmDcvDeploySubmission,
    EvmDcvReadBackend, EvmDcvReadCapability, EvmDcvReadError, EvmDcvReadFuture,
    EvmDcvSignerCapability, EvmDcvTransactionIntent, EvmDcvTransactionSubmitCapability,
    ProtectedRawTransaction, ValidateContractState, ValidationReport,
};

const DCV_OPERATION_KIND_NAME: &str = "deploy_configure_validate_workflow";
const DCV_OPERATION_VERSION: &str = "mfm.evm.dcv.operation.workflow.v1";
const ROOT_SCOPE: &str = "evm_dcv";
const OP_KEY: &str = "deploy_configure_validate";
const PUBLIC_OUTPUT_KEY: &str = "evm_dcv";

/// Typed EVM deploy/configure/validate workflow operation.
pub struct DeployConfigureValidateWorkflowOperation;

impl Operation for DeployConfigureValidateWorkflowOperation {
    type Config = DeployConfigureValidateCanonicalConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = DcvOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            "mfm.evm.dcv",
            DCV_OPERATION_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.evm.dcv.operation:workflow"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(DCV_OPERATION_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.deploy_configure_validate"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let built = build_deploy_configure_validate_config(config)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let mut deploy = built.execution.deploy;
        let mut configure = built.execution.configure;
        let mut validate = built.execution.validate;
        normalize_phase_artifacts(&mut deploy, &mut configure, &mut validate)?;
        validate_phase_alignment(&deploy, &configure, &validate)?;

        let deployed = builder.state::<DeployContractState, _>(
            StateKey::new("deploy_contract")?,
            deploy,
            (),
        )?;
        let configured = builder.state::<ConfigureContractState, _>(
            StateKey::new("configure_contract")?,
            configure,
            deployed.clone(),
        )?;
        let validation_report = builder.state::<ValidateContractState, _>(
            StateKey::new("validate_contract")?,
            validate,
            configured.clone(),
        )?;

        Ok(DcvOperationOutputs {
            deployed,
            configured,
            validation_report,
        })
    }
}

/// Builds the EVM DCV state registry used for authoring and certification.
pub fn dcv_state_registry() -> mfm_program::Result<mfm_program::StateRegistrySnapshot> {
    let mut states = StateRegistryBuilder::new();
    states.register::<DeployContractState>()?;
    states.register::<ConfigureContractState>()?;
    states.register::<ValidateContractState>()?;
    Ok(states.into_snapshot())
}

/// Builds the EVM DCV operation registry used for authoring and certification.
pub fn dcv_operation_registry() -> mfm_program::Result<mfm_program::OperationRegistrySnapshot> {
    let mut operations = OperationRegistryBuilder::new();
    operations.register::<DeployConfigureValidateWorkflowOperation>()?;
    Ok(operations.into_snapshot())
}

/// Adds EVM deploy/configure/validate workflow descriptors to a trusted certification registry.
pub fn register_dcv_certification_descriptors(
    registry: &mut mfm_certify::CertificationRegistry,
) -> mfm_certify::Result<()> {
    let mut states = StateRegistryBuilder::new();
    registry.register_state(
        &states
            .register::<DeployContractState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ConfigureContractState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ValidateContractState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    let mut operations = OperationRegistryBuilder::new();
    registry.register_operation(
        &operations
            .register::<DeployConfigureValidateWorkflowOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    Ok(())
}

/// Builds a typed EVM deploy/configure/validate program draft.
pub fn dcv_program_draft(
    config: DeployConfigureValidateCanonicalConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    mfm_program::build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        dcv_state_registry()?,
        dcv_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let result = root
                .scope()
                .call::<DeployConfigureValidateWorkflowOperation, _>(
                    OperationKey::new(OP_KEY)?,
                    DeployConfigureValidateWorkflowOperation,
                    config,
                    (),
                )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &DcvPublicOutputs {
                    validation_report: result.validation_report,
                },
            )
        },
    )
}

/// Builds and certifies the typed EVM deploy/configure/validate program.
pub fn certified_dcv_spec(
    config: DeployConfigureValidateCanonicalConfig,
) -> mfm_certify::Result<CertifiedTypedSpec> {
    let draft = dcv_program_draft(config)
        .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?;
    certify_program_draft(&draft)
}

/// Canonical bytes for one config artifact required by a typed EVM DCV spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcvConfigArtifact {
    /// Content-addressed artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical content digest.
    pub digest: ContentDigest,
    /// Canonical byte length.
    pub byte_len: u64,
    /// Canonical JSON bytes.
    pub bytes: Vec<u8>,
    /// Config schema id.
    pub schema_id: SchemaId,
    /// Config media type.
    pub media_type: spec::MediaType,
}

/// Returns all author-emitted config artifacts from an EVM DCV draft.
pub fn dcv_draft_config_artifacts(
    draft: &mfm_program::TypedProgramDraft,
) -> mfm_program::Result<Vec<DcvConfigArtifact>> {
    let media_type = spec::MediaType::new("application/json")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    Ok(draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        .map(|config| {
            config_artifact(
                config.canonical_json.clone(),
                config.schema_id.clone(),
                media_type.clone(),
            )
        })
        .collect())
}

/// Returns framework config artifacts introduced during certification.
pub fn dcv_framework_config_artifacts(
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_program::Result<Vec<DcvConfigArtifact>> {
    let mut artifacts = Vec::new();
    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let framework_kind = match framework {
            spec::FrameworkNodeSpec::Bridge(_) => "bridge_same_value",
            spec::FrameworkNodeSpec::PublicOutputRender(_) => "public_output_render",
        };
        let payload = serde_json::json!({
            "framework": framework_kind,
            "node_id": node.node_id.as_str(),
        });
        let json = serde_json::to_string(&payload)
            .map_err(|error| mfm_program::PlanError::Serialize(error.to_string()))?;
        let bytes = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|error| mfm_program::PlanError::Canonical(error.to_string()))?;
        if bytes.content_digest() != node.config_ref.digest
            || bytes.as_bytes().len() as u64 != node.config_ref.byte_len
        {
            return Err(mfm_program::PlanError::Canonical(format!(
                "framework config helper did not match certified config ref for node {}",
                node.node_id
            )));
        }
        artifacts.push(config_artifact(
            bytes,
            node.config_ref.schema_id.clone(),
            node.config_ref.media_type.clone(),
        ));
    }
    Ok(artifacts)
}

/// Returns only config artifacts required by the certified spec, with metadata matching
/// `config_refs`.
pub fn dcv_config_artifacts_for_spec(
    draft: &mfm_program::TypedProgramDraft,
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_program::Result<Vec<DcvConfigArtifact>> {
    let mut candidates = dcv_draft_config_artifacts(draft)?;
    candidates.extend(dcv_framework_config_artifacts(typed_spec)?);
    let mut selected = Vec::new();
    let mut seen_artifact_ids = BTreeMap::new();
    for config_ref in &typed_spec.config_refs {
        if let Some(previous_schema) =
            seen_artifact_ids.insert(config_ref.artifact_id.clone(), config_ref.schema_id.clone())
        {
            if previous_schema != config_ref.schema_id {
                return Err(mfm_program::PlanError::Key(format!(
                    "certified spec contains duplicate config artifact {} with schemas {} and {}",
                    config_ref.artifact_id, previous_schema, config_ref.schema_id
                )));
            }
            continue;
        }
        let artifact = candidates
            .iter()
            .find(|artifact| {
                artifact.artifact_id == config_ref.artifact_id
                    && artifact.digest == config_ref.digest
                    && artifact.byte_len == config_ref.byte_len
                    && artifact.media_type == config_ref.media_type
                    && artifact.schema_id == config_ref.schema_id
            })
            .cloned()
            .ok_or_else(|| {
                mfm_program::PlanError::Key(format!(
                    "missing typed config artifact for {}",
                    config_ref.artifact_id
                ))
            })?;
        selected.push(artifact);
    }
    Ok(selected)
}

fn normalize_phase_artifacts(
    deploy: &mut DeployConfigureValidateDeployConfig,
    configure: &mut DeployConfigureValidateConfigureConfig,
    validate: &mut DeployConfigureValidateValidateConfig,
) -> mfm_program::Result<()> {
    let Some(artifact) = deploy.artifact.clone() else {
        return Err(mfm_program::PlanError::Key(
            "deploy config must include an inline typed contract artifact; dynamic artifact ports are not part of certified typed EVM DCV execution"
                .to_owned(),
        ));
    };
    if configure.artifact.is_none() {
        configure.artifact = Some(artifact.clone());
    }
    if validate.artifact.is_none() {
        validate.artifact = Some(artifact);
    }
    Ok(())
}

fn validate_phase_alignment(
    deploy: &DeployConfigureValidateDeployConfig,
    configure: &DeployConfigureValidateConfigureConfig,
    validate: &DeployConfigureValidateValidateConfig,
) -> mfm_program::Result<()> {
    if deploy.network_id != configure.network_id || deploy.network_id != validate.network_id {
        return Err(mfm_program::PlanError::Key(
            "deploy/configure/validate network_id values must match".to_owned(),
        ));
    }
    if deploy.control_scope != configure.control_scope
        || deploy.control_scope != validate.control_scope
    {
        return Err(mfm_program::PlanError::Key(
            "deploy/configure/validate control_scope values must match".to_owned(),
        ));
    }
    Ok(())
}

fn config_artifact(
    bytes: mfm_canonical::PlainCanonicalJsonBytes,
    schema_id: SchemaId,
    media_type: spec::MediaType,
) -> DcvConfigArtifact {
    let digest = bytes.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let byte_len = bytes.as_bytes().len() as u64;
    DcvConfigArtifact {
        artifact_id,
        digest,
        byte_len,
        bytes: bytes.to_vec(),
        schema_id,
        media_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_evm_dcv_model::{AbiJson, BytecodeJson, ContractArtifactConfig};

    #[test]
    fn dcv_program_lowers_to_typed_lifecycle_states() {
        let draft = dcv_program_draft(sample_config()).expect("draft");
        let node_keys = draft
            .state_nodes()
            .iter()
            .map(|node| node.key.as_str())
            .collect::<Vec<_>>();
        assert!(node_keys.contains(&"deploy_contract"));
        assert!(node_keys.contains(&"configure_contract"));
        assert!(node_keys.contains(&"validate_contract"));
        assert_eq!(draft.state_nodes().len(), 3);
        assert!(
            draft.state_nodes().iter().all(|node| !node
                .state_descriptor_name
                .contains(&format!("{}{}", "Dyn", "Context"))),
            "EVM DCV state descriptors must not expose dynamic context"
        );

        let certified = certify_program_draft(&draft).expect("certified EVM DCV spec");
        certified.envelope().verify_hash().expect("hash verifies");
        let deploy = certified
            .envelope()
            .spec
            .nodes
            .iter()
            .find(|node| node.stable_key.as_str() == "deploy_contract")
            .expect("deploy node");
        let configure = certified
            .envelope()
            .spec
            .nodes
            .iter()
            .find(|node| node.stable_key.as_str() == "configure_contract")
            .expect("configure node");
        let validate = certified
            .envelope()
            .spec
            .nodes
            .iter()
            .find(|node| node.stable_key.as_str() == "validate_contract")
            .expect("validate node");
        assert!(deploy.side_effect.is_some());
        assert!(configure.side_effect.is_some());
        assert!(validate.side_effect.is_none());
        assert_eq!(deploy.adapter_bindings.len(), 1);
        assert_eq!(configure.adapter_bindings.len(), 1);
        assert_eq!(validate.adapter_bindings.len(), 1);
    }

    #[test]
    fn dcv_program_rejects_missing_inline_deploy_artifact() {
        let mut config = sample_config();
        config.deploy.artifact = None;
        let error = dcv_program_draft(config).expect_err("missing artifact rejected");
        assert!(error
            .to_string()
            .contains("dynamic artifact ports are not part of certified typed EVM DCV execution"));
    }

    #[test]
    fn config_artifacts_match_certified_spec_refs() {
        let draft = dcv_program_draft(sample_config()).expect("draft");
        let certified = certify_program_draft(&draft).expect("certified");
        let artifacts = dcv_config_artifacts_for_spec(&draft, &certified.envelope().spec)
            .expect("config artifacts");
        assert_eq!(artifacts.len(), certified.envelope().spec.config_refs.len());
    }

    fn sample_config() -> DeployConfigureValidateCanonicalConfig {
        let artifact = sample_artifact();
        DeployConfigureValidateCanonicalConfig {
            machine_id: "evm_deploy_configure_validate".to_owned(),
            pipeline_version: "v1".to_owned(),
            input: DeployConfigureValidateInput::from_json_value(&serde_json::json!({}))
                .expect("input"),
            deploy: DeployConfigureValidateDeployConfig {
                artifact: Some(artifact.clone()),
                network_id: "local".to_owned(),
                control_scope: "shared".to_owned(),
                from: "0x0000000000000000000000000000000000000000".to_owned(),
                constructor_args: Vec::new(),
                value_wei: None,
                signing_key_env: Some("MFM_TEST_KEY".to_owned()),
                poll_interval_ms: 1,
                max_receipt_polls: 1,
            },
            configure: DeployConfigureValidateConfigureConfig {
                artifact: None,
                network_id: "local".to_owned(),
                control_scope: "shared".to_owned(),
                from: "0x0000000000000000000000000000000000000000".to_owned(),
                signing_key_env: Some("MFM_TEST_KEY".to_owned()),
                calls: vec![mfm_evm_dcv_model::ConfigureCallConfig {
                    function: "configure".to_owned(),
                    args: Vec::new(),
                    value_wei: None,
                }],
                tx_hashes_export_key: "tx_hashes".to_owned(),
                receipts_export_key: "receipts".to_owned(),
                poll_interval_ms: 1,
                max_receipt_polls: 1,
            },
            validate: DeployConfigureValidateValidateConfig {
                artifact: None,
                network_id: "local".to_owned(),
                control_scope: "shared".to_owned(),
                expected_chain_id: 1,
                require_client_substring: "reth".to_owned(),
                read_assertions: Vec::new(),
                event_assertions: Vec::new(),
            },
        }
    }

    fn sample_artifact() -> ContractArtifactConfig {
        ContractArtifactConfig {
            abi: AbiJson::from_json_value(&serde_json::json!([
                {"type": "constructor", "inputs": []},
                {"type": "function", "name": "configure", "inputs": [], "outputs": []}
            ]))
            .expect("abi"),
            bytecode: BytecodeJson::from_json_value(&serde_json::json!("0x6000"))
                .expect("bytecode"),
        }
    }
}
