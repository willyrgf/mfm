#![warn(missing_docs)]
//! Deterministic internal EVM collection operations.
//!
//! A network coordinator resolves exactly one joint tip, then delegates native and ERC-20
//! source demand to separate reusable at-anchor resource operations. No operation in this crate
//! exposes a standalone root or public-output wrapper.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_portfolio_model::ids::NormalizedEvmAddress;
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig};
use mfm_program::{
    Handle, NoContext, NonEmptyHandles, Operation, OperationExpansion, OperationKey, StateKey,
};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput};
pub use mfm_states_evm::{
    AssembleEvmErc20BalanceBatchReceiptConfig, AssembleEvmErc20BalanceBatchReceiptInput,
    AssembleEvmErc20BalanceBatchReceiptInputHandles, AssembleEvmErc20BalanceBatchReceiptState,
    AssembleEvmNativeBalanceBatchReceiptConfig, AssembleEvmNativeBalanceBatchReceiptInput,
    AssembleEvmNativeBalanceBatchReceiptInputHandles, AssembleEvmNativeBalanceBatchReceiptState,
    AssembleEvmNetworkCollectionReceiptConfig, AssembleEvmNetworkCollectionReceiptInput,
    AssembleEvmNetworkCollectionReceiptInputHandles, AssembleEvmNetworkCollectionReceiptState,
    EvmAddressErc20BalanceSnapshotFact, EvmAddressNativeBalanceSnapshotFact,
    EvmErc20BalanceBatchReceipt, EvmJointTip, EvmNativeBalanceBatchReceipt,
    EvmNetworkCollectionReceipt, ObserveErc20BalanceConfig, ObserveErc20BalanceInput,
    ObserveErc20BalanceInputHandles, ObserveErc20BalanceState, ObserveErc20TokenMetadataConfig,
    ObserveErc20TokenMetadataInput, ObserveErc20TokenMetadataInputHandles,
    ObserveErc20TokenMetadataState, ObserveEvmNativeBalanceConfig, ObserveEvmNativeBalanceInput,
    ObserveEvmNativeBalanceInputHandles, ObserveEvmNativeBalanceState,
    RecordErc20BalanceFactConfig, RecordErc20BalanceFactInput, RecordErc20BalanceFactInputHandles,
    RecordErc20BalanceFactState, RecordEvmNativeBalanceFactConfig, RecordEvmNativeBalanceFactInput,
    RecordEvmNativeBalanceFactInputHandles, RecordEvmNativeBalanceFactState,
    ResolveEvmJointTipConfig, ResolveEvmJointTipInput, ResolveEvmJointTipInputHandles,
    ResolveEvmJointTipState, EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS,
    EVM_ERC20_METADATA_OBSERVE_SOURCE_READS, EVM_JOINT_TIP_SOURCE_READS,
    EVM_NATIVE_BALANCE_COVERAGE, EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS,
};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.evm";
const EVM_NETWORK_COLLECTION_OP_KIND_NAME: &str = "evm_network_collection";
const EVM_NETWORK_COLLECTION_OP_VERSION: &str = "mfm.evm.operation.evm_network_collection.v1";
const EVM_NATIVE_AT_ANCHOR_OP_KIND_NAME: &str = "evm_native_balances_at_anchor";
const EVM_NATIVE_AT_ANCHOR_OP_VERSION: &str = "mfm.evm.operation.evm_native_balances_at_anchor.v1";
const EVM_ERC20_AT_ANCHOR_OP_KIND_NAME: &str = "evm_erc20_balances_at_anchor";
const EVM_ERC20_AT_ANCHOR_OP_VERSION: &str = "mfm.evm.operation.evm_erc20_balances_at_anchor.v1";

/// One exact ERC-20 account source demanded at a shared EVM network anchor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "erc20-balance-source-config",
    schema = "mfm.evm.operation.erc20_balance_source_config"
)]
pub struct EvmErc20BalanceSourceConfig {
    /// Canonical non-zero ERC-20 contract address.
    pub contract_address: NormalizedEvmAddress,
    /// Canonical EVM account whose balance is required.
    pub account: NormalizedEvmAddress,
}

impl EvmErc20BalanceSourceConfig {
    fn validate(&self) -> Result<(), String> {
        if self.contract_address.is_zero() {
            return Err("erc20 contract_address must not be the zero address".to_owned());
        }
        Ok(())
    }
}

