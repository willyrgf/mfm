use mfm_authored_config::TOML_JSON_AUTHORED_CONFIG_FORMATS;
use mfm_certify::certify_program_draft;
use mfm_evm_contract_config::{ConfigurePhaseConfig, DeployPhaseConfig, ValidatePhaseConfig};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
use mfm_op_evm_contract_lifecycle::*;
use mfm_program::Operation;

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

fn lifecycle_config() -> ContractLifecycleConfig {
    ContractLifecycleConfig::new(deploy_config(), configure_config(), validate_config())
}

#[test]
fn lifecycle_config_rejects_stale_version_field() {
    let value = serde_json::json!({
        "lifecycle_version": 1,
        "deploy": serde_json::to_value(deploy_config()).expect("deploy json"),
        "configure": serde_json::to_value(configure_config()).expect("configure json"),
        "validate": serde_json::to_value(validate_config()).expect("validate json"),
    });

    let error = serde_json::from_value::<ContractLifecycleConfig>(value)
        .expect_err("stale lifecycle version field");

    assert!(error.to_string().contains("unknown field"));
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
    ];
    assert!(names
        .iter()
        .all(|name| name.starts_with("mfm.evm.contract.")));

    let versions = [
        DeployContractOperation::version().expect("deploy"),
        ConfigureContractOperation::version().expect("configure"),
        ValidateContractOperation::version().expect("validate"),
        ContractLifecycleOperation::version().expect("lifecycle"),
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
