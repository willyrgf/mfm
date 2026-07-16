#![warn(missing_docs)]
//! Deterministic end-to-end portfolio snapshot composition.
//!
//! The operation accepts exactly one normalized [`PortfolioConfig`] authority. It compiles the
//! explicit logical wallet-to-symbol demand into family-specific physical work, then proves that
//! every completed family receipt matches that manifest before receipt-pinned selection, snapshot
//! assembly, and report projection may run.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_op_portfolio_snapshot::portfolio_snapshot_program_draft;
//! use mfm_portfolio_model::portfolio::PortfolioConfig;
//!
//! fn draft(config: PortfolioConfig) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
//!     portfolio_snapshot_program_draft(config)
//! }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion, StateKind, StateVersion};
use mfm_op_btc_collectors::{
    BtcNativeBalancesAtAnchorConfig, BtcNetworkCollectionConfig, BtcNetworkCollectionOperation,
    BtcNetworkCollectionReceipt,
};
use mfm_op_evm_collectors::{
    EvmErc20BalanceSourceConfig, EvmNetworkCollectionConfig, EvmNetworkCollectionOperation,
    EvmNetworkCollectionReceipt,
};
use mfm_portfolio_model::domain_key::{
    HoldingsDomainKey, ReportDomainKey, SubjectDomainKey, ValuationDomainKey,
};
use mfm_portfolio_model::ids::NormalizedEvmAddress;
use mfm_portfolio_model::portfolio::{
    ExecutionAnchor, NetworkConfig, NetworkPin, PortfolioConfig, ValidatedPortfolioConfig,
};
use mfm_portfolio_model::symbol::HoldingSourceConfig;
use mfm_program::{
    build_root_with_registries, BridgeKey, BridgePolicy, NoContext, Operation, OperationExpansion,
    OperationKey, PublicOutputKey, PureState, RootBuilder, ScopeKey, StateError, StateResult,
    StateSpec, TypedProgramLaunchPlan, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, OperationOutput, StateInput};
use mfm_state_portfolio::{
    AssembleSnapshotConfig, AssembleSnapshotInputHandles, AssembleSnapshotState,
    CollectedHoldingReceipt, HoldingManifestEntry, HoldingRequirementKey, HoldingSourceKey,
    PortfolioCollectionReceipt, PortfolioHoldingErrorCode, PortfolioHoldingSelectionError,
    PortfolioPublicOutputs, ProjectReportConfig, ProjectReportInputHandles, ProjectReportState,
    ResolveSubjectsConfig, ResolveSubjectsState, ResolveValuationsConfig, ResolveValuationsState,
    SelectHoldingsConfig, SelectHoldingsInputHandles, SelectHoldingsState,
};
use mfm_values::ConfigError;
use serde::{Deserialize, Serialize};

#[path = "replay.rs"]
mod replay;
pub use self::replay::verify_portfolio_collection_receipt_replay;

const OP_NAMESPACE: &str = "mfm.portfolio";
const OP_KIND_NAME: &str = "snapshot";
const OP_VERSION: &str = "mfm.portfolio.operation.snapshot.v1";
const ROOT_SCOPE: &str = "portfolio_snapshot";
const OPERATION_KEY: &str = "portfolio_snapshot";
const PUBLIC_OUTPUT_KEY: &str = "portfolio_snapshot";

/// Certified manifest config for the operation-local portfolio receipt assembler.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.portfolio.operation.config.assemble_collection_receipt",
    validate = "validate_assemble_portfolio_collection_receipt_config"
)]
pub struct AssemblePortfolioCollectionReceiptConfig {
    /// Sorted exact logical-to-source manifest compiled from the sole portfolio config.
    pub manifest: Vec<HoldingManifestEntry>,
}

/// Validates the operation-local exact receipt manifest.
pub fn validate_assemble_portfolio_collection_receipt_config(
    config: &AssemblePortfolioCollectionReceiptConfig,
) -> Result<(), String> {
    mfm_state_portfolio::manifest_identity(&config.manifest)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Typed family receipt fan-in for the exact portfolio receipt assembler.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.portfolio.operation.input.assemble_collection_receipt")]
pub struct AssemblePortfolioCollectionReceiptInput {
    /// One receipt for every demanded Bitcoin network, in deterministic child order.
    pub bitcoin_receipts: Vec<BtcNetworkCollectionReceipt>,
    /// One receipt for every demanded EVM network, in deterministic child order.
    pub evm_receipts: Vec<EvmNetworkCollectionReceipt>,
}

/// Operation-local pure state that proves exact logical receipt completion.
pub struct AssemblePortfolioCollectionReceiptState {
    config: AssemblePortfolioCollectionReceiptConfig,
}

impl StateSpec for AssemblePortfolioCollectionReceiptState {
    type Config = AssemblePortfolioCollectionReceiptConfig;
    type Context = NoContext;
    type Input = AssemblePortfolioCollectionReceiptInput;
    type Output = PortfolioCollectionReceipt;
    type Effect = mfm_effects::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            OP_NAMESPACE,
            "assemble_collection_receipt",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.portfolio.state:assemble_collection_receipt"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.portfolio.state.assemble_collection_receipt.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.assemble_collection_receipt"
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for AssemblePortfolioCollectionReceiptState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        assemble_portfolio_collection_receipt(&self.config, input)
    }
}

