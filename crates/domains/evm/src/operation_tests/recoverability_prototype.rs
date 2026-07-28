use super::config;
use crate::state::{
    EvmBalanceAsset, EvmBalanceCollectionConfig, EvmBalanceCollectionError, EvmBalanceSource,
    EVM_BALANCE_COLLECTION_SOURCE_LIMIT,
};
use alloy_primitives::{address, Address, B256, U256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PrototypeRoutingGenerationRef(String);

impl PrototypeRoutingGenerationRef {
    fn checked(value: &str) -> Self {
        assert!(!value.is_empty(), "routing generation must be non-empty");
        Self(value.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBootstrapFrame {
    network_id: String,
    expected_chain_id: u64,
    routing_generation: PrototypeRoutingGenerationRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBootstrapRequest {
    network_id: String,
    expected_chain_id: u64,
    routing_generation: PrototypeRoutingGenerationRef,
}

fn author_bootstrap_request(frame: &PrototypeBootstrapFrame) -> PrototypeBootstrapRequest {
    PrototypeBootstrapRequest {
        network_id: frame.network_id.clone(),
        expected_chain_id: frame.expected_chain_id,
        routing_generation: frame.routing_generation.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeCheckedSource {
    network_id: String,
    chain_id: u64,
    routing_generation: PrototypeRoutingGenerationRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeCheckedAsset {
    Native,
    Erc20(Address),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeCheckedBalanceSource {
    account: Address,
    asset: PrototypeCheckedAsset,
}

impl PrototypeCheckedBalanceSource {
    fn from_validated(source: &EvmBalanceSource) -> Result<Self, EvmBalanceCollectionError> {
        let account = source.account_address()?;
        let asset = match source.asset() {
            EvmBalanceAsset::Native => PrototypeCheckedAsset::Native,
            EvmBalanceAsset::Erc20 { .. } => PrototypeCheckedAsset::Erc20(
                source
                    .contract_address_value()?
                    .expect("validated ERC-20 source has a contract"),
            ),
        };
        Ok(Self { account, asset })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeCheckedDemand {
    bootstrap: PrototypeBootstrapRequest,
    native_decimals: u8,
    sources: Vec<PrototypeCheckedBalanceSource>,
}

impl PrototypeCheckedDemand {
    fn from_validated(
        config: &EvmBalanceCollectionConfig,
        routing_generation: PrototypeRoutingGenerationRef,
    ) -> Result<Self, EvmBalanceCollectionError> {
        let binding = config.binding()?;
        let sources = config
            .sources()
            .iter()
            .map(PrototypeCheckedBalanceSource::from_validated)
            .collect::<Result<Vec<_>, _>>()?;
        let bootstrap = author_bootstrap_request(&PrototypeBootstrapFrame {
            network_id: binding.network_id().as_str().to_owned(),
            expected_chain_id: binding.expected_chain_id(),
            routing_generation,
        });
        Ok(Self {
            bootstrap,
            native_decimals: config.native_decimals(),
            sources,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeAnchor {
    number: U256,
    hash: B256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeLatestFrame {
    source: PrototypeCheckedSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeLatestRequest {
    source: PrototypeCheckedSource,
}

fn author_latest_request(frame: &PrototypeLatestFrame) -> PrototypeLatestRequest {
    PrototypeLatestRequest {
        source: frame.source.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeMetadataFrame {
    source: PrototypeCheckedSource,
    anchor: PrototypeAnchor,
    contract: Address,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeMetadataRequest {
    source: PrototypeCheckedSource,
    anchor: PrototypeAnchor,
    contract: Address,
}

fn author_metadata_request(frame: &PrototypeMetadataFrame) -> PrototypeMetadataRequest {
    PrototypeMetadataRequest {
        source: frame.source.clone(),
        anchor: frame.anchor.clone(),
        contract: frame.contract,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBalanceFrame {
    source: PrototypeCheckedSource,
    anchor: PrototypeAnchor,
    balance_source: PrototypeCheckedBalanceSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeBalanceRequest {
    source: PrototypeCheckedSource,
    anchor: PrototypeAnchor,
    balance_source: PrototypeCheckedBalanceSource,
}

fn author_balance_request(frame: &PrototypeBalanceFrame) -> PrototypeBalanceRequest {
    PrototypeBalanceRequest {
        source: frame.source.clone(),
        anchor: frame.anchor.clone(),
        balance_source: frame.balance_source.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeConfirmationFrame {
    source: PrototypeCheckedSource,
    anchor: PrototypeAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeConfirmationRequest {
    source: PrototypeCheckedSource,
    block_number: U256,
    expected_hash: B256,
}

fn author_confirmation_request(frame: &PrototypeConfirmationFrame) -> PrototypeConfirmationRequest {
    PrototypeConfirmationRequest {
        source: frame.source.clone(),
        block_number: frame.anchor.number,
        expected_hash: frame.anchor.hash,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeAnchorFailure {
    AnchorChanged { expected: B256, observed: B256 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeReadFailureVerdict {
    InvalidEvidence,
    InsufficientEvidence,
    Failed(PrototypeEvmReadTerminalFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeEvmReadTerminalFailure {
    DestinationRejected,
    SourceMismatch,
    AnchorChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeEvmSafeFailure {
    RoutingGenerationUnavailable,
    ConfigurationInvalid,
    RequestInvalid,
    AccessCancelledBeforeEntry,
    AccessCancelledAfterEntry,
    TransportFailedBeforeEntry,
    TransportFailedAfterEntry,
    HttpStatus(u16),
    JsonRpcError(i64),
    ResponseInvalidMalformedEnvelope,
    ResponseInvalidResult,
    ResponseMissingResult,
    ResponseTooLarge,
    Unclassified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeEvmReturnedFailure {
    SourceMismatch,
    ChainMismatch,
    AnchorChanged,
}

fn settle_evm_safe_failure(failure: PrototypeEvmSafeFailure) -> PrototypeReadFailureVerdict {
    use PrototypeEvmReadTerminalFailure::DestinationRejected;
    use PrototypeEvmSafeFailure::{
        AccessCancelledAfterEntry, AccessCancelledBeforeEntry, ConfigurationInvalid, HttpStatus,
        JsonRpcError, RequestInvalid, ResponseInvalidMalformedEnvelope, ResponseInvalidResult,
        ResponseMissingResult, ResponseTooLarge, RoutingGenerationUnavailable,
        TransportFailedAfterEntry, TransportFailedBeforeEntry, Unclassified,
    };
    use PrototypeReadFailureVerdict::{Failed, InsufficientEvidence, InvalidEvidence};

    match failure {
        RoutingGenerationUnavailable | ConfigurationInvalid | RequestInvalid => InvalidEvidence,
        AccessCancelledBeforeEntry
        | AccessCancelledAfterEntry
        | TransportFailedBeforeEntry
        | TransportFailedAfterEntry
        | Unclassified => InsufficientEvidence,
        HttpStatus(408 | 425 | 429 | 500 | 502 | 503 | 504 | 507)
        | JsonRpcError(-32603 | -32001 | -32002 | -32005) => InsufficientEvidence,
        HttpStatus(_) | JsonRpcError(_) => Failed(DestinationRejected),
        ResponseInvalidMalformedEnvelope
        | ResponseInvalidResult
        | ResponseMissingResult
        | ResponseTooLarge => InvalidEvidence,
    }
}

fn settle_evm_returned_failure(
    failure: PrototypeEvmReturnedFailure,
) -> PrototypeReadFailureVerdict {
    use PrototypeEvmReadTerminalFailure::{AnchorChanged, SourceMismatch};
    use PrototypeEvmReturnedFailure::{AnchorChanged as ReturnedAnchorChanged, ChainMismatch};
    use PrototypeReadFailureVerdict::Failed;

    match failure {
        PrototypeEvmReturnedFailure::SourceMismatch | ChainMismatch => Failed(SourceMismatch),
        ReturnedAnchorChanged => Failed(AnchorChanged),
    }
}

fn settle_confirmation(
    request: &PrototypeConfirmationRequest,
    observed_hash: B256,
) -> Result<(), PrototypeAnchorFailure> {
    if observed_hash == request.expected_hash {
        Ok(())
    } else {
        Err(PrototypeAnchorFailure::AnchorChanged {
            expected: request.expected_hash,
            observed: observed_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeEvmOperation {
    ChainId,
    LatestAnchor,
    TokenMetadata,
    NativeBalance,
    TokenBalance,
    ConfirmAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeEvmNodeKind {
    Bootstrap,
    LatestAnchor,
    TokenMetadata(Address),
    Balance(usize),
    ConfirmAnchor,
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeEvmNode {
    key: String,
    kind: PrototypeEvmNodeKind,
    dependencies: Vec<usize>,
    operation: Option<PrototypeEvmOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeEvmGraph {
    scope: String,
    demand: PrototypeCheckedDemand,
    nodes: Vec<PrototypeEvmNode>,
    fanout: Vec<usize>,
    confirmation: usize,
    aggregation: usize,
}

fn expand_audited_evm_prototype(
    scope: &str,
    config: &EvmBalanceCollectionConfig,
    routing_generation: PrototypeRoutingGenerationRef,
) -> Result<PrototypeEvmGraph, EvmBalanceCollectionError> {
    let demand = PrototypeCheckedDemand::from_validated(config, routing_generation)?;
    let mut nodes = vec![
        PrototypeEvmNode {
            key: format!("{scope}/bootstrap"),
            kind: PrototypeEvmNodeKind::Bootstrap,
            dependencies: Vec::new(),
            operation: Some(PrototypeEvmOperation::ChainId),
        },
        PrototypeEvmNode {
            key: format!("{scope}/latest_anchor"),
            kind: PrototypeEvmNodeKind::LatestAnchor,
            dependencies: vec![0],
            operation: Some(PrototypeEvmOperation::LatestAnchor),
        },
    ];
    let mut fanout = Vec::new();
    let contracts = demand
        .sources
        .iter()
        .filter_map(|source| match &source.asset {
            PrototypeCheckedAsset::Native => None,
            PrototypeCheckedAsset::Erc20(contract) => Some(*contract),
        })
        .collect::<BTreeSet<_>>();
    for (index, contract) in contracts.into_iter().enumerate() {
        fanout.push(nodes.len());
        nodes.push(PrototypeEvmNode {
            key: format!("{scope}/token_metadata_{index}"),
            kind: PrototypeEvmNodeKind::TokenMetadata(contract),
            dependencies: vec![1],
            operation: Some(PrototypeEvmOperation::TokenMetadata),
        });
    }
    for (index, source) in demand.sources.iter().enumerate() {
        let operation = match &source.asset {
            PrototypeCheckedAsset::Native => PrototypeEvmOperation::NativeBalance,
            PrototypeCheckedAsset::Erc20(_) => PrototypeEvmOperation::TokenBalance,
        };
        fanout.push(nodes.len());
        nodes.push(PrototypeEvmNode {
            key: format!("{scope}/balance_{index}"),
            kind: PrototypeEvmNodeKind::Balance(index),
            dependencies: vec![1],
            operation: Some(operation),
        });
    }
    let confirmation = nodes.len();
    let mut confirmation_dependencies = Vec::with_capacity(fanout.len() + 1);
    confirmation_dependencies.push(1);
    confirmation_dependencies.extend(fanout.iter().copied());
    nodes.push(PrototypeEvmNode {
        key: format!("{scope}/confirm_anchor"),
        kind: PrototypeEvmNodeKind::ConfirmAnchor,
        dependencies: confirmation_dependencies,
        operation: Some(PrototypeEvmOperation::ConfirmAnchor),
    });
    let aggregation = nodes.len();
    let mut aggregation_dependencies = fanout.clone();
    aggregation_dependencies.push(confirmation);
    nodes.push(PrototypeEvmNode {
        key: format!("{scope}/aggregate"),
        kind: PrototypeEvmNodeKind::Aggregate,
        dependencies: aggregation_dependencies,
        operation: None,
    });
    Ok(PrototypeEvmGraph {
        scope: scope.to_owned(),
        demand,
        nodes,
        fanout,
        confirmation,
        aggregation,
    })
}

fn maximum_token_sources() -> Vec<EvmBalanceSource> {
    let mut sources = (1..=EVM_BALANCE_COLLECTION_SOURCE_LIMIT)
        .map(|index| {
            let account = Address::from_word(U256::from(index).into());
            let contract =
                Address::from_word(U256::from(index + EVM_BALANCE_COLLECTION_SOURCE_LIMIT).into());
            EvmBalanceSource::new(
                account,
                EvmBalanceAsset::erc20(contract).expect("non-zero token"),
            )
            .expect("checked maximum source")
        })
        .collect::<Vec<_>>();
    sources.sort();
    sources
}

#[test]
fn recoverability_prototype_expands_exact_typed_fanout_and_fanin() {
    let account = address!("000000000000000000000000000000000000dead");
    let token = address!("0000000000000000000000000000000000000001");
    let other_account = address!("000000000000000000000000000000000000beef");
    let mut sources = vec![
        EvmBalanceSource::new(account, EvmBalanceAsset::Native).expect("native source"),
        EvmBalanceSource::new(account, EvmBalanceAsset::erc20(token).expect("token asset"))
            .expect("first token source"),
        EvmBalanceSource::new(
            other_account,
            EvmBalanceAsset::erc20(token).expect("token asset"),
        )
        .expect("second token source"),
    ];
    sources.sort();
    let config =
        EvmBalanceCollectionConfig::new("ethereum-mainnet", 1, 18, sources).expect("mixed config");
    let generation = PrototypeRoutingGenerationRef::checked("sha256:generation-a");

    let direct = expand_audited_evm_prototype("root/collect", &config, generation.clone())
        .expect("direct prototype");
    let repeated = expand_audited_evm_prototype("root/collect", &config, generation.clone())
        .expect("repeated prototype");
    let nested = expand_audited_evm_prototype("root/child/collect", &config, generation)
        .expect("nested prototype");

    assert_eq!(
        direct, repeated,
        "prototype expansion must be deterministic"
    );
    assert_eq!(direct.scope, "root/collect");
    assert_eq!(
        direct
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, PrototypeEvmNodeKind::TokenMetadata(_)))
            .count(),
        1,
        "metadata must be deduplicated by checked contract"
    );
    assert_eq!(direct.fanout.len(), 4);
    assert_eq!(direct.nodes.len(), 8);
    assert_eq!(
        direct.nodes[direct.confirmation].dependencies,
        [vec![1], direct.fanout.clone()].concat()
    );
    assert_eq!(
        direct.nodes[direct.aggregation].dependencies,
        [direct.fanout.clone(), vec![direct.confirmation],].concat()
    );
    assert!(direct.fanout.iter().all(|index| {
        direct.nodes[*index].dependencies == [1] && direct.nodes[*index].operation.is_some()
    }));
    assert_eq!(
        direct
            .nodes
            .iter()
            .filter(|node| node.operation.is_some())
            .count(),
        7,
        "each live node must own exactly one external operation"
    );
    assert_eq!(
        direct
            .nodes
            .iter()
            .map(|node| &node.kind)
            .collect::<Vec<_>>(),
        nested
            .nodes
            .iter()
            .map(|node| &node.kind)
            .collect::<Vec<_>>(),
        "nesting must change keys, not relative topology"
    );
    assert!(nested
        .nodes
        .iter()
        .all(|node| node.key.starts_with("root/child/collect/")));
}

#[test]
fn recoverability_prototype_requests_are_total_and_generation_stable() {
    let generation = PrototypeRoutingGenerationRef::checked("sha256:generation-a");
    let demand = PrototypeCheckedDemand::from_validated(&config(), generation.clone())
        .expect("checked demand");
    let bootstrap = &demand.bootstrap;
    let checked_source = PrototypeCheckedSource {
        network_id: bootstrap.network_id.clone(),
        chain_id: bootstrap.expected_chain_id,
        routing_generation: bootstrap.routing_generation.clone(),
    };
    let anchor = PrototypeAnchor {
        number: U256::from(42),
        hash: B256::from([0x11; 32]),
    };
    let balance_source = demand.sources[0].clone();
    let token = address!("0000000000000000000000000000000000000001");

    let _: fn(&PrototypeBootstrapFrame) -> PrototypeBootstrapRequest = author_bootstrap_request;
    let _: fn(&PrototypeLatestFrame) -> PrototypeLatestRequest = author_latest_request;
    let _: fn(&PrototypeMetadataFrame) -> PrototypeMetadataRequest = author_metadata_request;
    let _: fn(&PrototypeBalanceFrame) -> PrototypeBalanceRequest = author_balance_request;
    let _: fn(&PrototypeConfirmationFrame) -> PrototypeConfirmationRequest =
        author_confirmation_request;

    let latest = author_latest_request(&PrototypeLatestFrame {
        source: checked_source.clone(),
    });
    let metadata = author_metadata_request(&PrototypeMetadataFrame {
        source: checked_source.clone(),
        anchor: anchor.clone(),
        contract: token,
    });
    let balance = author_balance_request(&PrototypeBalanceFrame {
        source: checked_source.clone(),
        anchor: anchor.clone(),
        balance_source,
    });
    let confirmation = author_confirmation_request(&PrototypeConfirmationFrame {
        source: checked_source.clone(),
        anchor: anchor.clone(),
    });

    assert_eq!(latest.source.routing_generation, generation);
    assert_eq!(metadata.source.routing_generation, generation);
    assert_eq!(metadata.anchor, anchor);
    assert_eq!(metadata.contract, token);
    assert_eq!(balance.source.routing_generation, generation);
    assert_eq!(balance.anchor, anchor);
    assert_eq!(confirmation.source.routing_generation, generation);
    assert_eq!(confirmation.block_number, anchor.number);
    assert_eq!(confirmation.expected_hash, anchor.hash);
    assert_eq!(demand.native_decimals, 18);

    let changed = B256::from([0x22; 32]);
    assert_eq!(
        settle_confirmation(&confirmation, changed),
        Err(PrototypeAnchorFailure::AnchorChanged {
            expected: anchor.hash,
            observed: changed,
        }),
        "a valid changed hash is a typed semantic failure"
    );
}

#[test]
fn recoverability_prototype_maximum_fanout_has_a_fixed_structural_bound() {
    let config =
        EvmBalanceCollectionConfig::new("ethereum-mainnet", 1, 18, maximum_token_sources())
            .expect("maximum checked config");
    let graph = expand_audited_evm_prototype(
        "root/maximum",
        &config,
        PrototypeRoutingGenerationRef::checked("sha256:generation-maximum"),
    )
    .expect("maximum prototype graph");

    let expected_external_operations = 1 + 1 + EVM_BALANCE_COLLECTION_SOURCE_LIMIT * 2 + 1;
    assert_eq!(
        graph.demand.sources.len(),
        EVM_BALANCE_COLLECTION_SOURCE_LIMIT
    );
    assert_eq!(graph.fanout.len(), EVM_BALANCE_COLLECTION_SOURCE_LIMIT * 2);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.operation.is_some())
            .count(),
        expected_external_operations
    );
    assert_eq!(graph.nodes.len(), expected_external_operations + 1);
    assert_eq!(expected_external_operations, 2_051);
    assert_eq!(graph.nodes.len(), 2_052);
}

#[test]
fn recoverability_prototype_freezes_evm_read_failure_verdicts() {
    use PrototypeEvmReadTerminalFailure::{
        AnchorChanged as TerminalAnchorChanged, DestinationRejected,
        SourceMismatch as TerminalSourceMismatch,
    };
    use PrototypeEvmReturnedFailure::{AnchorChanged, ChainMismatch, SourceMismatch};
    use PrototypeEvmSafeFailure::{
        AccessCancelledAfterEntry, AccessCancelledBeforeEntry, ConfigurationInvalid, HttpStatus,
        JsonRpcError, RequestInvalid, ResponseInvalidMalformedEnvelope, ResponseInvalidResult,
        ResponseMissingResult, ResponseTooLarge, RoutingGenerationUnavailable,
        TransportFailedAfterEntry, TransportFailedBeforeEntry, Unclassified,
    };
    use PrototypeReadFailureVerdict::{Failed, InsufficientEvidence, InvalidEvidence};

    for failure in [
        RoutingGenerationUnavailable,
        ConfigurationInvalid,
        RequestInvalid,
        ResponseInvalidMalformedEnvelope,
        ResponseInvalidResult,
        ResponseMissingResult,
        ResponseTooLarge,
    ] {
        assert_eq!(settle_evm_safe_failure(failure), InvalidEvidence);
    }
    for failure in [
        AccessCancelledBeforeEntry,
        AccessCancelledAfterEntry,
        TransportFailedBeforeEntry,
        TransportFailedAfterEntry,
        Unclassified,
    ] {
        assert_eq!(settle_evm_safe_failure(failure), InsufficientEvidence);
    }
    for status in [408, 425, 429, 500, 502, 503, 504, 507] {
        assert_eq!(
            settle_evm_safe_failure(HttpStatus(status)),
            InsufficientEvidence
        );
    }
    for status in [307, 400, 401, 403, 404, 413, 422, 501, 505] {
        assert_eq!(
            settle_evm_safe_failure(HttpStatus(status)),
            Failed(DestinationRejected)
        );
    }
    for code in [-32603, -32001, -32002, -32005] {
        assert_eq!(
            settle_evm_safe_failure(JsonRpcError(code)),
            InsufficientEvidence
        );
    }
    for code in [-32700, -32602, -32000, -8, 0, i64::MAX] {
        assert_eq!(
            settle_evm_safe_failure(JsonRpcError(code)),
            Failed(DestinationRejected)
        );
    }

    assert_eq!(
        settle_evm_returned_failure(SourceMismatch),
        Failed(TerminalSourceMismatch)
    );
    assert_eq!(
        settle_evm_returned_failure(ChainMismatch),
        Failed(TerminalSourceMismatch)
    );
    assert_eq!(
        settle_evm_returned_failure(AnchorChanged),
        Failed(TerminalAnchorChanged)
    );
}
