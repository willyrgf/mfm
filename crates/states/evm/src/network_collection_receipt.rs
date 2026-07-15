//! EVM network-level receipt assembly from anchored resource receipts.
//!
//! The network receipt deliberately contains typed native and ERC-20 resource receipts rather
//! than a cross-family source enum. The coordinator proves their common anchor after every
//! resource child completed its managed fact records.

use mfm_capabilities::NoCaps;
use mfm_effects::Pure;
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig};
use mfm_program::{NoContext, PureState, StateError, StateResult, StateSpec, ValidatedConfig};
use mfm_program_derive::{MfmConfig, MfmValue, StateInput};
use serde::{de, Deserialize, Serialize};

use crate::native_balance_collect::{state_kind, state_version};
use crate::{
    canonical_evm_block_hash, EvmErc20BalanceBatchReceipt, EvmJointTip,
    EvmNativeBalanceBatchReceipt, EvmStateError,
};

/// Certified expectation for the resource receipts required by one EVM network collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.evm.state.config.assemble_network_collection_receipt",
    validate = "validate_assemble_evm_network_collection_receipt_config"
)]
pub struct AssembleEvmNetworkCollectionReceiptConfig {
    /// Exact semantic EVM network for this coordinator.
    pub network: NetworkConfig,
    /// Whether the coordinator requires one native-balance resource receipt.
    pub native_balance_required: bool,
    /// Whether the coordinator requires one ERC-20-balance resource receipt.
    pub erc20_balance_required: bool,
}

/// Validates the fixed resource shape of an EVM network receipt assembler.
pub fn validate_assemble_evm_network_collection_receipt_config(
    config: &AssembleEvmNetworkCollectionReceiptConfig,
) -> Result<(), String> {
    if config.network.family() != NetworkFamilyConfig::Evm {
        return Err("EVM network collection receipt requires an EVM network".to_owned());
    }
    if config.network.chain_id_u64().is_none() || config.network.native_decimals().is_none() {
        return Err(
            "EVM network collection receipt requires chain_id and native_decimals".to_owned(),
        );
    }
    if !config.native_balance_required && !config.erc20_balance_required {
        return Err("EVM network collection receipt requires at least one resource".to_owned());
    }
    Ok(())
}

/// Input for assembling one EVM network collection receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.assemble_network_collection_receipt")]
pub struct AssembleEvmNetworkCollectionReceiptInput {
    /// The shared joint tip consumed by every resource child.
    pub joint_tip: EvmJointTip,
    /// Zero or one completed native-balance resource receipt.
    pub native_balance_receipts: Vec<EvmNativeBalanceBatchReceipt>,
    /// Zero or one completed ERC-20-balance resource receipt.
    pub erc20_balance_receipts: Vec<EvmErc20BalanceBatchReceipt>,
}

/// Exact source-near collection receipt for one EVM network and one shared anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "network_collection_receipt",
    version = "1",
    schema = "mfm.evm.collection.network_receipt"
)]
pub struct EvmNetworkCollectionReceipt {
    network: String,
    chain_id: u64,
    block_number: u64,
    block_hash: String,
    native_balance_receipt: Option<EvmNativeBalanceBatchReceipt>,
    erc20_balance_receipt: Option<EvmErc20BalanceBatchReceipt>,
}

