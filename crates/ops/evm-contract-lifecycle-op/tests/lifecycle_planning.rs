use mfm_authored_config::TOML_JSON_AUTHORED_CONFIG_FORMATS;
use mfm_certify::certify_program_draft;
use mfm_evm_contract_config::{
    ConfigurePhaseConfig, DeployPhaseConfig, EvmContractConfigureEntryConfig,
    EvmContractDeployEntryConfig, EvmContractLifecycleEntryConfig, EvmContractValidateEntryConfig,
    ValidatePhaseConfig,
};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
use mfm_ids::{CellId, ContentDigest, ContextRef, DigestAlgorithm, DigestBytes, RunId, SpecHash};
use mfm_op_evm_contract_lifecycle::*;
use mfm_program::{InputBindingNodeRef, Operation};
use mfm_spec::v1::{CellContextSpec, InputContextSpec, NodeContextSpec};

fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest_str(byte: u8) -> String {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn context_ref_str(byte: u8) -> String {
    ContextRef::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn run_id_str(byte: u8) -> String {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn spec_hash_str(byte: u8) -> String {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn cell_id_str(byte: u8) -> String {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn network_json() -> serde_json::Value {
    network_json_for_chain(1)
}

fn network_json_for_chain(chain_id: u64) -> serde_json::Value {
    serde_json::json!({
        "network_id": "ethereum-mainnet",
        "expected_chain_id": chain_id,
    })
}

fn signer_json() -> serde_json::Value {
    serde_json::json!({
        "signer_ref": "deployer",
        "expected_signer_address": "0x000000000000000000000000000000000000dead",
    })
}

fn context_json() -> serde_json::Value {
    serde_json::json!({
        "lifecycle_key": "example-lifecycle",
        "network": {
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "chain_fingerprint": null,
            "finality_or_observation_policy": null,
        },
        "contract_profile": {
            "profile_id": "example-profile",
            "artifact_digest": content_digest_str(0x20),
            "interface_digest": content_digest_str(0x21),
            "creation_bytecode_digest": null,
            "deployed_code_hash": null,
            "selector_event_policy_digest": null,
        },
    })
}

fn import_from_mfm_run_json(required_stage: &str, context_byte: u8) -> serde_json::Value {
    serde_json::json!({
        "source_run_id": run_id_str(0x30),
        "source_spec_hash": spec_hash_str(0x31),
        "source_cell_or_output_id": {
            "kind": "cell",
            "cell_id": cell_id_str(0x32),
        },
        "source_value_digest": content_digest_str(0x33),
        "source_context_ref": context_ref_str(context_byte),
        "required_stage": required_stage,
    })
}

fn deploy_config() -> DeployPhaseConfig {
    serde_json::from_value(serde_json::json!({
        "network": network_json(),
        "signer": signer_json(),
    }))
    .expect("deploy config")
}

fn configure_config() -> ConfigurePhaseConfig {
    configure_config_for_chain(1)
}

fn configure_config_for_chain(chain_id: u64) -> ConfigurePhaseConfig {
    serde_json::from_value(serde_json::json!({
        "network": network_json_for_chain(chain_id),
        "signer": signer_json(),
        "calls": [],
    }))
    .expect("configure config")
}

fn validate_config() -> ValidatePhaseConfig {
    validate_config_for_chain(1)
}

fn validate_config_for_chain(chain_id: u64) -> ValidatePhaseConfig {
    serde_json::from_value(serde_json::json!({
        "network": network_json_for_chain(chain_id),
    }))
    .expect("validate config")
}

fn context_deploy_entry_config() -> EvmContractDeployEntryConfig {
    serde_json::from_value(serde_json::json!({
        "context": context_json(),
        "deploy": {
            "signer": signer_json(),
        },
    }))
    .expect("context deploy entry")
}

fn context_configure_entry_config() -> EvmContractConfigureEntryConfig {
    serde_json::from_value(serde_json::json!({
        "context": context_json(),
        "import_deployed": {
            "kind": "from_mfm_run",
            "source": import_from_mfm_run_json("deployed", 0x34),
        },
        "configure": {
            "signer": signer_json(),
            "calls": [],
        },
    }))
    .expect("context configure entry")
}

fn context_validate_entry_config() -> EvmContractValidateEntryConfig {
    serde_json::from_value(serde_json::json!({
        "context": context_json(),
        "import_configured": {
            "kind": "from_mfm_run",
            "source": import_from_mfm_run_json("configured", 0x35),
        },
        "validate": {},
    }))
    .expect("context validate entry")
}

fn context_lifecycle_entry_config() -> EvmContractLifecycleEntryConfig {
    serde_json::from_value(serde_json::json!({
        "context": context_json(),
        "deploy": {
            "signer": signer_json(),
        },
        "configure": {
            "signer": signer_json(),
            "calls": [],
        },
        "validate": {},
    }))
    .expect("context lifecycle entry")
}

fn lifecycle_config() -> ContractLifecycleConfig {
    ContractLifecycleConfig::new(deploy_config(), configure_config(), validate_config())
}

fn deployed_contract() -> DeployedContract {
    deployed_contract_on_chain(1)
}

fn deployed_contract_on_chain(chain_id: u64) -> DeployedContract {
    DeployedContract {
        lifecycle_version: 1,
        network_id: "ethereum-mainnet".to_owned(),
        expected_chain_id: chain_id,
        contract_address: "0x000000000000000000000000000000000000dead".to_owned(),
        deploy_tx_hash: "0x01".to_owned(),
        deploy_receipt_evidence: None,
        deployed_block_number: Some(1),
    }
}

fn configured_contract() -> ConfiguredContract {
    configured_contract_on_chain(1)
}

fn configured_contract_on_chain(chain_id: u64) -> ConfiguredContract {
    ConfiguredContract {
        lifecycle_version: 1,
        deployed: deployed_contract_on_chain(chain_id),
        configure_calls: Vec::new(),
        confirmation_read_assertions: Vec::new(),
        confirmation_event_assertions: Vec::new(),
        configure_tx_hashes: Vec::new(),
        configure_receipt_evidence: Vec::new(),
        configured_block_number: Some(2),
    }
}

fn assert_node_requires_draft_context(draft: &mfm_program::TypedProgramDraft, key: &str) {
    let node = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == key)
        .expect("state node");
    assert!(matches!(
        &node.context,
        NodeContextSpec::Required { context_ref } if context_ref == &draft.contexts()[0].context_ref
    ));
}

fn assert_node_output_bound_to_draft_context(draft: &mfm_program::TypedProgramDraft, key: &str) {
    let node = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == key)
        .expect("state node");
    assert!(matches!(
        &node.output_context,
        CellContextSpec::Bound { context_ref, .. } if context_ref == &draft.contexts()[0].context_ref
    ));
}

fn assert_struct_input_bound_to_draft_context(
    draft: &mfm_program::TypedProgramDraft,
    key: &str,
    field: &str,
) {
    let node = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == key)
        .expect("state node");
    let field = match node.input.root.as_ref() {
        InputBindingNodeRef::Struct(fields) => fields
            .iter()
            .find(|binding| binding.field_path.as_str() == field)
            .expect("input field"),
        other => panic!("expected struct input binding, got {other:?}"),
    };
    let cell = match field.node.as_ref() {
        InputBindingNodeRef::Cell(cell) => cell,
        other => panic!("expected cell input binding, got {other:?}"),
    };
    assert!(matches!(
        cell.context(),
        InputContextSpec::Required { context_ref, .. } if context_ref == &draft.contexts()[0].context_ref
    ));
}

#[test]
fn full_lifecycle_program_rejects_phase_network_mismatch() {
    let config = ContractLifecycleConfig::new(
        deploy_config(),
        configure_config_for_chain(2),
        validate_config(),
    );

    let error = contract_lifecycle_program_draft(config).expect_err("network mismatch");

    assert!(error
        .to_string()
        .contains("contract lifecycle deploy/configure expected chain id mismatch"));
}

#[test]
fn configure_program_rejects_seed_typestate_network_mismatch() {
    let error = configure_contract_program_draft(configure_config(), deployed_contract_on_chain(2))
        .expect_err("seed mismatch");

    assert!(error
        .to_string()
        .contains("contract lifecycle seed expected chain id mismatch"));
}

#[test]
fn validate_program_rejects_seed_typestate_network_mismatch() {
    let error = validate_contract_program_draft(validate_config(), configured_contract_on_chain(2))
        .expect_err("seed mismatch");

    assert!(error
        .to_string()
        .contains("contract lifecycle seed expected chain id mismatch"));
}

#[test]
fn full_lifecycle_program_lowers_to_three_ordered_states() {
    let draft = contract_lifecycle_program_draft(lifecycle_config()).expect("draft");

    assert_eq!(draft.state_nodes().len(), 3);
    assert_eq!(draft.state_nodes()[0].key.as_str(), "deploy");
    assert_eq!(draft.state_nodes()[1].key.as_str(), "configure");
    assert_eq!(draft.state_nodes()[2].key.as_str(), "validate");
    assert!(draft
        .state_nodes()
        .iter()
        .all(|node| node.state_descriptor_name.starts_with("mfm.evm.contract.")));

    let certified = certify_program_draft(&draft).expect("certified");
    certified.envelope().verify_hash().expect("hash verifies");
    assert!(certified
        .envelope()
        .spec
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.stable_key.as_str(),
                "deploy" | "configure" | "validate"
            )
        })
        .all(|node| !node.adapter_bindings.is_empty() || node.stable_key.as_str() == "root"));
}