/// Assembles a portfolio receipt only if typed family receipts exactly satisfy the manifest.
pub fn assemble_portfolio_collection_receipt(
    config: &AssemblePortfolioCollectionReceiptConfig,
    input: AssemblePortfolioCollectionReceiptInput,
) -> StateResult<PortfolioCollectionReceipt> {
    validate_assemble_portfolio_collection_receipt_config(config).map_err(StateError::Message)?;

    let expected_by_source = config
        .manifest
        .iter()
        .map(|entry| (entry.source().clone(), entry.requirement().clone()))
        .collect::<BTreeMap<_, _>>();
    if expected_by_source.len() != config.manifest.len() {
        return Err(receipt_state_error(
            "portfolio collection manifest contained duplicate sources",
            None,
        ));
    }

    let mut seen_networks = BTreeSet::new();
    let mut actual_by_source = BTreeMap::new();
    for receipt in input.bitcoin_receipts {
        if !seen_networks.insert(receipt.network().to_owned()) {
            return Err(receipt_state_error(
                "portfolio collection receipt contained duplicate network receipts",
                None,
            ));
        }
        let anchor = ExecutionAnchor::Bitcoin {
            height: receipt.anchor_height(),
            block_hash: receipt.anchor_hash().to_owned(),
        };
        for entry in receipt.entries() {
            let source = HoldingSourceKey::BitcoinNative {
                network_id: entry.source_key().network().to_owned(),
                bitcoin_network: entry.source_key().bitcoin_network().to_owned(),
                semantic_source_identity: entry.source_key().semantic_source_identity().to_owned(),
                address: entry.source_key().address().to_owned(),
            };
            insert_actual_receipt(
                &mut actual_by_source,
                source,
                anchor.clone(),
                entry.coverage().to_owned(),
                entry.source_status().to_owned(),
                mfm_facts::FactContentIdentityEvidence::from_verified(
                    entry.fact_content_identity(),
                ),
            )?;
        }
    }
    for receipt in input.evm_receipts {
        if !seen_networks.insert(receipt.network().to_owned()) {
            return Err(receipt_state_error(
                "portfolio collection receipt contained duplicate network receipts",
                None,
            ));
        }
        let anchor = ExecutionAnchor::Evm {
            chain_id: receipt.chain_id(),
            block_number: receipt.block_number(),
            block_hash: receipt.block_hash().to_owned(),
        };
        if let Some(native) = receipt.native_balance_receipt() {
            for entry in native.entries() {
                let source = HoldingSourceKey::EvmNative {
                    network_id: entry.source_key().network().to_owned(),
                    chain_id: entry.source_key().chain_id(),
                    account: entry.source_key().account().to_owned(),
                };
                insert_actual_receipt(
                    &mut actual_by_source,
                    source,
                    anchor.clone(),
                    entry.coverage().to_owned(),
                    entry.source_status().to_owned(),
                    mfm_facts::FactContentIdentityEvidence::from_verified(
                        entry.fact_content_identity(),
                    ),
                )?;
            }
        }
        if let Some(erc20) = receipt.erc20_balance_receipt() {
            for entry in erc20.entries() {
                let source = HoldingSourceKey::EvmErc20 {
                    network_id: entry.source_key().network().to_owned(),
                    chain_id: entry.source_key().chain_id(),
                    contract_address: entry.source_key().contract_address().to_owned(),
                    account: entry.source_key().account().to_owned(),
                };
                insert_actual_receipt(
                    &mut actual_by_source,
                    source,
                    anchor.clone(),
                    entry.coverage().to_owned(),
                    entry.source_status().to_owned(),
                    mfm_facts::FactContentIdentityEvidence::from_verified(
                        entry.fact_content_identity(),
                    ),
                )?;
            }
        }
    }

    if let Some((source, requirement)) = expected_by_source
        .iter()
        .find(|(source, _)| !actual_by_source.contains_key(*source))
    {
        return Err(receipt_state_error(
            format!("portfolio collection receipt was missing required source {source:?}"),
            Some(requirement),
        ));
    }
    if let Some(source) = actual_by_source
        .keys()
        .find(|source| !expected_by_source.contains_key(*source))
    {
        return Err(receipt_state_error(
            format!("portfolio collection receipt contained unexpected source {source:?}"),
            None,
        ));
    }

    let mut entries = Vec::with_capacity(config.manifest.len());
    for manifest in &config.manifest {
        let actual = actual_by_source.get(manifest.source()).ok_or_else(|| {
            receipt_state_error(
                "portfolio collection receipt was missing a required source",
                Some(manifest.requirement()),
            )
        })?;
        entries.push(
            CollectedHoldingReceipt::new(
                manifest.requirement().clone(),
                manifest.source().clone(),
                actual.anchor.clone(),
                actual.coverage.clone(),
                actual.source_status.clone(),
                actual.fact_content_identity.clone(),
            )
            .map_err(|error| StateError::Message(error.to_string()))?,
        );
    }
    let mut anchors = BTreeMap::new();
    for entry in &entries {
        match anchors.get(&entry.requirement().network_id) {
            None => {
                anchors.insert(
                    entry.requirement().network_id.clone(),
                    entry.anchor().clone(),
                );
            }
            Some(anchor) if anchor == entry.anchor() => {}
            Some(_) => {
                return Err(receipt_state_error(
                    "portfolio collection receipt entries disagreed on a network anchor",
                    Some(entry.requirement()),
                ));
            }
        }
    }
    let network_anchors = anchors
        .into_iter()
        .map(|(network_id, anchor)| NetworkPin { network_id, anchor })
        .collect();
    PortfolioCollectionReceipt::new(&config.manifest, entries, network_anchors)
        .map_err(|error| StateError::Message(error.to_string()))
}