impl EvmNetworkCollectionReceipt {
    fn from_resource_receipts(
        network: String,
        chain_id: u64,
        block_number: u64,
        block_hash: String,
        native_balance_receipt: Option<EvmNativeBalanceBatchReceipt>,
        erc20_balance_receipt: Option<EvmErc20BalanceBatchReceipt>,
    ) -> Result<Self, EvmStateError> {
        if network.trim().is_empty() || chain_id == 0 {
            return Err(EvmStateError::InvalidInput {
                reason: "EVM network collection receipt requires a non-empty network and chain"
                    .to_owned(),
            });
        }
        if native_balance_receipt.is_none() && erc20_balance_receipt.is_none() {
            return Err(EvmStateError::InvalidInput {
                reason: "EVM network collection receipt requires at least one resource receipt"
                    .to_owned(),
            });
        }
        let block_hash = canonical_evm_block_hash(block_hash)?;
        if let Some(receipt) = &native_balance_receipt {
            if receipt.network() != network
                || receipt.chain_id() != chain_id
                || receipt.block_number() != block_number
                || receipt.block_hash() != block_hash
            {
                return Err(EvmStateError::InvalidInput {
                    reason: "EVM native resource receipt did not match the network anchor"
                        .to_owned(),
                });
            }
        }
        if let Some(receipt) = &erc20_balance_receipt {
            if receipt.network() != network
                || receipt.chain_id() != chain_id
                || receipt.block_number() != block_number
                || receipt.block_hash() != block_hash
            {
                return Err(EvmStateError::InvalidInput {
                    reason: "EVM ERC-20 resource receipt did not match the network anchor"
                        .to_owned(),
                });
            }
        }
        Ok(Self {
            network,
            chain_id,
            block_number,
            block_hash,
            native_balance_receipt,
            erc20_balance_receipt,
        })
    }

    /// Returns the semantic EVM network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the exact shared anchor block number.
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the exact shared canonical anchor block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the native resource receipt when native demand was configured.
    pub fn native_balance_receipt(&self) -> Option<&EvmNativeBalanceBatchReceipt> {
        self.native_balance_receipt.as_ref()
    }

    /// Returns the ERC-20 resource receipt when token demand was configured.
    pub fn erc20_balance_receipt(&self) -> Option<&EvmErc20BalanceBatchReceipt> {
        self.erc20_balance_receipt.as_ref()
    }
}

impl<'de> Deserialize<'de> for EvmNetworkCollectionReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            network: String,
            chain_id: u64,
            block_number: u64,
            block_hash: String,
            native_balance_receipt: Option<EvmNativeBalanceBatchReceipt>,
            erc20_balance_receipt: Option<EvmErc20BalanceBatchReceipt>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::from_resource_receipts(
            wire.network,
            wire.chain_id,
            wire.block_number,
            wire.block_hash,
            wire.native_balance_receipt,
            wire.erc20_balance_receipt,
        )
        .map_err(de::Error::custom)
    }
}

/// Pure state that proves the exact resource shape and common anchor for an EVM network.
pub struct AssembleEvmNetworkCollectionReceiptState {
    config: AssembleEvmNetworkCollectionReceiptConfig,
}

impl StateSpec for AssembleEvmNetworkCollectionReceiptState {
    type Config = AssembleEvmNetworkCollectionReceiptConfig;
    type Context = NoContext;
    type Input = AssembleEvmNetworkCollectionReceiptInput;
    type Output = EvmNetworkCollectionReceipt;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("network_collection.assemble_receipt")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("network_collection.assemble_receipt")
    }

    fn name() -> &'static str {
        "mfm.evm.network_collection.assemble_receipt"
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for AssembleEvmNetworkCollectionReceiptState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        assemble_evm_network_collection_receipt(&self.config, input)
    }
}