/// Reusable EVM native source demand that must consume a coordinator-owned tip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "native-balances-at-anchor-config",
    schema = "mfm.evm.operation.config.native_balances_at_anchor",
    validate = "validate_evm_native_balances_at_anchor_config"
)]
pub struct EvmNativeBalancesAtAnchorConfig {
    /// Exact semantic EVM network including its certified native scale.
    pub network: NetworkConfig,
    /// Strictly sorted, unique canonical EVM accounts.
    pub accounts: Vec<NormalizedEvmAddress>,
}

impl EvmNativeBalancesAtAnchorConfig {
    /// Builds the exact state-owned native observation policy for one account.
    pub fn observe_config_for_account(
        &self,
        account: &NormalizedEvmAddress,
    ) -> ObserveEvmNativeBalanceConfig {
        ObserveEvmNativeBalanceConfig {
            network: self.network.clone(),
            account: account.as_str().to_owned(),
            coverage: EVM_NATIVE_BALANCE_COVERAGE.to_owned(),
            max_source_reads: NonZeroU64::new(EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS)
                .expect("non-zero EVM native observation policy"),
        }
    }
}

/// Validates deterministic EVM native source demand.
pub fn validate_evm_native_balances_at_anchor_config(
    config: &EvmNativeBalancesAtAnchorConfig,
) -> Result<(), String> {
    validate_evm_network(&config.network)?;
    if config.accounts.is_empty() {
        return Err("accounts must contain at least one canonical EVM address".to_owned());
    }
    let mut previous: Option<&NormalizedEvmAddress> = None;
    for account in &config.accounts {
        if previous.is_some_and(|prior| prior >= account) {
            return Err("accounts must be strictly sorted and unique".to_owned());
        }
        mfm_states_evm::validate_observe_evm_native_balance_config(
            &config.observe_config_for_account(account),
        )?;
        previous = Some(account);
    }
    Ok(())
}

/// Reusable EVM ERC-20 source demand that must consume a coordinator-owned tip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "erc20-balances-at-anchor-config",
    schema = "mfm.evm.operation.config.erc20_balances_at_anchor",
    validate = "validate_evm_erc20_balances_at_anchor_config"
)]
pub struct EvmErc20BalancesAtAnchorConfig {
    /// Exact semantic EVM network and its certified chain identity.
    pub network: NetworkConfig,
    /// Strictly sorted, unique `(contract_address, account)` token sources.
    pub sources: Vec<EvmErc20BalanceSourceConfig>,
}

impl EvmErc20BalancesAtAnchorConfig {
    /// Builds the exact state-owned metadata observation policy for one token.
    pub fn metadata_config_for_contract(
        &self,
        contract_address: &NormalizedEvmAddress,
    ) -> ObserveErc20TokenMetadataConfig {
        ObserveErc20TokenMetadataConfig {
            network: self.network.clone(),
            contract_address: contract_address.clone(),
            max_source_reads: NonZeroU64::new(EVM_ERC20_METADATA_OBSERVE_SOURCE_READS)
                .expect("non-zero EVM ERC-20 metadata policy"),
        }
    }

    /// Builds the exact state-owned balance observation policy for one token source.
    pub fn balance_config_for_source(
        &self,
        source: &EvmErc20BalanceSourceConfig,
    ) -> ObserveErc20BalanceConfig {
        ObserveErc20BalanceConfig {
            network: self.network.clone(),
            contract_address: source.contract_address.clone(),
            account: source.account.clone(),
            max_source_reads: NonZeroU64::new(EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS)
                .expect("non-zero EVM ERC-20 balance policy"),
        }
    }
}

/// Validates deterministic EVM ERC-20 source demand.
pub fn validate_evm_erc20_balances_at_anchor_config(
    config: &EvmErc20BalancesAtAnchorConfig,
) -> Result<(), String> {
    validate_evm_network(&config.network)?;
    if config.sources.is_empty() {
        return Err("sources must contain at least one ERC-20 token source".to_owned());
    }
    let mut previous: Option<&EvmErc20BalanceSourceConfig> = None;
    for source in &config.sources {
        if previous.is_some_and(|prior| prior >= source) {
            return Err("ERC-20 sources must be strictly sorted and unique".to_owned());
        }
        source.validate()?;
        mfm_states_evm::validate_observe_erc20_token_metadata_config(
            &config.metadata_config_for_contract(&source.contract_address),
        )?;
        mfm_states_evm::validate_observe_erc20_balance_config(
            &config.balance_config_for_source(source),
        )?;
        previous = Some(source);
    }
    Ok(())
}

