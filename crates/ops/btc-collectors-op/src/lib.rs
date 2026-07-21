#![warn(missing_docs)]
//! Deterministic Bitcoin balance-collection operations.
//!
//! This crate owns the internal anchored native-balance topology: one network tip, per-source
//! facts, and one checked receipt.
//!
//! It performs no live IO and does not register app assembly, adapters, transports, storage,
//! binaries, or recurring scheduler policy.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_btc_collectors::{
//!     validate_btc_network_collection_config, BtcNativeBalancesAtAnchorConfig,
//!     BtcNetworkCollectionConfig,
//! };
//!
//! let config = BtcNetworkCollectionConfig {
//!     native_balances: BtcNativeBalancesAtAnchorConfig {
//!         network: "bitcoin-mainnet".to_owned(),
//!         bitcoin_network: "main".to_owned(),
//!         semantic_source_identity: "public-bitcoin-core".to_owned(),
//!         addresses: vec!["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned()],
//!     },
//! };
//! validate_btc_network_collection_config(&config).unwrap();
//! ```

use std::num::NonZeroU64;

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{
    Handle, NoContext, NonEmptyHandles, Operation, OperationExpansion, OperationKey, StateKey,
};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput};
pub use mfm_states_btc::{
    AssembleBtcNetworkCollectionReceiptConfig, AssembleBtcNetworkCollectionReceiptInput,
    AssembleBtcNetworkCollectionReceiptInputHandles, AssembleBtcNetworkCollectionReceiptState,
    BtcAddressBalanceSnapshotFact, BtcJointTip, BtcNetworkCollectionReceipt,
    ObserveBtcAddressBalanceConfig, ObserveBtcAddressBalanceInput,
    ObserveBtcAddressBalanceInputHandles, ObserveBtcAddressBalanceState,
    RecordBtcAddressBalanceFactConfig, RecordBtcAddressBalanceFactInput,
    RecordBtcAddressBalanceFactInputHandles, RecordBtcAddressBalanceFactState,
    ResolveBtcJointTipConfig, ResolveBtcJointTipInput, ResolveBtcJointTipInputHandles,
    ResolveBtcJointTipState, BTC_JOINT_TIP_SOURCE_READS, BTC_NATIVE_BALANCE_COVERAGE,
    BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS, BTC_NATIVE_BALANCE_SOURCE_STATUS,
};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.bitcoin";

// --- Internal anchored native-balance collection --------------------------------

const BTC_NETWORK_COLLECTION_OP_KIND_NAME: &str = "btc_network_collection";
const BTC_NETWORK_COLLECTION_OP_VERSION: &str = "mfm.bitcoin.operation.btc_network_collection.v1";
const BTC_NATIVE_AT_ANCHOR_OP_KIND_NAME: &str = "btc_native_balances_at_anchor";
const BTC_NATIVE_AT_ANCHOR_OP_VERSION: &str =
    "mfm.bitcoin.operation.btc_native_balances_at_anchor.v1";

/// Deterministic native Bitcoin source demand for one semantic network collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.bitcoin.operation.config.native_balances_at_anchor",
    validate = "validate_btc_native_balances_at_anchor_config"
)]
pub struct BtcNativeBalancesAtAnchorConfig {
    /// Semantic Bitcoin network id.
    pub network: String,
    /// Expected Bitcoin Core network tag.
    pub bitcoin_network: String,
    /// Non-secret semantic source identity.
    pub semantic_source_identity: String,
    /// Strictly sorted, unique public Bitcoin addresses to collect.
    pub addresses: Vec<String>,
}

impl BtcNativeBalancesAtAnchorConfig {
    /// Builds the exact state-owned joint-tip policy for this network.
    pub fn joint_tip_config(&self) -> ResolveBtcJointTipConfig {
        ResolveBtcJointTipConfig {
            network: self.network.clone(),
            bitcoin_network: self.bitcoin_network.clone(),
            semantic_source_identity: self.semantic_source_identity.clone(),
            max_source_reads: NonZeroU64::new(BTC_JOINT_TIP_SOURCE_READS)
                .expect("non-zero Bitcoin joint-tip policy"),
        }
    }

    /// Builds the exact state-owned observation policy for one demanded address.
    pub fn observe_config_for_address(&self, address: &str) -> ObserveBtcAddressBalanceConfig {
        ObserveBtcAddressBalanceConfig {
            network: self.network.clone(),
            bitcoin_network: self.bitcoin_network.clone(),
            semantic_source_identity: self.semantic_source_identity.clone(),
            address: address.to_owned(),
            coverage: BTC_NATIVE_BALANCE_COVERAGE.to_owned(),
            max_source_reads: NonZeroU64::new(BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS)
                .expect("non-zero Bitcoin native-balance policy"),
        }
    }
}

/// Validates deterministic native Bitcoin source demand.
pub fn validate_btc_native_balances_at_anchor_config(
    config: &BtcNativeBalancesAtAnchorConfig,
) -> Result<(), String> {
    if config.addresses.is_empty() {
        return Err("addresses must contain at least one public Bitcoin address".to_owned());
    }
    let mut previous: Option<&str> = None;
    for address in &config.addresses {
        if previous.is_some_and(|prior| prior >= address.as_str()) {
            return Err("addresses must be strictly sorted and unique".to_owned());
        }
        mfm_states_btc::validate_observe_btc_address_balance_config(
            &config.observe_config_for_address(address),
        )?;
        previous = Some(address);
    }
    mfm_states_btc::validate_resolve_btc_joint_tip_config(&config.joint_tip_config())
}