/// Derives one exact EVM network receipt from completed typed resource receipts.
pub fn assemble_evm_network_collection_receipt(
    config: &AssembleEvmNetworkCollectionReceiptConfig,
    input: AssembleEvmNetworkCollectionReceiptInput,
) -> StateResult<EvmNetworkCollectionReceipt> {
    validate_assemble_evm_network_collection_receipt_config(config)
        .map_err(|reason| StateError::from(EvmStateError::InvalidInput { reason }))?;
    let expected_network = config.network.network_id().as_str();
    let expected_chain_id = config.network.chain_id_u64().ok_or_else(|| {
        StateError::from(EvmStateError::InvalidInput {
            reason: "EVM network collection receipt config did not contain a chain id".to_owned(),
        })
    })?;
    if input.joint_tip.network() != expected_network
        || input.joint_tip.chain_id() != expected_chain_id
        || !input.joint_tip.is_admissible_for_balance_write()
    {
        return Err(StateError::from(EvmStateError::InvalidInput {
            reason: "EVM network collection joint tip did not match the certified network"
                .to_owned(),
        }));
    }

    let expected_native_count = usize::from(config.native_balance_required);
    let expected_erc20_count = usize::from(config.erc20_balance_required);
    if input.native_balance_receipts.len() != expected_native_count
        || input.erc20_balance_receipts.len() != expected_erc20_count
    {
        return Err(StateError::from(EvmStateError::InvalidInput {
            reason: "EVM network collection resource receipt shape did not match certified demand"
                .to_owned(),
        }));
    }

    let native_balance_receipt = input.native_balance_receipts.into_iter().next();
    let erc20_balance_receipt = input.erc20_balance_receipts.into_iter().next();
    EvmNetworkCollectionReceipt::from_resource_receipts(
        input.joint_tip.network().to_owned(),
        input.joint_tip.chain_id(),
        input.joint_tip.block_number(),
        input.joint_tip.block_hash().to_owned(),
        native_balance_receipt,
        erc20_balance_receipt,
    )
    .map_err(StateError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
    use mfm_values::NonEmpty;

    const HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const ACCOUNT: &str = "0x0000000000000000000000000000000000000001";
    const TOKEN: &str = "0x0000000000000000000000000000000000000002";

    fn network() -> NetworkConfig {
        NetworkConfig::new(
            "ethereum-mainnet".to_owned(),
            NetworkFamilyConfig::Evm,
            Some(1),
            Some(18),
            None,
            None,
            std::collections::BTreeMap::new(),
        )
        .expect("network")
    }

    fn tip() -> EvmJointTip {
        EvmJointTip::new("ethereum-mainnet", 1, 100, HASH).expect("tip")
    }

    fn native_receipt() -> EvmNativeBalanceBatchReceipt {
        let fact = crate::EvmAddressNativeBalanceSnapshotFact::try_new(
            crate::EvmAddressNativeBalanceSubject::new("ethereum-mainnet", 1, ACCOUNT)
                .expect("subject"),
            100,
            HASH,
            "0",
            18,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .expect("fact");
        crate::assemble_evm_native_balance_batch_receipt(
            crate::AssembleEvmNativeBalanceBatchReceiptInput {
                joint_tip: tip(),
                balance_facts: NonEmpty::try_from_vec(vec![fact]).expect("fact"),
            },
        )
        .expect("native receipt")
    }

    fn erc20_receipt() -> EvmErc20BalanceBatchReceipt {
        let fact = crate::EvmAddressErc20BalanceSnapshotFact::try_new(
            crate::EvmAddressErc20BalanceSubject::new("ethereum-mainnet", 1, TOKEN, ACCOUNT)
                .expect("subject"),
            100,
            HASH,
            "0",
            6,
        )
        .expect("fact");
        crate::assemble_evm_erc20_balance_batch_receipt(
            crate::AssembleEvmErc20BalanceBatchReceiptInput {
                joint_tip: tip(),
                balance_facts: NonEmpty::try_from_vec(vec![fact]).expect("fact"),
            },
        )
        .expect("ERC-20 receipt")
    }

    #[test]
    fn network_receipt_requires_exact_nonempty_resource_shape_and_shared_anchor() {
        let config = AssembleEvmNetworkCollectionReceiptConfig {
            network: network(),
            native_balance_required: true,
            erc20_balance_required: true,
        };
        let receipt = assemble_evm_network_collection_receipt(
            &config,
            AssembleEvmNetworkCollectionReceiptInput {
                joint_tip: tip(),
                native_balance_receipts: vec![native_receipt()],
                erc20_balance_receipts: vec![erc20_receipt()],
            },
        )
        .expect("mixed receipt");
        assert!(receipt.native_balance_receipt().is_some());
        assert!(receipt.erc20_balance_receipt().is_some());
        assert_eq!(receipt.block_hash(), HASH);

        assert!(assemble_evm_network_collection_receipt(
            &config,
            AssembleEvmNetworkCollectionReceiptInput {
                joint_tip: tip(),
                native_balance_receipts: vec![native_receipt()],
                erc20_balance_receipts: Vec::new(),
            },
        )
        .is_err());

        let mut tampered = serde_json::to_value(&receipt).expect("receipt JSON");
        tampered["block_hash"] =
            serde_json::json!("0x2222222222222222222222222222222222222222222222222222222222222222");
        assert!(serde_json::from_value::<EvmNetworkCollectionReceipt>(tampered).is_err());
    }
}