/// Certified source demand for one EVM network coordinator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "network-collection-config",
    schema = "mfm.evm.operation.config.network_collection",
    validate = "validate_evm_network_collection_config"
)]
pub struct EvmNetworkCollectionConfig {
    /// Exact semantic EVM network used by all resource children.
    pub network: NetworkConfig,
    /// Strictly sorted native accounts; empty when native demand is absent.
    pub native_accounts: Vec<NormalizedEvmAddress>,
    /// Strictly sorted token sources; empty when ERC-20 demand is absent.
    pub erc20_sources: Vec<EvmErc20BalanceSourceConfig>,
}

impl EvmNetworkCollectionConfig {
    /// Builds the exact state-owned joint-tip resolve policy for this network.
    pub fn joint_tip_config(&self) -> Result<ResolveEvmJointTipConfig, String> {
        let chain_id = evm_chain_id(&self.network)?;
        Ok(ResolveEvmJointTipConfig {
            network: self.network.network_id().as_str().to_owned(),
            chain_id,
            max_source_reads: NonZeroU64::new(EVM_JOINT_TIP_SOURCE_READS)
                .expect("non-zero EVM joint-tip policy"),
        })
    }

    /// Returns native resource demand when the network needs native balances.
    pub fn native_balances_config(&self) -> Option<EvmNativeBalancesAtAnchorConfig> {
        (!self.native_accounts.is_empty()).then(|| EvmNativeBalancesAtAnchorConfig {
            network: self.network.clone(),
            accounts: self.native_accounts.clone(),
        })
    }

    /// Returns ERC-20 resource demand when the network needs token balances.
    pub fn erc20_balances_config(&self) -> Option<EvmErc20BalancesAtAnchorConfig> {
        (!self.erc20_sources.is_empty()).then(|| EvmErc20BalancesAtAnchorConfig {
            network: self.network.clone(),
            sources: self.erc20_sources.clone(),
        })
    }
}

/// Validates the deterministic EVM network collection shape.
pub fn validate_evm_network_collection_config(
    config: &EvmNetworkCollectionConfig,
) -> Result<(), String> {
    validate_evm_network(&config.network)?;
    if config.native_accounts.is_empty() && config.erc20_sources.is_empty() {
        return Err("EVM network collection requires native or ERC-20 source demand".to_owned());
    }
    if let Some(native) = config.native_balances_config() {
        validate_evm_native_balances_at_anchor_config(&native)?;
    }
    if let Some(erc20) = config.erc20_balances_config() {
        validate_evm_erc20_balances_at_anchor_config(&erc20)?;
    }
    mfm_states_evm::validate_resolve_evm_joint_tip_config(&config.joint_tip_config()?)
}

fn validate_evm_network(network: &NetworkConfig) -> Result<(), String> {
    if network.family() != NetworkFamilyConfig::Evm {
        return Err("EVM collection requires an EVM network".to_owned());
    }
    if network.chain_id_u64().is_none() || network.native_decimals().is_none() {
        return Err("EVM collection network requires chain_id and native_decimals".to_owned());
    }
    Ok(())
}

fn evm_chain_id(network: &NetworkConfig) -> Result<u64, String> {
    validate_evm_network(network)?;
    network
        .chain_id_u64()
        .ok_or_else(|| "EVM collection network is missing chain_id".to_owned())
}

/// Output from an at-anchor EVM native resource child.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.operation_outputs.native_balances_at_anchor")]
pub struct EvmNativeBalancesAtAnchorOutputs<'program, 'scope> {
    /// Exact native resource receipt after all managed fact records completed.
    pub receipt: Handle<'program, 'scope, EvmNativeBalanceBatchReceipt>,
}

/// Output from an at-anchor EVM ERC-20 resource child.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.operation_outputs.erc20_balances_at_anchor")]
pub struct EvmErc20BalancesAtAnchorOutputs<'program, 'scope> {
    /// Exact ERC-20 resource receipt after all managed fact records completed.
    pub receipt: Handle<'program, 'scope, EvmErc20BalanceBatchReceipt>,
}