#[test]
fn context_full_lifecycle_program_lowers_to_context_bound_states() {
    let draft =
        context_contract_lifecycle_program_draft(context_lifecycle_entry_config()).expect("draft");

    assert_eq!(draft.contexts().len(), 1);
    assert!(draft.seeds().is_empty());
    assert_eq!(draft.state_nodes().len(), 3);
    assert_eq!(draft.state_nodes()[0].key.as_str(), "deploy");
    assert_eq!(draft.state_nodes()[1].key.as_str(), "configure");
    assert_eq!(draft.state_nodes()[2].key.as_str(), "validate");
    assert_eq!(
        draft
            .state_nodes()
            .iter()
            .map(|node| node.state_descriptor_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "mfm.evm.contract.context_deploy",
            "mfm.evm.contract.context_configure",
            "mfm.evm.contract.context_validate"
        ]
    );
    for key in ["deploy", "configure", "validate"] {
        assert_node_requires_draft_context(&draft, key);
        assert_node_output_bound_to_draft_context(&draft, key);
    }
    assert_struct_input_bound_to_draft_context(&draft, "configure", "deployed");
    assert_struct_input_bound_to_draft_context(&draft, "validate", "configured");

    let certified = certify_program_draft(&draft).expect("certified");
    certified.envelope().verify_hash().expect("hash verifies");
}

