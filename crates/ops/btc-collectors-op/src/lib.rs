#![warn(missing_docs)]
//! Deterministic Bitcoin collector family operations.
//!
//! This monocrate owns BTC collector topologies only:
//! - one-cycle chain-head collector (control checkpoints; not report pin authority)
//! - internal anchored native-balance collection (one network tip → per-source facts → receipt)
//!
//! It performs no live IO and does not register app assembly, adapters, transports, storage,
//! binaries, or recurring scheduler policy.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_btc_collectors::{
//!     btc_chain_head_collector_cycle_program_draft, BtcChainHeadCollectorConfig,
//! };
//!
//! let draft = btc_chain_head_collector_cycle_program_draft(
//!     BtcChainHeadCollectorConfig::default(),
//! )
//! .unwrap();
//! assert_eq!(draft.state_nodes().len(), 4);
//! ```

use std::num::NonZeroU64;

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, Handle, NoContext, NonEmptyHandles, Operation,
    OperationExpansion, OperationKey, PublicOutputKey, RootBuilder, ScopeKey, SeedKey, StateKey,
};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs};
pub use mfm_states_btc::{
    AssembleBtcNetworkCollectionReceiptConfig, AssembleBtcNetworkCollectionReceiptInput,
    AssembleBtcNetworkCollectionReceiptInputHandles, AssembleBtcNetworkCollectionReceiptState,
    BtcAddressBalanceSnapshotFact, BtcChainHeadFact, BtcChainHeadObservation,
    BtcChainHeadObservationContext, BtcJointTip, BtcNetworkCollectionReceipt,
    CollectorCheckpointFact, LoadedCollectorCheckpoint, ObserveBtcAddressBalanceConfig,
    ObserveBtcAddressBalanceInput, ObserveBtcAddressBalanceInputHandles,
    ObserveBtcAddressBalanceState, ObserveBtcChainHeadConfig, ObserveBtcChainHeadInput,
    ObserveBtcChainHeadInputHandles, ObserveBtcChainHeadState, QueryCollectorCheckpointConfig,
    QueryCollectorCheckpointInput, QueryCollectorCheckpointInputHandles,
    QueryCollectorCheckpointState, RecordBtcAddressBalanceFactConfig,
    RecordBtcAddressBalanceFactInput, RecordBtcAddressBalanceFactInputHandles,
    RecordBtcAddressBalanceFactState, RecordBtcChainHeadFactConfig, RecordBtcChainHeadFactInput,
    RecordBtcChainHeadFactInputHandles, RecordBtcChainHeadFactState,
    RecordCollectorCheckpointConfig, RecordCollectorCheckpointInput,
    RecordCollectorCheckpointInputHandles, RecordCollectorCheckpointState,
    ResolveBtcJointTipConfig, ResolveBtcJointTipInput, ResolveBtcJointTipInputHandles,
    ResolveBtcJointTipState, BTC_JOINT_TIP_SOURCE_READS, BTC_NATIVE_BALANCE_COVERAGE,
    BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS, BTC_NATIVE_BALANCE_SOURCE_STATUS,
};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.bitcoin";

// --- Chain-head collector cycle (not report pin authority) --------------------

const CHAIN_HEAD_OP_KIND_NAME: &str = "btc_chain_head_collector_cycle";
const CHAIN_HEAD_OP_VERSION: &str = "mfm.bitcoin.operation.btc_chain_head_collector_cycle.v1";
const CHAIN_HEAD_ROOT_SCOPE: &str = "btc_chain_head_collector";
const CHAIN_HEAD_OP_KEY: &str = "btc_chain_head_collector_cycle";
const CHAIN_HEAD_PUBLIC_OUTPUT_KEY: &str = "collector_checkpoint";
const CHAIN_HEAD_CONTEXT_SEED_KEY: &str = "observation_context";