/// Certified demand for one internally coordinated Bitcoin network collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.bitcoin.operation.config.network_collection",
    validate = "validate_btc_network_collection_config"
)]
pub struct BtcNetworkCollectionConfig {
    /// All native source demand for this Bitcoin network.
    pub native_balances: BtcNativeBalancesAtAnchorConfig,
}

/// Validates the deterministic one-resource Bitcoin network collection shape.
pub fn validate_btc_network_collection_config(
    config: &BtcNetworkCollectionConfig,
) -> Result<(), String> {
    validate_btc_native_balances_at_anchor_config(&config.native_balances)
}

/// Output of an at-anchor Bitcoin native resource child.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.bitcoin.operation_outputs.native_balances_at_anchor")]
pub struct BtcNativeBalancesAtAnchorOutputs<'program, 'scope> {
    /// Exact source-near receipt after every managed fact record completed.
    pub receipt: Handle<'program, 'scope, BtcNetworkCollectionReceipt>,
}

/// Output of one internally coordinated Bitcoin network collection.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.bitcoin.operation_outputs.network_collection")]
pub struct BtcNetworkCollectionOutputs<'program, 'scope> {
    /// Joint tip resolved exactly once for this network.
    pub joint_tip: Handle<'program, 'scope, BtcJointTip>,
    /// Exact source-near network receipt.
    pub receipt: Handle<'program, 'scope, BtcNetworkCollectionReceipt>,
}

/// Reusable at-anchor Bitcoin native balance resource operation.
pub struct BtcNativeBalancesAtAnchorOperation;

impl Operation for BtcNativeBalancesAtAnchorOperation {
    type Config = BtcNativeBalancesAtAnchorConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, BtcJointTip>;
    type Output<'program, 'scope> = BtcNativeBalancesAtAnchorOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            BTC_NATIVE_AT_ANCHOR_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(
                b"mfm.bitcoin.operation:btc_native_balances_at_anchor",
            ),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(BTC_NATIVE_AT_ANCHOR_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.btc_native_balances_at_anchor"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        joint_tip: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let mut fact_handles = Vec::with_capacity(config.addresses.len());
        for (index, address) in config.addresses.iter().enumerate() {
            let observation = builder.state::<ObserveBtcAddressBalanceState, _>(
                StateKey::new(format!("observe_address_balance_{index}"))?,
                NoContext,
                config.observe_config_for_address(address),
                ObserveBtcAddressBalanceInputHandles {
                    joint_tip: joint_tip.clone(),
                },
            )?;
            fact_handles.push(builder.state::<RecordBtcAddressBalanceFactState, _>(
                StateKey::new(format!("record_address_balance_{index}"))?,
                NoContext,
                RecordBtcAddressBalanceFactConfig {},
                RecordBtcAddressBalanceFactInputHandles { observation },
            )?);
        }
        let balance_facts = NonEmptyHandles::try_from_vec(fact_handles)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let receipt = builder.state::<AssembleBtcNetworkCollectionReceiptState, _>(
            StateKey::new("assemble_network_collection_receipt")?,
            NoContext,
            AssembleBtcNetworkCollectionReceiptConfig {},
            AssembleBtcNetworkCollectionReceiptInputHandles {
                joint_tip,
                balance_facts,
            },
        )?;
        Ok(BtcNativeBalancesAtAnchorOutputs { receipt })
    }
}

/// Network coordinator that resolves one Bitcoin tip then invokes its anchored resource child.
pub struct BtcNetworkCollectionOperation;

impl Operation for BtcNetworkCollectionOperation {
    type Config = BtcNetworkCollectionConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = BtcNetworkCollectionOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            BTC_NETWORK_COLLECTION_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.bitcoin.operation:btc_network_collection"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(BTC_NETWORK_COLLECTION_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.btc_network_collection"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let joint_tip = builder.state::<ResolveBtcJointTipState, _>(
            StateKey::new("resolve_joint_tip")?,
            NoContext,
            config.native_balances.joint_tip_config(),
            ResolveBtcJointTipInputHandles {},
        )?;
        let resource = builder.call::<BtcNativeBalancesAtAnchorOperation, _>(
            OperationKey::new("native_balances_at_anchor")?,
            BtcNativeBalancesAtAnchorOperation,
            config.native_balances,
            joint_tip.clone(),
        )?;
        Ok(BtcNetworkCollectionOutputs {
            joint_tip,
            receipt: resource.receipt,
        })
    }
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub btc_collectors_state_registry,
    operation_registry: pub btc_collectors_operation_registry,
    certification: pub register_btc_collectors_certification_descriptors,
    includes: [],
    states: [
        ResolveBtcJointTipState,
        ObserveBtcAddressBalanceState,
        RecordBtcAddressBalanceFactState,
        AssembleBtcNetworkCollectionReceiptState,
    ],
    operations: [
        BtcNativeBalancesAtAnchorOperation,
        BtcNetworkCollectionOperation,
    ],
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