#[test]
fn context_deploy_program_declares_context_without_seeds() {
    let draft =
        context_deploy_contract_program_draft(context_deploy_entry_config()).expect("draft");

    assert_eq!(draft.contexts().len(), 1);
    assert!(draft.seeds().is_empty());
    assert_eq!(draft.state_nodes().len(), 1);
    assert_eq!(draft.state_nodes()[0].key.as_str(), "deploy");
    assert_eq!(
        draft.state_nodes()[0].state_descriptor_name,
        "mfm.evm.contract.context_deploy"
    );
    assert_node_requires_draft_context(&draft, "deploy");
    assert_node_output_bound_to_draft_context(&draft, "deploy");
    certify_program_draft(&draft).expect("certified");
}

#[test]
fn context_configure_program_imports_deployed_without_seed_material() {
    let draft =
        context_configure_contract_program_draft(context_configure_entry_config()).expect("draft");

    assert_eq!(draft.contexts().len(), 1);
    assert!(draft.seeds().is_empty());
    assert_eq!(draft.state_nodes().len(), 2);
    assert_eq!(draft.state_nodes()[0].key.as_str(), "import_deployed");
    assert_eq!(draft.state_nodes()[1].key.as_str(), "configure");
    assert_eq!(
        draft
            .state_nodes()
            .iter()
            .map(|node| node.state_descriptor_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "mfm.evm.contract.import_deployed",
            "mfm.evm.contract.context_configure"
        ]
    );
    assert_node_requires_draft_context(&draft, "import_deployed");
    assert_node_requires_draft_context(&draft, "configure");
    assert_node_output_bound_to_draft_context(&draft, "import_deployed");
    assert_node_output_bound_to_draft_context(&draft, "configure");
    assert_struct_input_bound_to_draft_context(&draft, "configure", "deployed");
    certify_program_draft(&draft).expect("certified");
}

