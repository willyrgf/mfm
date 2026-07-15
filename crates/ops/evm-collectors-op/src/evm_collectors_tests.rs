use std::collections::BTreeMap;

use super::*;
use mfm_program::{
    build_root_with_registries, InputBindingNodeRef, OperationKey, PublicOutputKey, RootBuilder,
    ScopeKey,
};
use mfm_program_derive::PublicOutputs;

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.test.network_collection_public_outputs")]
struct TestNetworkCollectionPublicOutputs<'program, 'scope> {
    receipt: mfm_program::Handle<'program, 'scope, EvmNetworkCollectionReceipt>,
}

const ACCOUNT_A: &str = "0x0000000000000000000000000000000000000001";
const ACCOUNT_B: &str = "0x0000000000000000000000000000000000000002";
const TOKEN_A: &str = "0x0000000000000000000000000000000000000011";
const TOKEN_B: &str = "0x0000000000000000000000000000000000000012";

fn address(value: &str) -> NormalizedEvmAddress {
    NormalizedEvmAddress::new(value, "test address").expect("normalized address")
}

fn evm_network() -> NetworkConfig {
    NetworkConfig::new(
        "ethereum-mainnet".to_owned(),
        NetworkFamilyConfig::Evm,
        Some(1),
        Some(18),
        None,
        None,
        BTreeMap::new(),
    )
    .expect("EVM network")
}

fn fixture() -> EvmNetworkCollectionConfig {
    EvmNetworkCollectionConfig {
        network: evm_network(),
        native_accounts: vec![address(ACCOUNT_A)],
        erc20_sources: vec![EvmErc20BalanceSourceConfig {
            contract_address: address(TOKEN_A),
            account: address(ACCOUNT_A),
        }],
    }
}

fn network_collection_draft(config: EvmNetworkCollectionConfig) -> mfm_program::TypedProgramDraft {
    build_root_with_registries(
        ScopeKey::new("evm_network_collection_topology").expect("root key"),
        evm_collectors_state_registry().expect("state registry"),
        evm_collectors_operation_registry().expect("operation registry"),
        |root: &mut RootBuilder<'_, '_>| {
            let outputs = root.scope().call::<EvmNetworkCollectionOperation, _>(
                OperationKey::new("network_collection").expect("operation key"),
                EvmNetworkCollectionOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("network_receipt").expect("public output key"),
                &TestNetworkCollectionPublicOutputs {
                    receipt: outputs.receipt,
                },
            )
        },
    )
    .expect("network collection draft")
}

fn state_keys(draft: &mfm_program::TypedProgramDraft) -> Vec<&str> {
    draft
        .state_nodes()
        .iter()
        .map(|node| node.key.as_str())
        .collect()
}

fn state_input_cells<'a>(
    node: &'a mfm_program::StateNodeSpec,
    all_nodes: &'a [mfm_program::StateNodeSpec],
) -> Vec<&'a str> {
    let output_cells = all_nodes
        .iter()
        .map(|node| node.output_cell_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut cells = Vec::new();
    collect_input_cells(node.input.root.as_ref(), &mut cells);
    cells
        .into_iter()
        .filter(|cell| output_cells.contains(cell))
        .collect()
}

fn collect_input_cells<'a>(node: InputBindingNodeRef<'a>, output: &mut Vec<&'a str>) {
    match node {
        InputBindingNodeRef::Unit => {}
        InputBindingNodeRef::Cell(cell) => output.push(cell.cell_id().as_str()),
        InputBindingNodeRef::Tuple(elements) => {
            for element in elements {
                collect_input_cells(element.as_ref(), output);
            }
        }
        InputBindingNodeRef::Struct(fields) => {
            for field in fields {
                collect_input_cells(field.node.as_ref(), output);
            }
        }
        InputBindingNodeRef::Vec { elements, .. }
        | InputBindingNodeRef::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells(element.as_ref(), output);
            }
        }
    }
}

#[test]
fn network_collection_derives_closed_state_policy() {
    let config = fixture();
    validate_evm_network_collection_config(&config).expect("network config");
    let joint_tip = config.joint_tip_config().expect("joint tip config");
    assert_eq!(joint_tip.network, "ethereum-mainnet");
    assert_eq!(joint_tip.chain_id, 1);
    assert_eq!(joint_tip.max_source_reads.get(), EVM_JOINT_TIP_SOURCE_READS);

    let native = config.native_balances_config().expect("native demand");
    let observe = native.observe_config_for_account(&native.accounts[0]);
    assert_eq!(observe.coverage, EVM_NATIVE_BALANCE_COVERAGE);
    assert_eq!(
        observe.max_source_reads.get(),
        EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS
    );

    let erc20 = config.erc20_balances_config().expect("token demand");
    let metadata = erc20.metadata_config_for_contract(&erc20.sources[0].contract_address);
    assert_eq!(
        metadata.max_source_reads.get(),
        EVM_ERC20_METADATA_OBSERVE_SOURCE_READS
    );
    let balance = erc20.balance_config_for_source(&erc20.sources[0]);
    assert_eq!(
        balance.max_source_reads.get(),
        EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS
    );
}