#[derive(Clone)]
struct ActualReceiptEntry {
    anchor: ExecutionAnchor,
    coverage: String,
    source_status: String,
    fact_content_identity: mfm_facts::FactContentIdentityEvidence,
}

fn insert_actual_receipt(
    actual_by_source: &mut BTreeMap<HoldingSourceKey, ActualReceiptEntry>,
    source: HoldingSourceKey,
    anchor: ExecutionAnchor,
    coverage: String,
    source_status: String,
    fact_content_identity: mfm_facts::FactContentIdentityEvidence,
) -> StateResult<()> {
    if actual_by_source
        .insert(
            source,
            ActualReceiptEntry {
                anchor,
                coverage,
                source_status,
                fact_content_identity,
            },
        )
        .is_some()
    {
        return Err(receipt_state_error(
            "portfolio collection receipt contained a duplicate source",
            None,
        ));
    }
    Ok(())
}

fn receipt_state_error(
    message: impl Into<String>,
    requirement: Option<&HoldingRequirementKey>,
) -> StateError {
    let (holding_key, network_id) = requirement
        .map(|key| (Some(key.as_key_str()), Some(key.network_id.clone())))
        .unwrap_or((None, None));
    StateError::Message(
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::ReceiptMismatch,
            message,
            holding_key,
            network_id,
        )
        .to_string(),
    )
}

/// Internal handles produced by the complete portfolio snapshot operation.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.portfolio.operation_outputs.snapshot")]
pub struct PortfolioSnapshotOperationOutputs<'program, 'scope> {
    /// Fully assembled portfolio snapshot.
    pub snapshot:
        mfm_program::Handle<'program, 'scope, mfm_portfolio_model::portfolio::PortfolioSnapshot>,
    /// Public report projection derived from the snapshot.
    pub report:
        mfm_program::Handle<'program, 'scope, mfm_portfolio_model::portfolio::PortfolioReport>,
}

/// Deterministic operation for one complete receipt-pinned portfolio snapshot.
pub struct PortfolioSnapshotOperation;