#[test]
fn context_validate_program_imports_configured_without_seed_material() {
    let draft =
        context_validate_contract_program_draft(context_validate_entry_config()).expect("draft");

    assert_eq!(draft.contexts().len(), 1);
    assert!(draft.seeds().is_empty());
    assert_eq!(draft.state_nodes().len(), 2);
    assert_eq!(draft.state_nodes()[0].key.as_str(), "import_configured");
    assert_eq!(draft.state_nodes()[1].key.as_str(), "validate");
    assert_eq!(
        draft
            .state_nodes()
            .iter()
            .map(|node| node.state_descriptor_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "mfm.evm.contract.import_configured",
            "mfm.evm.contract.context_validate"
        ]
    );
    assert_node_requires_draft_context(&draft, "import_configured");
    assert_node_requires_draft_context(&draft, "validate");
    assert_node_output_bound_to_draft_context(&draft, "import_configured");
    assert_node_output_bound_to_draft_context(&draft, "validate");
    assert_struct_input_bound_to_draft_context(&draft, "validate", "configured");
    certify_program_draft(&draft).expect("certified");
}

#[test]
fn phase_program_helpers_are_planning_only() {
    let deploy = deploy_contract_program_draft(deploy_config()).expect("deploy draft");
    let configure = configure_contract_program_draft(configure_config(), deployed_contract())
        .expect("configure draft");
    let validate = validate_contract_program_draft(validate_config(), configured_contract())
        .expect("validate draft");

    assert_eq!(deploy.state_nodes().len(), 1);
    assert_eq!(configure.state_nodes().len(), 1);
    assert_eq!(configure.seeds().len(), 1);
    assert_eq!(validate.state_nodes().len(), 1);
    assert_eq!(validate.seeds().len(), 1);
}

#[test]
fn entry_point_plan_helpers_are_draft_only_and_preserve_seeds() {
    let deploy = plan_contract_deploy_entry_point(deploy_config()).expect("deploy plan");
    let configure = plan_contract_configure_entry_point(ContractConfigureEntryPointConfig {
        config: configure_config(),
        deployed: deployed_contract(),
    })
    .expect("configure plan");
    let validate = plan_contract_validate_entry_point(ContractValidateEntryPointConfig {
        config: validate_config(),
        configured: configured_contract(),
    })
    .expect("validate plan");
    let lifecycle =
        plan_contract_lifecycle_entry_point(lifecycle_config()).expect("lifecycle plan");

    assert_eq!(deploy.draft.state_nodes().len(), 1);
    assert!(deploy.seed_material.is_empty());
    assert_eq!(configure.draft.state_nodes().len(), 1);
    assert_eq!(configure.seed_material.len(), 1);
    assert_eq!(
        configure.seed_material[0].seed_id,
        configure.draft.seeds()[0].seed_id
    );
    assert_eq!(validate.draft.state_nodes().len(), 1);
    assert_eq!(validate.seed_material.len(), 1);
    assert_eq!(
        validate.seed_material[0].seed_id,
        validate.draft.seeds()[0].seed_id
    );
    assert_eq!(lifecycle.draft.state_nodes().len(), 3);
    assert!(lifecycle.seed_material.is_empty());
    assert!(!lifecycle.config_material.is_empty());
}