#[test]
fn token_only_and_native_only_networks_are_valid() {
    let token_only = EvmNetworkCollectionConfig {
        network: evm_network(),
        native_accounts: Vec::new(),
        erc20_sources: fixture().erc20_sources,
    };
    validate_evm_network_collection_config(&token_only).expect("token-only");
    assert!(token_only.native_balances_config().is_none());
    assert!(token_only.erc20_balances_config().is_some());

    let native_only = EvmNetworkCollectionConfig {
        network: evm_network(),
        native_accounts: fixture().native_accounts,
        erc20_sources: Vec::new(),
    };
    validate_evm_network_collection_config(&native_only).expect("native-only");
    assert!(native_only.native_balances_config().is_some());
    assert!(native_only.erc20_balances_config().is_none());
}

#[test]
fn deterministic_source_demand_rejects_unsorted_and_duplicate_entries() {
    let mut native_unsorted = fixture();
    native_unsorted.native_accounts = vec![address(ACCOUNT_B), address(ACCOUNT_A)];
    assert!(validate_evm_network_collection_config(&native_unsorted).is_err());

    let mut token_unsorted = fixture();
    token_unsorted.erc20_sources = vec![
        EvmErc20BalanceSourceConfig {
            contract_address: address(TOKEN_B),
            account: address(ACCOUNT_A),
        },
        EvmErc20BalanceSourceConfig {
            contract_address: address(TOKEN_A),
            account: address(ACCOUNT_A),
        },
    ];
    assert!(validate_evm_network_collection_config(&token_unsorted).is_err());
}

#[test]
fn empty_network_collection_is_rejected() {
    let empty = EvmNetworkCollectionConfig {
        network: evm_network(),
        native_accounts: Vec::new(),
        erc20_sources: Vec::new(),
    };
    assert!(validate_evm_network_collection_config(&empty).is_err());
}

#[test]
fn network_expansion_uses_one_joint_tip_for_each_required_resource() {
    let config = fixture();
    let first = network_collection_draft(config.clone());
    let second = network_collection_draft(config);
    assert_eq!(first, second, "network expansion must be deterministic");

    let nodes = first.state_nodes();
    let keys = state_keys(&first);
    assert_eq!(
        keys.iter()
            .filter(|key| **key == "resolve_joint_tip")
            .count(),
        1
    );
    assert!(keys.contains(&"observe_native_balance_0"));
    assert!(keys.contains(&"observe_erc20_metadata_0"));
    assert!(keys.contains(&"observe_erc20_balance_0"));
    assert!(keys.contains(&"assemble_native_balance_receipt"));
    assert!(keys.contains(&"assemble_erc20_balance_receipt"));
    assert!(keys.contains(&"assemble_network_collection_receipt"));

    let tip_cell = nodes
        .iter()
        .find(|node| node.key.as_str() == "resolve_joint_tip")
        .expect("joint tip node")
        .output_cell_id
        .as_str();
    for key in [
        "observe_native_balance_0",
        "observe_erc20_metadata_0",
        "observe_erc20_balance_0",
    ] {
        let node = nodes
            .iter()
            .find(|node| node.key.as_str() == key)
            .unwrap_or_else(|| panic!("{key} node"));
        assert!(
            state_input_cells(node, nodes).contains(&tip_cell),
            "{key} must consume the coordinator-owned joint tip"
        );
    }
}

#[test]
fn network_expansion_omits_unrequested_resources_and_reuses_token_metadata() {
    let native_only = EvmNetworkCollectionConfig {
        network: evm_network(),
        native_accounts: vec![address(ACCOUNT_A)],
        erc20_sources: Vec::new(),
    };
    let native_draft = network_collection_draft(native_only);
    let native_keys = state_keys(&native_draft);
    assert!(native_keys.iter().all(|key| !key.contains("erc20")));
    assert!(native_keys.contains(&"observe_native_balance_0"));

    let token_only = EvmNetworkCollectionConfig {
        network: evm_network(),
        native_accounts: Vec::new(),
        erc20_sources: vec![EvmErc20BalanceSourceConfig {
            contract_address: address(TOKEN_A),
            account: address(ACCOUNT_A),
        }],
    };
    let token_draft = network_collection_draft(token_only);
    let token_keys = state_keys(&token_draft);
    assert!(token_keys.iter().all(|key| !key.contains("native_balance")));
    assert!(token_keys.contains(&"observe_erc20_metadata_0"));
    assert!(token_keys.contains(&"observe_erc20_balance_0"));

    let repeated_token = EvmNetworkCollectionConfig {
        network: evm_network(),
        native_accounts: Vec::new(),
        erc20_sources: vec![
            EvmErc20BalanceSourceConfig {
                contract_address: address(TOKEN_A),
                account: address(ACCOUNT_A),
            },
            EvmErc20BalanceSourceConfig {
                contract_address: address(TOKEN_A),
                account: address(ACCOUNT_B),
            },
        ],
    };
    let repeated_draft = network_collection_draft(repeated_token);
    let repeated_keys = state_keys(&repeated_draft);
    assert_eq!(
        repeated_keys
            .iter()
            .filter(|key| key.starts_with("observe_erc20_metadata_"))
            .count(),
        1
    );
    assert_eq!(
        repeated_keys
            .iter()
            .filter(|key| key.starts_with("observe_erc20_balance_"))
            .count(),
        2
    );
}