/// Output from one EVM network coordinator.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.operation_outputs.network_collection")]
pub struct EvmNetworkCollectionOutputs<'program, 'scope> {
    /// One joint tip shared by all required resource children.
    pub joint_tip: Handle<'program, 'scope, EvmJointTip>,
    /// Exact non-empty EVM network collection receipt.
    pub receipt: Handle<'program, 'scope, EvmNetworkCollectionReceipt>,
}

/// Reusable at-anchor EVM native balance resource operation.
pub struct EvmNativeBalancesAtAnchorOperation;

impl Operation for EvmNativeBalancesAtAnchorOperation {
    type Config = EvmNativeBalancesAtAnchorConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, EvmJointTip>;
    type Output<'program, 'scope> = EvmNativeBalancesAtAnchorOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            EVM_NATIVE_AT_ANCHOR_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.evm.operation:evm_native_balances_at_anchor"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(EVM_NATIVE_AT_ANCHOR_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.evm_native_balances_at_anchor"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        joint_tip: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let mut fact_handles = Vec::with_capacity(config.accounts.len());
        for (index, account) in config.accounts.iter().enumerate() {
            let observation = builder.state::<ObserveEvmNativeBalanceState, _>(
                StateKey::new(format!("observe_native_balance_{index}"))?,
                NoContext,
                config.observe_config_for_account(account),
                ObserveEvmNativeBalanceInputHandles {
                    joint_tip: joint_tip.clone(),
                },
            )?;
            fact_handles.push(builder.state::<RecordEvmNativeBalanceFactState, _>(
                StateKey::new(format!("record_native_balance_{index}"))?,
                NoContext,
                RecordEvmNativeBalanceFactConfig {},
                RecordEvmNativeBalanceFactInputHandles { observation },
            )?);
        }
        let balance_facts = NonEmptyHandles::try_from_vec(fact_handles)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let receipt = builder.state::<AssembleEvmNativeBalanceBatchReceiptState, _>(
            StateKey::new("assemble_native_balance_receipt")?,
            NoContext,
            AssembleEvmNativeBalanceBatchReceiptConfig {},
            AssembleEvmNativeBalanceBatchReceiptInputHandles {
                joint_tip,
                balance_facts,
            },
        )?;
        Ok(EvmNativeBalancesAtAnchorOutputs { receipt })
    }
}

/// Reusable at-anchor EVM ERC-20 balance resource operation.
pub struct EvmErc20BalancesAtAnchorOperation;

impl Operation for EvmErc20BalancesAtAnchorOperation {
    type Config = EvmErc20BalancesAtAnchorConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, EvmJointTip>;
    type Output<'program, 'scope> = EvmErc20BalancesAtAnchorOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            EVM_ERC20_AT_ANCHOR_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.evm.operation:evm_erc20_balances_at_anchor"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(EVM_ERC20_AT_ANCHOR_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.evm_erc20_balances_at_anchor"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        joint_tip: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let mut metadata_by_contract = BTreeMap::new();
        for (index, source) in config.sources.iter().enumerate() {
            if metadata_by_contract.contains_key(&source.contract_address) {
                continue;
            }
            let metadata = builder.state::<ObserveErc20TokenMetadataState, _>(
                StateKey::new(format!("observe_erc20_metadata_{index}"))?,
                NoContext,
                config.metadata_config_for_contract(&source.contract_address),
                ObserveErc20TokenMetadataInputHandles {
                    joint_tip: joint_tip.clone(),
                },
            )?;
            metadata_by_contract.insert(source.contract_address.clone(), metadata);
        }