#[test]
fn context_entry_plan_helpers_do_not_emit_legacy_seed_material() {
    let deploy =
        plan_context_contract_deploy_entry_point(context_deploy_entry_config()).expect("deploy");
    let configure = plan_context_contract_configure_entry_point(context_configure_entry_config())
        .expect("configure");
    let validate = plan_context_contract_validate_entry_point(context_validate_entry_config())
        .expect("validate");
    let lifecycle = plan_context_contract_lifecycle_entry_point(context_lifecycle_entry_config())
        .expect("lifecycle");

    assert_eq!(deploy.draft.state_nodes().len(), 1);
    assert!(deploy.draft.seeds().is_empty());
    assert!(deploy.seed_material.is_empty());
    assert_eq!(configure.draft.state_nodes().len(), 2);
    assert!(configure.draft.seeds().is_empty());
    assert!(configure.seed_material.is_empty());
    assert_eq!(validate.draft.state_nodes().len(), 2);
    assert!(validate.draft.seeds().is_empty());
    assert!(validate.seed_material.is_empty());
    assert_eq!(lifecycle.draft.state_nodes().len(), 3);
    assert!(lifecycle.draft.seeds().is_empty());
    assert!(lifecycle.seed_material.is_empty());

    for plan in [&deploy, &configure, &validate, &lifecycle] {
        assert!(!plan.config_material.is_empty());
        assert!(plan
            .draft
            .seeds()
            .iter()
            .all(|seed| seed.key.as_str() != "deployed_contract"
                && seed.key.as_str() != "configured_contract"));
    }
}

#[test]
fn entry_point_descriptors_are_public_launch_surface() {
    assert_eq!(
        CONTRACT_ENTRY_POINTS
            .iter()
            .map(|descriptor| descriptor.public_name)
            .collect::<Vec<_>>(),
        vec![
            "evm_contract_deploy",
            "evm_contract_configure",
            "evm_contract_validate",
            "evm_contract_lifecycle"
        ]
    );
    assert!(CONTRACT_ENTRY_POINTS
        .iter()
        .all(|descriptor| descriptor.namespace == "mfm.evm.contract"
            && descriptor.version == 1
            && descriptor.accepted_config_formats == TOML_JSON_AUTHORED_CONFIG_FORMATS));
}

#[test]
fn operation_ids_use_contract_lifecycle_namespace() {
    let names = [
        DeployContractOperation::name(),
        ConfigureContractOperation::name(),
        ValidateContractOperation::name(),
        ContractLifecycleOperation::name(),
        ContextDeployContractOperation::name(),
        ContextConfigureContractOperation::name(),
        ContextValidateContractOperation::name(),
        ContextContractLifecycleOperation::name(),
    ];
    assert!(names
        .iter()
        .all(|name| name.starts_with("mfm.evm.contract.")));

    let versions = [
        DeployContractOperation::version().expect("deploy"),
        ConfigureContractOperation::version().expect("configure"),
        ValidateContractOperation::version().expect("validate"),
        ContractLifecycleOperation::version().expect("lifecycle"),
        ContextDeployContractOperation::version().expect("context deploy"),
        ContextConfigureContractOperation::version().expect("context configure"),
        ContextValidateContractOperation::version().expect("context validate"),
        ContextContractLifecycleOperation::version().expect("context lifecycle"),
    ];
    assert!(versions
        .iter()
        .all(|version| version.as_str().starts_with("mfm.evm.contract.operation.")));
}

#[test]
fn operation_crate_has_no_live_runtime_dependencies() {
    let source = include_str!("../src/lib.rs");
    for forbidden in [
        ["mfm", "_transports"].concat(),
        ["mfm", "_adapters"].concat(),
        ["mfm", "_signers"].concat(),
        ["artifact", "_store"].concat(),
        ["rpc", "_url"].concat(),
        ["key", "store"].concat(),
        ["private", "_key"].concat(),
        ["pass", "word"].concat(),
    ] {
        assert!(
            !source.contains(&forbidden),
            "operation source contains live runtime dependency term {forbidden}"
        );
    }
}