/// Planning config for one bounded Bitcoin chain-head collector cycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.operation.config.btc_chain_head_collector_cycle",
    validate = "validate_btc_chain_head_collector_config"
)]
pub struct BtcChainHeadCollectorConfig {
    /// Collector kind whose checkpoint should be loaded and advanced.
    pub collector_kind: String,
    /// Non-secret semantic source identity to observe.
    pub semantic_source_identity: String,
    /// Semantic checkpoint partition.
    pub partition: String,
    /// Semantic Bitcoin network id.
    pub network: String,
    /// Expected Bitcoin Core network tag (`main`, `test`, `signet`, or `regtest`).
    pub bitcoin_network: String,
    /// Non-secret store scope for the internal checkpoint fact query.
    pub store_scope: String,
    /// Head kind requested by the observation state.
    pub head_kind: String,
    /// Optional confirmation depth for confirmed-head reads.
    pub confirmation_depth: Option<NonZeroU64>,
    /// Maximum number of source reads this bounded cycle may request.
    pub max_source_reads: NonZeroU64,
}

impl Default for BtcChainHeadCollectorConfig {
    fn default() -> Self {
        let checkpoint_query = QueryCollectorCheckpointConfig::default();
        Self {
            collector_kind: checkpoint_query.collector_kind,
            semantic_source_identity: checkpoint_query.semantic_source_identity,
            partition: checkpoint_query.partition,
            network: checkpoint_query.network,
            bitcoin_network: checkpoint_query.bitcoin_network,
            store_scope: checkpoint_query.store_scope,
            head_kind: "best".to_owned(),
            confirmation_depth: None,
            max_source_reads: NonZeroU64::new(1).expect("non-zero static value"),
        }
    }
}

impl BtcChainHeadCollectorConfig {
    /// Builds the checkpoint query state config for this cycle.
    pub fn checkpoint_query_config(&self) -> QueryCollectorCheckpointConfig {
        QueryCollectorCheckpointConfig {
            collector_kind: self.collector_kind.clone(),
            semantic_source_identity: self.semantic_source_identity.clone(),
            partition: self.partition.clone(),
            network: self.network.clone(),
            bitcoin_network: self.bitcoin_network.clone(),
            store_scope: self.store_scope.clone(),
        }
    }

    /// Builds the chain-head observation state config for this cycle.
    pub fn observe_chain_head_config(&self) -> ObserveBtcChainHeadConfig {
        ObserveBtcChainHeadConfig {
            network: self.network.clone(),
            bitcoin_network: self.bitcoin_network.clone(),
            semantic_source_identity: self.semantic_source_identity.clone(),
            head_kind: self.head_kind.clone(),
            confirmation_depth: self.confirmation_depth,
            max_source_reads: self.max_source_reads,
        }
    }

    /// Builds the checkpoint record state config for this cycle.
    pub fn record_checkpoint_config(&self) -> RecordCollectorCheckpointConfig {
        RecordCollectorCheckpointConfig {
            collector_kind: self.collector_kind.clone(),
            partition: self.partition.clone(),
        }
    }
}

/// Validates one-cycle collector planning config consistency.
pub fn validate_btc_chain_head_collector_config(
    config: &BtcChainHeadCollectorConfig,
) -> Result<(), String> {
    config
        .checkpoint_query_config()
        .request()
        .map_err(|error| error.to_string())?;
    config
        .observe_chain_head_config()
        .selection()
        .map_err(|error| error.to_string())?;
    mfm_states_btc::validate_record_collector_checkpoint_config(
        &config.record_checkpoint_config(),
    )?;
    Ok(())
}

/// Output handles produced by one Bitcoin chain-head collector cycle.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.bitcoin.operation_outputs.btc_chain_head_collector_cycle")]
pub struct BtcChainHeadCollectorCycleOutputs<'program, 'scope> {
    /// Recorded platform chain-head fact.
    pub chain_head_fact: mfm_program::Handle<'program, 'scope, BtcChainHeadFact>,
    /// Recorded internal collector checkpoint fact.
    pub collector_checkpoint: mfm_program::Handle<'program, 'scope, CollectorCheckpointFact>,
}