impl Operation for PortfolioSnapshotOperation {
    type Config = PortfolioConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = PortfolioSnapshotOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.portfolio.operation:snapshot"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.snapshot"
    }

    fn expand<'program, 'scope>(
        &self,
        config: ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let supplied = config.into_inner();
        let portfolio = ValidatedPortfolioConfig::new(supplied.clone())
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?
            .into_config();
        if supplied != portfolio {
            return Err(mfm_program::PlanError::Key(
                "portfolio operation config must be normalized before expansion".to_owned(),
            ));
        }
        let compiled = compile_collection(&portfolio)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;

        let mut bitcoin_receipts = Vec::with_capacity(compiled.bitcoin_collections.len());
        for (index, child_config) in compiled.bitcoin_collections.iter().enumerate() {
            let child_config = child_config.clone();
            bitcoin_receipts.push(builder.child_scope(
                ScopeKey::new(format!("bitcoin_collection_{index}"))?,
                |child| {
                    let output = child.scope().call::<BtcNetworkCollectionOperation, _>(
                        OperationKey::new("btc_network_collection")?,
                        BtcNetworkCollectionOperation,
                        child_config,
                        (),
                    )?;
                    let receipt = child.export_to_parent(
                        BridgeKey::new("network_receipt")?,
                        output.receipt,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(receipt)
                },
            )?);
        }
        let mut evm_receipts = Vec::with_capacity(compiled.evm_collections.len());
        for (index, child_config) in compiled.evm_collections.iter().enumerate() {
            let child_config = child_config.clone();
            evm_receipts.push(builder.child_scope(
                ScopeKey::new(format!("evm_collection_{index}"))?,
                |child| {
                    let output = child.scope().call::<EvmNetworkCollectionOperation, _>(
                        OperationKey::new("evm_network_collection")?,
                        EvmNetworkCollectionOperation,
                        child_config,
                        (),
                    )?;
                    let receipt = child.export_to_parent(
                        BridgeKey::new("network_receipt")?,
                        output.receipt,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(receipt)
                },
            )?);
        }
        let receipt = builder.state::<AssemblePortfolioCollectionReceiptState, _>(
            mfm_program::StateKey::new("assemble_collection_receipt")?,
            NoContext,
            AssemblePortfolioCollectionReceiptConfig {
                manifest: compiled.manifest,
            },
            AssemblePortfolioCollectionReceiptInputHandles {
                bitcoin_receipts,
                evm_receipts,
            },
        )?;
        expand_receipt_pinned_report(builder, portfolio, receipt)
    }
}

/// Expands the report half of the portfolio objective after exact collection receipt assembly.
///
/// This remains private to the snapshot operation: callers may only plan the complete collection
/// and reporting objective from one normalized portfolio config.
fn expand_receipt_pinned_report<'program, 'scope>(
    builder: &mut OperationExpansion<'program, 'scope>,
    portfolio: PortfolioConfig,
    receipt: mfm_program::Handle<'program, 'scope, PortfolioCollectionReceipt>,
) -> mfm_program::Result<PortfolioSnapshotOperationOutputs<'program, 'scope>> {
    let normalized = ValidatedPortfolioConfig::new(portfolio.clone())
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?
        .into_config();
    if normalized != portfolio {
        return Err(mfm_program::PlanError::Key(
            "portfolio snapshot config must be normalized before expansion".to_owned(),
        ));
    }

    let subject_key = SubjectDomainKey::new("portfolio_subjects")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    let holdings_key = HoldingsDomainKey::new("portfolio_holdings")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    let valuation_key = ValuationDomainKey::new("portfolio_valuations")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    let report_key = ReportDomainKey::new("portfolio_report")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;

    let subjects = builder.state_with_domain_keys::<ResolveSubjectsState, _, _>(
        mfm_program::StateKey::new("resolve_subjects")?,
        NoContext,
        ResolveSubjectsConfig::new(portfolio.clone())
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        (),
        vec![subject_key],
    )?;
    let holdings = builder.state_with_domain_keys::<SelectHoldingsState, _, _>(
        mfm_program::StateKey::new("select_holdings")?,
        NoContext,
        SelectHoldingsConfig::new(portfolio.clone())
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        SelectHoldingsInputHandles {
            receipt: receipt.clone(),
        },
        vec![holdings_key],
    )?;
    let valuations = builder.state_with_domain_keys::<ResolveValuationsState, _, _>(
        mfm_program::StateKey::new("resolve_valuations")?,
        NoContext,
        ResolveValuationsConfig::new(portfolio.clone())
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        (),
        vec![valuation_key],
    )?;
    let snapshot = builder.state::<AssembleSnapshotState, _>(
        mfm_program::StateKey::new("assemble_snapshot")?,
        NoContext,
        AssembleSnapshotConfig::new(2, portfolio.clone())
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        AssembleSnapshotInputHandles {
            subjects,
            holdings,
            valuations,
            receipt,
        },
    )?;
    let report = builder.state_with_domain_keys::<ProjectReportState, _, _>(
        mfm_program::StateKey::new("project_report")?,
        NoContext,
        ProjectReportConfig::new(2)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        ProjectReportInputHandles {
            snapshot: snapshot.clone(),
        },
        vec![report_key],
    )?;

    Ok(PortfolioSnapshotOperationOutputs { snapshot, report })
}

/// Builds one complete typed portfolio snapshot program draft.
///
/// The draft binds exactly one [`PortfolioPublicOutputs`] value. It intentionally remains an
/// internal operation helper until application ingress publishes the portfolio snapshot entry
/// point in the later ingress phase.
pub fn portfolio_snapshot_program_draft(
    config: PortfolioConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        portfolio_snapshot_state_registry()?,
        portfolio_snapshot_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let output = root.scope().call::<PortfolioSnapshotOperation, _>(
                OperationKey::new(OPERATION_KEY)?,
                PortfolioSnapshotOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &PortfolioPublicOutputs {
                    snapshot: output.snapshot,
                    report: output.report,
                },
            )
        },
    )
}

/// Builds the launch plan for one complete typed portfolio snapshot program.
pub fn portfolio_snapshot_program_launch_plan(
    config: PortfolioConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    TypedProgramLaunchPlan::from_draft(portfolio_snapshot_program_draft(config)?)
}

struct CompiledCollection {
    manifest: Vec<HoldingManifestEntry>,
    bitcoin_collections: Vec<BtcNetworkCollectionConfig>,
    evm_collections: Vec<EvmNetworkCollectionConfig>,
}

#[derive(Default)]
struct NetworkCollectionDemand {
    bitcoin_addresses: BTreeSet<String>,
    evm_native_accounts: BTreeSet<NormalizedEvmAddress>,
    erc20_sources: BTreeSet<EvmErc20BalanceSourceConfig>,
}

