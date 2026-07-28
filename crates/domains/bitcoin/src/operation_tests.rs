use super::*;
use crate::{
    BitcoinAddress, BitcoinBalanceCollectionError, BitcoinSourceBinding,
    BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT,
};
use bitcoin::{Address, Network, ScriptBuf};
use mfm_program::{
    build_root_with_registries, BridgeKey, BridgePolicy, OperationKey, PublicOutputKey,
    RootBuilder, ScopeKey, StateSpec as _,
};
use mfm_program_derive::PublicOutputs;

const LEGACY_MAIN: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
const SEGWIT_MAIN: &str = "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw";

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.bitcoin.test.balance_operation_outputs")]
struct BalanceOperationOutputs<'program, 'scope> {
    receipt: mfm_program::Handle<'program, 'scope, BitcoinBalanceCollectionReceipt>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.bitcoin.test.multi_parent_outputs")]
struct MultiParentOutputs<'program, 'scope> {
    first: mfm_program::Handle<'program, 'scope, BitcoinBalanceCollectionReceipt>,
    second: mfm_program::Handle<'program, 'scope, BitcoinBalanceCollectionReceipt>,
}

fn config() -> BitcoinBalanceCollectionConfig {
    BitcoinBalanceCollectionConfig::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        vec![LEGACY_MAIN.to_owned(), SEGWIT_MAIN.to_owned()],
    )
    .expect("config")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeRoutingGenerationRef(String);

impl PrototypeRoutingGenerationRef {
    fn checked(value: &str) -> Self {
        assert!(!value.is_empty(), "routing generation must be non-empty");
        Self(value.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBitcoinDemand {
    binding: BitcoinSourceBinding,
    addresses: Vec<BitcoinAddress>,
    routing_generation: PrototypeRoutingGenerationRef,
}

impl PrototypeBitcoinDemand {
    fn from_validated(
        config: &BitcoinBalanceCollectionConfig,
        routing_generation: PrototypeRoutingGenerationRef,
    ) -> Result<Self, BitcoinBalanceCollectionError> {
        let request = config.request()?;
        Ok(Self {
            binding: request.binding().clone(),
            addresses: request.addresses().to_vec(),
            routing_generation,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBlockchainInfoFrame {
    demand: PrototypeBitcoinDemand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBlockchainInfoRequest {
    binding: BitcoinSourceBinding,
    routing_generation: PrototypeRoutingGenerationRef,
}

fn author_blockchain_info_request(
    frame: &PrototypeBlockchainInfoFrame,
) -> PrototypeBlockchainInfoRequest {
    PrototypeBlockchainInfoRequest {
        binding: frame.demand.binding.clone(),
        routing_generation: frame.demand.routing_generation.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeCheckedBitcoinSource {
    binding: BitcoinSourceBinding,
    routing_generation: PrototypeRoutingGenerationRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeScanFrame {
    source: PrototypeCheckedBitcoinSource,
    addresses: Vec<BitcoinAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeScanRequest {
    source: PrototypeCheckedBitcoinSource,
    descriptors: Vec<String>,
}

fn author_scan_request(frame: &PrototypeScanFrame) -> PrototypeScanRequest {
    PrototypeScanRequest {
        source: frame.source.clone(),
        descriptors: frame
            .addresses
            .iter()
            .map(BitcoinAddress::scan_descriptor)
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeScanResult {
    source: PrototypeCheckedBitcoinSource,
    height: u64,
    anchor_hash: String,
    balances: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBlockHashFrame {
    scan: PrototypeScanResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBlockHashRequest {
    source: PrototypeCheckedBitcoinSource,
    height: u64,
    expected_hash: String,
}

fn author_block_hash_request(frame: &PrototypeBlockHashFrame) -> PrototypeBlockHashRequest {
    PrototypeBlockHashRequest {
        source: frame.scan.source.clone(),
        height: frame.scan.height,
        expected_hash: frame.scan.anchor_hash.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeBitcoinFailure {
    AnchorChanged { expected: String, observed: String },
}

fn settle_block_hash_confirmation(
    request: &PrototypeBlockHashRequest,
    observed_hash: &str,
) -> Result<(), PrototypeBitcoinFailure> {
    if observed_hash == request.expected_hash {
        Ok(())
    } else {
        Err(PrototypeBitcoinFailure::AnchorChanged {
            expected: request.expected_hash.clone(),
            observed: observed_hash.to_owned(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeBitcoinOperation {
    GetBlockchainInfo,
    ScanTxOutSetStart,
    GetBlockHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeBitcoinNodeKind {
    BlockchainInfoBootstrap,
    Scan,
    BlockHashConfirmation,
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBitcoinNode {
    kind: PrototypeBitcoinNodeKind,
    dependencies: Vec<usize>,
    operation: Option<PrototypeBitcoinOperation>,
}

fn audited_bitcoin_prototype_graph() -> Vec<PrototypeBitcoinNode> {
    vec![
        PrototypeBitcoinNode {
            kind: PrototypeBitcoinNodeKind::BlockchainInfoBootstrap,
            dependencies: Vec::new(),
            operation: Some(PrototypeBitcoinOperation::GetBlockchainInfo),
        },
        PrototypeBitcoinNode {
            kind: PrototypeBitcoinNodeKind::Scan,
            dependencies: vec![0],
            operation: Some(PrototypeBitcoinOperation::ScanTxOutSetStart),
        },
        PrototypeBitcoinNode {
            kind: PrototypeBitcoinNodeKind::BlockHashConfirmation,
            dependencies: vec![1],
            operation: Some(PrototypeBitcoinOperation::GetBlockHash),
        },
        PrototypeBitcoinNode {
            kind: PrototypeBitcoinNodeKind::Aggregate,
            dependencies: vec![1, 2],
            operation: None,
        },
    ]
}

fn maximum_mainnet_addresses() -> Vec<String> {
    let mut addresses = (0_u64..BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT as u64)
        .map(|index| {
            let script = ScriptBuf::from_bytes(index.to_be_bytes().to_vec());
            Address::p2wsh(&script, Network::Bitcoin).to_string()
        })
        .collect::<Vec<_>>();
    addresses.sort();
    addresses
}

#[test]
fn balance_operation_has_the_exact_deterministic_one_state_topology() {
    let build = || {
        build_root_with_registries(
            ScopeKey::new("balance_operation_test")?,
            bitcoin_collectors_state_registry()?,
            bitcoin_collectors_operation_registry()?,
            |root: &mut RootBuilder<'_, '_>| {
                let output = root.scope().call::<BitcoinBalanceCollectionOperation, _>(
                    OperationKey::new("collect")?,
                    BitcoinBalanceCollectionOperation,
                    config(),
                    (),
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("receipt")?,
                    &BalanceOperationOutputs {
                        receipt: output.receipt,
                    },
                )
            },
        )
    };
    let first = build().expect("first draft");
    let second = build().expect("second draft");
    assert_eq!(first, second);
    assert_eq!(first.state_nodes().len(), 1);
    assert_eq!(first.operation_lineage().len(), 1);
    assert_eq!(
        first.operation_lineage()[0].operation_kind,
        BitcoinBalanceCollectionOperation::kind().expect("operation kind")
    );
    assert_eq!(
        first.state_nodes()[0].state_kind,
        CollectBitcoinBalancesState::kind().expect("collect kind")
    );
    assert_eq!(first.public_output_spec().outputs().len(), 1);
    assert_eq!(
        first.public_output_spec().outputs()[0]
            .public_field_path()
            .as_str(),
        "receipt"
    );
    assert_eq!(
        first.state_nodes()[0].fact_descriptor_allowlist.len(),
        1,
        "the read state automatically declares its fact batch"
    );
    mfm_certify::certify_program_draft(&first).expect("operation draft certifies");
}

#[test]
fn multiple_parent_scopes_compose_the_same_operation_topology() {
    let draft = build_root_with_registries(
        ScopeKey::new("multi_parent").expect("root key"),
        bitcoin_collectors_state_registry().expect("state registry"),
        bitcoin_collectors_operation_registry().expect("operation registry"),
        |root: &mut RootBuilder<'_, '_>| {
            let first = root
                .scope()
                .child_scope(ScopeKey::new("first_parent")?, |child| {
                    let output = child.scope().call::<BitcoinBalanceCollectionOperation, _>(
                        OperationKey::new("collect")?,
                        BitcoinBalanceCollectionOperation,
                        config(),
                        (),
                    )?;
                    let receipt = child.export_to_parent(
                        BridgeKey::new("receipt")?,
                        output.receipt,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(receipt)
                })?;
            let second = root
                .scope()
                .child_scope(ScopeKey::new("second_parent")?, |child| {
                    let output = child.scope().call::<BitcoinBalanceCollectionOperation, _>(
                        OperationKey::new("collect")?,
                        BitcoinBalanceCollectionOperation,
                        config(),
                        (),
                    )?;
                    let receipt = child.export_to_parent(
                        BridgeKey::new("receipt")?,
                        output.receipt,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(receipt)
                })?;
            root.bind_public_outputs(
                PublicOutputKey::new("receipts")?,
                &MultiParentOutputs { first, second },
            )
        },
    )
    .expect("draft");

    assert_eq!(draft.operation_lineage().len(), 2);
    assert_eq!(draft.state_nodes().len(), 2);
    assert!(draft
        .state_nodes()
        .iter()
        .all(|node| node.state_descriptor_name == "mfm.bitcoin.collect_balances"));
    mfm_certify::certify_program_draft(&draft).expect("composed draft certifies");
}

#[test]
fn config_rejects_noncanonical_order_duplicates_and_empty_demand() {
    for addresses in [
        Vec::new(),
        vec![SEGWIT_MAIN.to_owned(), LEGACY_MAIN.to_owned()],
        vec![LEGACY_MAIN.to_owned(), LEGACY_MAIN.to_owned()],
    ] {
        assert!(BitcoinBalanceCollectionConfig::new(
            "bitcoin-mainnet",
            "main",
            "public-bitcoin-core",
            addresses,
        )
        .is_err());
    }
}

#[test]
fn recoverability_prototype_has_exact_response_dependent_chain() {
    let graph = audited_bitcoin_prototype_graph();
    assert_eq!(graph.len(), 4);
    assert_eq!(
        graph.iter().map(|node| node.kind).collect::<Vec<_>>(),
        [
            PrototypeBitcoinNodeKind::BlockchainInfoBootstrap,
            PrototypeBitcoinNodeKind::Scan,
            PrototypeBitcoinNodeKind::BlockHashConfirmation,
            PrototypeBitcoinNodeKind::Aggregate,
        ]
    );
    assert_eq!(graph[0].dependencies, Vec::<usize>::new());
    assert_eq!(graph[1].dependencies, [0]);
    assert_eq!(graph[2].dependencies, [1]);
    assert_eq!(graph[3].dependencies, [1, 2]);
    assert_eq!(
        graph.iter().filter(|node| node.operation.is_some()).count(),
        3,
        "each JSON-RPC operation must have its own live node"
    );
}

#[test]
fn recoverability_prototype_requests_are_total_and_generation_stable() {
    let generation = PrototypeRoutingGenerationRef::checked("sha256:generation-a");
    let demand = PrototypeBitcoinDemand::from_validated(&config(), generation.clone())
        .expect("checked Bitcoin demand");

    let _: fn(&PrototypeBlockchainInfoFrame) -> PrototypeBlockchainInfoRequest =
        author_blockchain_info_request;
    let _: fn(&PrototypeScanFrame) -> PrototypeScanRequest = author_scan_request;
    let _: fn(&PrototypeBlockHashFrame) -> PrototypeBlockHashRequest = author_block_hash_request;

    let info = author_blockchain_info_request(&PrototypeBlockchainInfoFrame {
        demand: demand.clone(),
    });
    let source = PrototypeCheckedBitcoinSource {
        binding: info.binding.clone(),
        routing_generation: info.routing_generation.clone(),
    };
    let scan_request = author_scan_request(&PrototypeScanFrame {
        source: source.clone(),
        addresses: demand.addresses.clone(),
    });
    let scan_result = PrototypeScanResult {
        source,
        height: 840_000,
        anchor_hash: "abababababababababababababababababababababababababababababababab".to_owned(),
        balances: vec![0; demand.addresses.len()],
    };
    let confirmation = author_block_hash_request(&PrototypeBlockHashFrame {
        scan: scan_result.clone(),
    });

    assert_eq!(info.routing_generation, generation);
    assert_eq!(scan_request.source.routing_generation, generation);
    assert_eq!(scan_request.descriptors.len(), demand.addresses.len());
    assert!(scan_request
        .descriptors
        .iter()
        .all(|descriptor| descriptor.starts_with("addr(") && descriptor.ends_with(')')));
    assert_eq!(confirmation.source.routing_generation, generation);
    assert_eq!(confirmation.height, scan_result.height);
    assert_eq!(confirmation.expected_hash, scan_result.anchor_hash);

    let observed = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
    assert_eq!(
        settle_block_hash_confirmation(&confirmation, observed),
        Err(PrototypeBitcoinFailure::AnchorChanged {
            expected: scan_result.anchor_hash.clone(),
            observed: observed.to_owned(),
        })
    );
    assert_eq!(
        scan_result.balances.len(),
        demand.addresses.len(),
        "typed anchor failure must not erase the retained scan output"
    );
}

#[test]
fn recoverability_prototype_maximum_descriptor_demand_remains_one_scan_node() {
    let config = BitcoinBalanceCollectionConfig::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        maximum_mainnet_addresses(),
    )
    .expect("maximum Bitcoin config");
    let demand = PrototypeBitcoinDemand::from_validated(
        &config,
        PrototypeRoutingGenerationRef::checked("sha256:generation-maximum"),
    )
    .expect("maximum checked demand");
    let source = PrototypeCheckedBitcoinSource {
        binding: demand.binding.clone(),
        routing_generation: demand.routing_generation.clone(),
    };
    let request = author_scan_request(&PrototypeScanFrame {
        source,
        addresses: demand.addresses,
    });

    assert_eq!(
        request.descriptors.len(),
        BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT
    );
    assert_eq!(
        audited_bitcoin_prototype_graph()
            .iter()
            .filter(|node| { node.operation == Some(PrototypeBitcoinOperation::ScanTxOutSetStart) })
            .count(),
        1,
        "maximum descriptor demand is one indivisible scan operation"
    );
}