        let mut fact_handles = Vec::with_capacity(config.sources.len());
        for (index, source) in config.sources.iter().enumerate() {
            let metadata = metadata_by_contract
                .get(&source.contract_address)
                .ok_or_else(|| {
                    mfm_program::PlanError::Key(
                        "ERC-20 metadata handle was missing for a configured token".to_owned(),
                    )
                })?
                .clone();
            let observation = builder.state::<ObserveErc20BalanceState, _>(
                StateKey::new(format!("observe_erc20_balance_{index}"))?,
                NoContext,
                config.balance_config_for_source(source),
                ObserveErc20BalanceInputHandles {
                    joint_tip: joint_tip.clone(),
                    metadata,
                },
            )?;
            fact_handles.push(builder.state::<RecordErc20BalanceFactState, _>(
                StateKey::new(format!("record_erc20_balance_{index}"))?,
                NoContext,
                RecordErc20BalanceFactConfig {},
                RecordErc20BalanceFactInputHandles { observation },
            )?);
        }
        let balance_facts = NonEmptyHandles::try_from_vec(fact_handles)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let receipt = builder.state::<AssembleEvmErc20BalanceBatchReceiptState, _>(
            StateKey::new("assemble_erc20_balance_receipt")?,
            NoContext,
            AssembleEvmErc20BalanceBatchReceiptConfig {},
            AssembleEvmErc20BalanceBatchReceiptInputHandles {
                joint_tip,
                balance_facts,
            },
        )?;
        Ok(EvmErc20BalancesAtAnchorOutputs { receipt })
    }
}

/// Network coordinator that resolves one EVM tip and fans it into required resource children.
pub struct EvmNetworkCollectionOperation;

impl Operation for EvmNetworkCollectionOperation {
    type Config = EvmNetworkCollectionConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = EvmNetworkCollectionOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            EVM_NETWORK_COLLECTION_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.evm.operation:evm_network_collection"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(EVM_NETWORK_COLLECTION_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.evm_network_collection"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let native_config = config.native_balances_config();
        let erc20_config = config.erc20_balances_config();
        let joint_tip = builder.state::<ResolveEvmJointTipState, _>(
            StateKey::new("resolve_joint_tip")?,
            NoContext,
            config
                .joint_tip_config()
                .map_err(mfm_program::PlanError::Key)?,
            ResolveEvmJointTipInputHandles {},
        )?;
        let native_balance_receipts = match native_config {
            Some(native_config) => vec![
                builder
                    .call::<EvmNativeBalancesAtAnchorOperation, _>(
                        OperationKey::new("native_balances_at_anchor")?,
                        EvmNativeBalancesAtAnchorOperation,
                        native_config,
                        joint_tip.clone(),
                    )?
                    .receipt,
            ],
            None => Vec::new(),
        };
        let erc20_balance_receipts = match erc20_config {
            Some(erc20_config) => vec![
                builder
                    .call::<EvmErc20BalancesAtAnchorOperation, _>(
                        OperationKey::new("erc20_balances_at_anchor")?,
                        EvmErc20BalancesAtAnchorOperation,
                        erc20_config,
                        joint_tip.clone(),
                    )?
                    .receipt,
            ],
            None => Vec::new(),
        };
        let receipt = builder.state::<AssembleEvmNetworkCollectionReceiptState, _>(
            StateKey::new("assemble_network_collection_receipt")?,
            NoContext,
            AssembleEvmNetworkCollectionReceiptConfig {
                network: config.network,
                native_balance_required: !native_balance_receipts.is_empty(),
                erc20_balance_required: !erc20_balance_receipts.is_empty(),
            },
            AssembleEvmNetworkCollectionReceiptInputHandles {
                joint_tip: joint_tip.clone(),
                native_balance_receipts,
                erc20_balance_receipts,
            },
        )?;
        Ok(EvmNetworkCollectionOutputs { joint_tip, receipt })
    }
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub evm_collectors_state_registry,
    operation_registry: pub evm_collectors_operation_registry,
    certification: pub register_evm_collectors_certification_descriptors,
    states: [
        ResolveEvmJointTipState,
        ObserveEvmNativeBalanceState,
        RecordEvmNativeBalanceFactState,
        AssembleEvmNativeBalanceBatchReceiptState,
        ObserveErc20TokenMetadataState,
        ObserveErc20BalanceState,
        RecordErc20BalanceFactState,
        AssembleEvmErc20BalanceBatchReceiptState,
        AssembleEvmNetworkCollectionReceiptState,
    ],
    operations: [
        EvmNativeBalancesAtAnchorOperation,
        EvmErc20BalancesAtAnchorOperation,
        EvmNetworkCollectionOperation,
    ],
}

#[cfg(test)]
#[path = "evm_collectors_tests.rs"]
mod tests;