fn compile_collection(portfolio: &PortfolioConfig) -> Result<CompiledCollection, ConfigError> {
    let symbols = portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let networks = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().as_str(), network))
        .collect::<BTreeMap<_, _>>();
    let mut demand = BTreeMap::<String, NetworkCollectionDemand>::new();
    let mut manifest = Vec::new();

    for wallet in &portfolio.wallets {
        let network = networks
            .get(wallet.network_id.as_str())
            .copied()
            .ok_or_else(|| ConfigError::new("portfolio wallet referenced an unknown network"))?;
        let network_demand = demand.entry(wallet.network_id.to_string()).or_default();
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols
                .get(symbol_id.as_str())
                .copied()
                .ok_or_else(|| ConfigError::new("portfolio wallet referenced an unknown symbol"))?;
            let requirement = HoldingRequirementKey {
                wallet_id: wallet.wallet_id.to_string(),
                symbol_id: symbol.symbol_id.to_string(),
                network_id: wallet.network_id.to_string(),
            };
            let source = match (network, &symbol.source) {
                (
                    NetworkConfig::Bitcoin {
                        network_id,
                        bitcoin_network,
                        source_identity,
                        ..
                    },
                    HoldingSourceConfig::Native,
                ) => {
                    let address = wallet.subject.address_str().to_owned();
                    network_demand.bitcoin_addresses.insert(address.clone());
                    HoldingSourceKey::BitcoinNative {
                        network_id: network_id.to_string(),
                        bitcoin_network: bitcoin_network.clone(),
                        semantic_source_identity: source_identity.to_string(),
                        address,
                    }
                }
                (
                    NetworkConfig::Evm {
                        network_id,
                        chain_id,
                        ..
                    },
                    HoldingSourceConfig::Native,
                ) => {
                    let account = wallet
                        .subject
                        .evm_address()
                        .ok_or_else(|| {
                            ConfigError::new("EVM wallet did not contain an EVM address")
                        })?
                        .clone();
                    network_demand.evm_native_accounts.insert(account.clone());
                    HoldingSourceKey::EvmNative {
                        network_id: network_id.to_string(),
                        chain_id: chain_id.get(),
                        account: account.to_string(),
                    }
                }
                (
                    NetworkConfig::Evm {
                        network_id,
                        chain_id,
                        ..
                    },
                    HoldingSourceConfig::Erc20 { contract_address },
                ) => {
                    let account = wallet
                        .subject
                        .evm_address()
                        .ok_or_else(|| {
                            ConfigError::new("EVM wallet did not contain an EVM address")
                        })?
                        .clone();
                    network_demand
                        .erc20_sources
                        .insert(EvmErc20BalanceSourceConfig {
                            contract_address: contract_address.clone(),
                            account: account.clone(),
                        });
                    HoldingSourceKey::EvmErc20 {
                        network_id: network_id.to_string(),
                        chain_id: chain_id.get(),
                        contract_address: contract_address.to_string(),
                        account: account.to_string(),
                    }
                }
                _ => {
                    return Err(ConfigError::new(
                        "portfolio source did not match network family",
                    ))
                }
            };
            manifest.push(
                HoldingManifestEntry::new(requirement, source)
                    .map_err(|error| ConfigError::new(error.to_string()))?,
            );
        }
    }
    manifest.sort_by(|left, right| left.requirement().cmp(right.requirement()));
    mfm_state_portfolio::manifest_identity(&manifest)
        .map_err(|error| ConfigError::new(error.to_string()))?;

    let mut bitcoin_collections = Vec::new();
    let mut evm_collections = Vec::new();
    for (network_id, network_demand) in demand {
        let network = networks
            .get(network_id.as_str())
            .copied()
            .ok_or_else(|| ConfigError::new("compiled demand referenced an unknown network"))?;
        match network {
            NetworkConfig::Bitcoin {
                bitcoin_network,
                source_identity,
                ..
            } => {
                if network_demand.bitcoin_addresses.is_empty()
                    || !network_demand.evm_native_accounts.is_empty()
                    || !network_demand.erc20_sources.is_empty()
                {
                    return Err(ConfigError::new(
                        "Bitcoin demand did not match native source shape",
                    ));
                }
                bitcoin_collections.push(BtcNetworkCollectionConfig {
                    native_balances: BtcNativeBalancesAtAnchorConfig {
                        network: network_id,
                        bitcoin_network: bitcoin_network.clone(),
                        semantic_source_identity: source_identity.to_string(),
                        addresses: network_demand.bitcoin_addresses.into_iter().collect(),
                    },
                });
            }
            NetworkConfig::Evm { .. } => {
                if !network_demand.bitcoin_addresses.is_empty()
                    || (network_demand.evm_native_accounts.is_empty()
                        && network_demand.erc20_sources.is_empty())
                {
                    return Err(ConfigError::new("EVM demand did not match source shape"));
                }
                evm_collections.push(EvmNetworkCollectionConfig {
                    network: network.clone(),
                    native_accounts: network_demand.evm_native_accounts.into_iter().collect(),
                    erc20_sources: network_demand.erc20_sources.into_iter().collect(),
                });
            }
        }
    }
    Ok(CompiledCollection {
        manifest,
        bitcoin_collections,
        evm_collections,
    })
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub portfolio_snapshot_state_registry,
    operation_registry: pub portfolio_snapshot_operation_registry,
    certification: pub register_portfolio_snapshot_certification_descriptors,
    states: [
        AssemblePortfolioCollectionReceiptState,
        mfm_state_portfolio::ResolveSubjectsState,
        mfm_state_portfolio::SelectHoldingsState,
        mfm_state_portfolio::ResolveValuationsState,
        mfm_state_portfolio::AssembleSnapshotState,
        mfm_state_portfolio::ProjectReportState,
        mfm_op_btc_collectors::ResolveBtcJointTipState,
        mfm_op_btc_collectors::ObserveBtcAddressBalanceState,
        mfm_op_btc_collectors::RecordBtcAddressBalanceFactState,
        mfm_op_btc_collectors::AssembleBtcNetworkCollectionReceiptState,
        mfm_op_evm_collectors::ResolveEvmJointTipState,
        mfm_op_evm_collectors::ObserveEvmNativeBalanceState,
        mfm_op_evm_collectors::RecordEvmNativeBalanceFactState,
        mfm_op_evm_collectors::AssembleEvmNativeBalanceBatchReceiptState,
        mfm_op_evm_collectors::ObserveErc20TokenMetadataState,
        mfm_op_evm_collectors::ObserveErc20BalanceState,
        mfm_op_evm_collectors::RecordErc20BalanceFactState,
        mfm_op_evm_collectors::AssembleEvmErc20BalanceBatchReceiptState,
        mfm_op_evm_collectors::AssembleEvmNetworkCollectionReceiptState,
    ],
    operations: [
        PortfolioSnapshotOperation,
        BtcNetworkCollectionOperation,
        mfm_op_btc_collectors::BtcNativeBalancesAtAnchorOperation,
        EvmNetworkCollectionOperation,
        mfm_op_evm_collectors::EvmNativeBalancesAtAnchorOperation,
        mfm_op_evm_collectors::EvmErc20BalancesAtAnchorOperation,
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
    use mfm_portfolio_model::portfolio::NetworkFamilyConfig;
    use mfm_values::NonEmpty;
    use serde_json::json;

    const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
    const TOKEN: &str = "0x0000000000000000000000000000000000000001";
    const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
    const EVM_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn compiler_derives_sorted_logical_manifest_and_family_work() {
        let portfolio = portfolio_config(true, true, true);
        let compiled = compile_collection(&portfolio).expect("compile collection");
        assert_eq!(compiled.manifest.len(), 3);
        assert_eq!(compiled.bitcoin_collections.len(), 1);
        assert_eq!(compiled.evm_collections.len(), 1);
        assert_eq!(compiled.evm_collections[0].native_accounts.len(), 1);
        assert_eq!(compiled.evm_collections[0].erc20_sources.len(), 1);
        assert_eq!(
            compiled.evm_collections[0].erc20_sources[0]
                .contract_address
                .as_str(),
            TOKEN
        );
    }

    #[test]
    fn compiler_keeps_token_only_network_without_native_child_demand() {
        let compiled = compile_collection(&portfolio_config(false, false, true))
            .expect("compile token-only collection");
        assert!(compiled.bitcoin_collections.is_empty());
        assert_eq!(compiled.evm_collections.len(), 1);
        assert!(compiled.evm_collections[0].native_accounts.is_empty());
        assert_eq!(compiled.evm_collections[0].erc20_sources.len(), 1);
    }

    #[test]
    fn snapshot_root_owns_the_complete_objective_and_one_public_output_binding() {
        let config = ValidatedPortfolioConfig::new(portfolio_config(true, true, true))
            .expect("normalize portfolio")
            .into_config();
        let first = portfolio_snapshot_program_draft(config.clone()).expect("first snapshot draft");
        let second =
            portfolio_snapshot_program_draft(config.clone()).expect("second snapshot draft");

        assert_eq!(first, second, "snapshot expansion must be deterministic");
        assert_eq!(first.root_key().as_str(), ROOT_SCOPE);
        assert_eq!(first.public_output_spec().key().as_str(), PUBLIC_OUTPUT_KEY);
        assert_eq!(
            first
                .public_output_spec()
                .outputs()
                .iter()
                .map(|output| output.public_field_path().as_str())
                .collect::<Vec<_>>(),
            ["snapshot", "report"]
        );
        assert!(first
            .operation_lineage()
            .iter()
            .any(|operation| operation.operation_name == "mfm.portfolio.snapshot"));
        assert!(first.operation_lineage().iter().all(|operation| {
            !operation.operation_name.contains("tracker")
                && !operation.operation_name.contains("collect_then_report")
        }));

        mfm_certify::certify_program_draft(&first).expect("snapshot draft certifies");
        let launch = portfolio_snapshot_program_launch_plan(config).expect("snapshot launch plan");
        assert_eq!(launch.draft, first);
        assert!(!launch.config_material.is_empty());
    }

    #[test]
    fn receipt_assembler_requires_the_exact_family_completed_set() {
        let compiled = compile_collection(&portfolio_config(true, true, true))
            .expect("compile mixed collection");
        let bitcoin = bitcoin_receipt(BTC_ADDRESS);
        let evm = evm_receipt(EVM_ACCOUNT, true, true);
        let receipt = assemble_portfolio_collection_receipt(
            &AssemblePortfolioCollectionReceiptConfig {
                manifest: compiled.manifest.clone(),
            },
            AssemblePortfolioCollectionReceiptInput {
                bitcoin_receipts: vec![bitcoin.clone()],
                evm_receipts: vec![evm.clone()],
            },
        )
        .expect("exact family receipts");
        assert_eq!(receipt.holdings().len(), 3);
        assert_eq!(receipt.network_anchors().len(), 2);
        let native_identity = evm
            .native_balance_receipt()
            .expect("native receipt")
            .entries()[0]
            .fact_content_identity();
        let expected_evidence = serde_json::to_value(
            mfm_facts::FactContentIdentityEvidence::from_verified(native_identity),
        )
        .expect("family identity evidence serializes");
        assert!(receipt.holdings().iter().any(|entry| {
            serde_json::to_value(entry.fact_content_identity_evidence())
                .expect("receipt identity evidence serializes")
                == expected_evidence
        }));

        let missing = assemble_portfolio_collection_receipt(
            &AssemblePortfolioCollectionReceiptConfig {
                manifest: compiled.manifest.clone(),
            },
            AssemblePortfolioCollectionReceiptInput {
                bitcoin_receipts: Vec::new(),
                evm_receipts: vec![evm.clone()],
            },
        )
        .expect_err("missing Bitcoin family receipt");
        assert!(missing.to_string().contains("missing required source"));

        let duplicate = assemble_portfolio_collection_receipt(
            &AssemblePortfolioCollectionReceiptConfig {
                manifest: compiled.manifest,
            },
            AssemblePortfolioCollectionReceiptInput {
                bitcoin_receipts: vec![bitcoin.clone(), bitcoin],
                evm_receipts: vec![evm],
            },
        )
        .expect_err("duplicate Bitcoin network receipt");
        assert!(duplicate.to_string().contains("duplicate network receipts"));
    }

    #[test]
    fn receipt_assembler_rejects_unexpected_physical_source_and_empty_demand() {
        let native_only = compile_collection(&portfolio_config(false, true, false))
            .expect("compile native-only collection");
        let unexpected = assemble_portfolio_collection_receipt(
            &AssemblePortfolioCollectionReceiptConfig {
                manifest: native_only.manifest,
            },
            AssemblePortfolioCollectionReceiptInput {
                bitcoin_receipts: Vec::new(),
                evm_receipts: vec![evm_receipt(EVM_ACCOUNT, true, true)],
            },
        )
        .expect_err("unexpected token source");
        assert!(unexpected.to_string().contains("unexpected source"));

        assert!(compile_collection(&portfolio_config(false, false, false)).is_err());
    }

    fn bitcoin_receipt(address: &str) -> BtcNetworkCollectionReceipt {
        let tip = mfm_states_btc::BtcJointTip::new(
            "bitcoin-mainnet",
            "main",
            "public-bitcoin-core",
            850_000,
            "aa".repeat(32),
            "synced",
            "main",
        )
        .expect("Bitcoin tip");
        let fact = mfm_states_btc::BtcAddressBalanceSnapshotFact::try_new(
            mfm_states_btc::BtcAddressBalanceSubject::new(
                "bitcoin-mainnet",
                "main",
                "public-bitcoin-core",
                address,
            )
            .expect("Bitcoin subject"),
            850_000,
            "aa".repeat(32),
            0,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .expect("Bitcoin fact");
        mfm_states_btc::assemble_btc_network_collection_receipt(
            mfm_states_btc::AssembleBtcNetworkCollectionReceiptInput {
                joint_tip: tip,
                balance_facts: NonEmpty::try_from_vec(vec![fact]).expect("one Bitcoin fact"),
            },
        )
        .expect("Bitcoin receipt")
    }

    fn evm_receipt(
        account: &str,
        include_native: bool,
        include_erc20: bool,
    ) -> EvmNetworkCollectionReceipt {
        let network = NetworkConfig::new(
            "ethereum-mainnet".to_owned(),
            NetworkFamilyConfig::Evm,
            Some(1),
            Some(18),
            None,
            None,
            BTreeMap::new(),
        )
        .expect("EVM network");
        let tip =
            mfm_states_evm::EvmJointTip::new("ethereum-mainnet", 1, 20, EVM_HASH).expect("EVM tip");
        let native = include_native.then(|| {
            let fact = mfm_states_evm::EvmAddressNativeBalanceSnapshotFact::try_new(
                mfm_states_evm::EvmAddressNativeBalanceSubject::new("ethereum-mainnet", 1, account)
                    .expect("native subject"),
                20,
                EVM_HASH,
                "0",
                18,
                CoverageStatus::ConfiguredOnly,
                HoldingSourceStatus::Ok,
            )
            .expect("native fact");
            mfm_states_evm::assemble_evm_native_balance_batch_receipt(
                mfm_states_evm::AssembleEvmNativeBalanceBatchReceiptInput {
                    joint_tip: tip.clone(),
                    balance_facts: NonEmpty::try_from_vec(vec![fact]).expect("one native fact"),
                },
            )
            .expect("native receipt")
        });
        let erc20 = include_erc20.then(|| {
            let fact = mfm_states_evm::EvmAddressErc20BalanceSnapshotFact::try_new(
                mfm_states_evm::EvmAddressErc20BalanceSubject::new(
                    "ethereum-mainnet",
                    1,
                    TOKEN,
                    account,
                )
                .expect("token subject"),
                20,
                EVM_HASH,
                "0",
                6,
            )
            .expect("token fact");
            mfm_states_evm::assemble_evm_erc20_balance_batch_receipt(
                mfm_states_evm::AssembleEvmErc20BalanceBatchReceiptInput {
                    joint_tip: tip.clone(),
                    balance_facts: NonEmpty::try_from_vec(vec![fact]).expect("one token fact"),
                },
            )
            .expect("token receipt")
        });
        mfm_states_evm::assemble_evm_network_collection_receipt(
            &mfm_states_evm::AssembleEvmNetworkCollectionReceiptConfig {
                network,
                native_balance_required: include_native,
                erc20_balance_required: include_erc20,
            },
            mfm_states_evm::AssembleEvmNetworkCollectionReceiptInput {
                joint_tip: tip,
                native_balance_receipts: native.into_iter().collect(),
                erc20_balance_receipts: erc20.into_iter().collect(),
            },
        )
        .expect("EVM network receipt")
    }

    fn portfolio_config(
        include_btc: bool,
        include_evm_native: bool,
        include_evm_token: bool,
    ) -> PortfolioConfig {
        let mut networks = Vec::new();
        let mut wallets = Vec::new();
        let mut symbols = Vec::new();
        if include_evm_native || include_evm_token {
            networks.push(json!({
                "network_id": "ethereum-mainnet",
                "family": "evm",
                "chain_id": 1,
                "native_decimals": 18,
                "metadata": {}
            }));
            let mut symbol_ids = Vec::new();
            if include_evm_native {
                symbol_ids.push("eth.native.ethereum-mainnet");
                symbols.push(symbol(
                    "eth.native.ethereum-mainnet",
                    "ethereum-mainnet",
                    json!({"kind": "native"}),
                ));
            }
            if include_evm_token {
                symbol_ids.push("usdc.wallet.ethereum-mainnet");
                symbols.push(symbol(
                    "usdc.wallet.ethereum-mainnet",
                    "ethereum-mainnet",
                    json!({"kind": "erc20", "contract_address": TOKEN}),
                ));
            }
            wallets.push(json!({
                "wallet_id": "wallet_eth",
                "network_id": "ethereum-mainnet",
                "symbol_ids": symbol_ids,
                "subject": {"kind": "evm_address", "address": EVM_ACCOUNT},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            }));
        }
        if include_btc {
            networks.push(json!({
                "network_id": "bitcoin-mainnet",
                "family": "bitcoin",
                "bitcoin_network": "main",
                "source_identity": "public-bitcoin-core",
                "metadata": {}
            }));
            wallets.push(json!({
                "wallet_id": "wallet_btc",
                "network_id": "bitcoin-mainnet",
                "symbol_ids": ["btc.native.bitcoin-mainnet"],
                "subject": {"kind": "bitcoin_address", "address": "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            }));
            symbols.push(symbol(
                "btc.native.bitcoin-mainnet",
                "bitcoin-mainnet",
                json!({"kind": "native"}),
            ));
        }
        serde_json::from_value(json!({
            "portfolio_id": "composition-test",
            "quote_codes": ["USD"],
            "networks": networks,
            "wallets": wallets,
            "symbol_configs": symbols,
            "metadata": {}
        }))
        .expect("portfolio fixture")
    }

    fn symbol(symbol_id: &str, network_id: &str, source: serde_json::Value) -> serde_json::Value {
        json!({
            "symbol_id": symbol_id,
            "display_symbol": symbol_id,
            "network_id": network_id,
            "source": source,
            "valuation": {
                "quotes": [{
                    "quote": "USD",
                    "priced_symbol_id": symbol_id,
                    "unit_price_dec": "1.00"
                }]
            },
            "metadata": {}
        })
    }
}