/// Root public outputs for the chain-head cycle.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.bitcoin.public_outputs.btc_chain_head_collector_cycle")]
pub struct BtcChainHeadCollectorCyclePublicOutputs<'program, 'scope> {
    /// Final checkpoint fact produced by the cycle.
    pub collector_checkpoint: mfm_program::Handle<'program, 'scope, CollectorCheckpointFact>,
}

/// Deterministic one-cycle Bitcoin chain-head collector operation.
pub struct BtcChainHeadCollectorCycleOperation;

impl Operation for BtcChainHeadCollectorCycleOperation {
    type Config = BtcChainHeadCollectorConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, BtcChainHeadObservationContext>;
    type Output<'program, 'scope> = BtcChainHeadCollectorCycleOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            CHAIN_HEAD_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(
                b"mfm.bitcoin.operation:btc_chain_head_collector_cycle",
            ),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(CHAIN_HEAD_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.btc_chain_head_collector_cycle"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        observation_context: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let loaded_checkpoint = builder.state::<QueryCollectorCheckpointState, _>(
            StateKey::new("query_collector_checkpoint")?,
            NoContext,
            config.checkpoint_query_config(),
            QueryCollectorCheckpointInputHandles {},
        )?;
        let observation = builder.state::<ObserveBtcChainHeadState, _>(
            StateKey::new("observe_btc_chain_head")?,
            NoContext,
            config.observe_chain_head_config(),
            ObserveBtcChainHeadInputHandles {
                loaded_checkpoint: loaded_checkpoint.clone(),
                context: observation_context,
            },
        )?;
        let chain_head_fact = builder.state::<RecordBtcChainHeadFactState, _>(
            StateKey::new("record_btc_chain_head_fact")?,
            NoContext,
            RecordBtcChainHeadFactConfig {},
            RecordBtcChainHeadFactInputHandles { observation },
        )?;
        let collector_checkpoint = builder.state::<RecordCollectorCheckpointState, _>(
            StateKey::new("record_collector_checkpoint")?,
            NoContext,
            config.record_checkpoint_config(),
            RecordCollectorCheckpointInputHandles {
                chain_head_fact: chain_head_fact.clone(),
                loaded_checkpoint,
            },
        )?;

        Ok(BtcChainHeadCollectorCycleOutputs {
            chain_head_fact,
            collector_checkpoint,
        })
    }
}

/// Builds a typed program draft for one Bitcoin chain-head collector cycle.
pub fn btc_chain_head_collector_cycle_program_draft(
    config: BtcChainHeadCollectorConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(CHAIN_HEAD_ROOT_SCOPE)?,
        btc_collectors_state_registry()?,
        btc_collectors_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let observation_context = root.seed(
                SeedKey::new(CHAIN_HEAD_CONTEXT_SEED_KEY)?,
                CanonicalSeed::from_value(&BtcChainHeadObservationContext {
                    observed_at_unix_ms: None,
                })?,
            )?;
            let result = root
                .scope()
                .call::<BtcChainHeadCollectorCycleOperation, _>(
                    OperationKey::new(CHAIN_HEAD_OP_KEY)?,
                    BtcChainHeadCollectorCycleOperation,
                    config,
                    observation_context,
                )?;
            root.bind_public_outputs(
                PublicOutputKey::new(CHAIN_HEAD_PUBLIC_OUTPUT_KEY)?,
                &BtcChainHeadCollectorCyclePublicOutputs {
                    collector_checkpoint: result.collector_checkpoint,
                },
            )
        },
    )
}

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
    states: [
        QueryCollectorCheckpointState,
        ObserveBtcChainHeadState,
        RecordBtcChainHeadFactState,
        RecordCollectorCheckpointState,
        ResolveBtcJointTipState,
        ObserveBtcAddressBalanceState,
        RecordBtcAddressBalanceFactState,
        AssembleBtcNetworkCollectionReceiptState,
    ],
    operations: [
        BtcChainHeadCollectorCycleOperation,
        BtcNativeBalancesAtAnchorOperation,
        BtcNetworkCollectionOperation,
    ],
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
