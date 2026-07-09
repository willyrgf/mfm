#![warn(missing_docs)]
//! Deterministic Bitcoin collector family operations.
//!
//! This monocrate owns BTC collector topologies only:
//! - one-cycle chain-head collector (control checkpoints; not report pin authority)
//! - multi-address native balance collector (joint tip once → pin-in observe → record)
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
use mfm_program_derive::{MfmConfig, OperationOutput, PublicOutputs};
pub use mfm_states_btc::{
    AssembleBtcAddressBalanceBatchConfig, AssembleBtcAddressBalanceBatchInput,
    AssembleBtcAddressBalanceBatchInputHandles, AssembleBtcAddressBalanceBatchState,
    BtcAddressBalanceBatchSummary, BtcAddressBalanceObservation,
    BtcAddressBalanceObservationContext, BtcAddressBalanceSnapshotFact, BtcChainHeadFact,
    BtcChainHeadObservation, BtcChainHeadObservationContext, BtcJointTip, CollectorCheckpointFact,
    LoadedCollectorCheckpoint, ObserveBtcAddressBalanceConfig, ObserveBtcAddressBalanceInput,
    ObserveBtcAddressBalanceInputHandles, ObserveBtcAddressBalanceState, ObserveBtcChainHeadConfig,
    ObserveBtcChainHeadInput, ObserveBtcChainHeadInputHandles, ObserveBtcChainHeadState,
    QueryCollectorCheckpointConfig, QueryCollectorCheckpointInput,
    QueryCollectorCheckpointInputHandles, QueryCollectorCheckpointState,
    RecordBtcAddressBalanceFactConfig, RecordBtcAddressBalanceFactInput,
    RecordBtcAddressBalanceFactInputHandles, RecordBtcAddressBalanceFactState,
    RecordBtcChainHeadFactConfig, RecordBtcChainHeadFactInput, RecordBtcChainHeadFactInputHandles,
    RecordBtcChainHeadFactState, RecordCollectorCheckpointConfig, RecordCollectorCheckpointInput,
    RecordCollectorCheckpointInputHandles, RecordCollectorCheckpointState, ResolveBtcJointTipConfig,
    ResolveBtcJointTipInput, ResolveBtcJointTipInputHandles, ResolveBtcJointTipState,
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

// --- Address-balance collector (joint tip + multi-address pin-in reads) ------

const BALANCE_OP_KIND_NAME: &str = "btc_address_balance_collector";
const BALANCE_OP_VERSION: &str = "mfm.bitcoin.operation.btc_address_balance_collector.v1";
const BALANCE_ROOT_SCOPE: &str = "btc_address_balance_collector";
const BALANCE_OP_KEY: &str = "btc_address_balance_collector";
const BALANCE_PUBLIC_OUTPUT_KEY: &str = "balance_batch";
const BALANCE_CONTEXT_SEED_KEY: &str = "balance_observation_context";

/// Planning config for a multi-address Bitcoin native balance collector batch.
///
/// Multi-subject same-network batches **must** share one joint tip resolved once
/// in expand (F26).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.operation.config.btc_address_balance_collector",
    validate = "validate_btc_address_balance_collector_config"
)]
pub struct BtcAddressBalanceCollectorConfig {
    /// Semantic Bitcoin network id.
    pub network: String,
    /// Expected Bitcoin Core network tag.
    pub bitcoin_network: String,
    /// Non-secret semantic source identity.
    pub semantic_source_identity: String,
    /// Public Bitcoin addresses to observe (at least one).
    pub addresses: Vec<String>,
    /// Head kind for the shared joint tip (`best` or `confirmed`).
    pub head_kind: String,
    /// Optional confirmation depth for confirmed-head joint tips.
    pub confirmation_depth: Option<NonZeroU64>,
    /// Coverage claim written on success (default `configured_only`).
    pub coverage: String,
    /// Maximum source reads for joint-tip resolution.
    pub max_source_reads: NonZeroU64,
}

impl Default for BtcAddressBalanceCollectorConfig {
    fn default() -> Self {
        Self {
            network: "bitcoin-mainnet".to_owned(),
            bitcoin_network: "main".to_owned(),
            semantic_source_identity: "public-bitcoin-core".to_owned(),
            addresses: vec!["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned()],
            head_kind: "best".to_owned(),
            confirmation_depth: None,
            coverage: "configured_only".to_owned(),
            max_source_reads: NonZeroU64::new(1).expect("non-zero static value"),
        }
    }
}

impl BtcAddressBalanceCollectorConfig {
    /// Builds the joint-tip resolve config for this batch.
    pub fn joint_tip_config(&self) -> ResolveBtcJointTipConfig {
        ResolveBtcJointTipConfig {
            network: self.network.clone(),
            bitcoin_network: self.bitcoin_network.clone(),
            semantic_source_identity: self.semantic_source_identity.clone(),
            head_kind: self.head_kind.clone(),
            confirmation_depth: self.confirmation_depth,
            max_source_reads: self.max_source_reads,
        }
    }

    /// Builds an observe config for one address in this batch.
    pub fn observe_config_for_address(&self, address: &str) -> ObserveBtcAddressBalanceConfig {
        ObserveBtcAddressBalanceConfig {
            network: self.network.clone(),
            bitcoin_network: self.bitcoin_network.clone(),
            semantic_source_identity: self.semantic_source_identity.clone(),
            address: address.to_owned(),
            coverage: self.coverage.clone(),
            max_source_reads: NonZeroU64::new(1).expect("non-zero static value"),
        }
    }
}

/// Validates multi-address balance collector planning config.
pub fn validate_btc_address_balance_collector_config(
    config: &BtcAddressBalanceCollectorConfig,
) -> Result<(), String> {
    if config.addresses.is_empty() {
        return Err("addresses must contain at least one public Bitcoin address".to_owned());
    }
    let mut seen = std::collections::BTreeSet::new();
    for address in &config.addresses {
        if !seen.insert(address.as_str()) {
            return Err(format!("duplicate address in batch: {address}"));
        }
        mfm_states_btc::validate_observe_btc_address_balance_config(
            &config.observe_config_for_address(address),
        )?;
    }
    mfm_states_btc::validate_resolve_btc_joint_tip_config(&config.joint_tip_config())?;
    Ok(())
}

/// Output handles produced by one Bitcoin address-balance collector batch.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.bitcoin.operation_outputs.btc_address_balance_collector")]
pub struct BtcAddressBalanceCollectorOutputs<'program, 'scope> {
    /// Joint tip shared by every subject in the batch.
    pub joint_tip: mfm_program::Handle<'program, 'scope, BtcJointTip>,
    /// Batch summary after shared-tip verification.
    pub batch_summary: mfm_program::Handle<'program, 'scope, BtcAddressBalanceBatchSummary>,
}

/// Root public outputs for the address-balance collector.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.bitcoin.public_outputs.btc_address_balance_collector")]
pub struct BtcAddressBalanceCollectorPublicOutputs<'program, 'scope> {
    /// Batch summary produced by the collector.
    pub batch_summary: mfm_program::Handle<'program, 'scope, BtcAddressBalanceBatchSummary>,
}

/// Deterministic multi-address Bitcoin native balance collector operation.
///
/// Expand resolves joint tip **once**, then observes and records each address
/// at that tip. Recorded facts are verified to share the tip before summary.
pub struct BtcAddressBalanceCollectorOperation;

impl Operation for BtcAddressBalanceCollectorOperation {
    type Config = BtcAddressBalanceCollectorConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, BtcAddressBalanceObservationContext>;
    type Output<'program, 'scope> = BtcAddressBalanceCollectorOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            BALANCE_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(
                b"mfm.bitcoin.operation:btc_address_balance_collector",
            ),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(BALANCE_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.btc_address_balance_collector"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        observation_context: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        // F26: resolve joint tip once for the entire same-network batch.
        let joint_tip = builder.state::<ResolveBtcJointTipState, _>(
            StateKey::new("resolve_joint_tip")?,
            NoContext,
            config.joint_tip_config(),
            ResolveBtcJointTipInputHandles {
                context: observation_context.clone(),
            },
        )?;

        let mut fact_handles = Vec::with_capacity(config.addresses.len());
        for (index, address) in config.addresses.iter().enumerate() {
            let observe = builder.state::<ObserveBtcAddressBalanceState, _>(
                StateKey::new(format!("observe_address_balance_{index}"))?,
                NoContext,
                config.observe_config_for_address(address),
                ObserveBtcAddressBalanceInputHandles {
                    joint_tip: joint_tip.clone(),
                    context: observation_context.clone(),
                },
            )?;
            let fact = builder.state::<RecordBtcAddressBalanceFactState, _>(
                StateKey::new(format!("record_address_balance_{index}"))?,
                NoContext,
                RecordBtcAddressBalanceFactConfig {},
                RecordBtcAddressBalanceFactInputHandles {
                    observation: observe,
                },
            )?;
            fact_handles.push(fact);
        }

        let balance_facts = NonEmptyHandles::try_from_vec(fact_handles)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let batch_summary = builder.state::<AssembleBtcAddressBalanceBatchState, _>(
            StateKey::new("assemble_address_balance_batch")?,
            NoContext,
            AssembleBtcAddressBalanceBatchConfig {},
            AssembleBtcAddressBalanceBatchInputHandles {
                joint_tip: joint_tip.clone(),
                balance_facts,
            },
        )?;

        Ok(BtcAddressBalanceCollectorOutputs {
            joint_tip,
            batch_summary,
        })
    }
}

/// Builds a typed program draft for a multi-address Bitcoin balance collector batch.
pub fn btc_address_balance_collector_program_draft(
    config: BtcAddressBalanceCollectorConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(BALANCE_ROOT_SCOPE)?,
        btc_collectors_state_registry()?,
        btc_collectors_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let observation_context = root.seed(
                SeedKey::new(BALANCE_CONTEXT_SEED_KEY)?,
                CanonicalSeed::from_value(&BtcAddressBalanceObservationContext {
                    observed_at_unix_ms: None,
                })?,
            )?;
            let result = root.scope().call::<BtcAddressBalanceCollectorOperation, _>(
                OperationKey::new(BALANCE_OP_KEY)?,
                BtcAddressBalanceCollectorOperation,
                config,
                observation_context,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(BALANCE_PUBLIC_OUTPUT_KEY)?,
                &BtcAddressBalanceCollectorPublicOutputs {
                    batch_summary: result.batch_summary,
                },
            )
        },
    )
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
        AssembleBtcAddressBalanceBatchState,
    ],
    operations: [
        BtcChainHeadCollectorCycleOperation,
        BtcAddressBalanceCollectorOperation,
    ],
}

/// Compatibility alias for chain-head certification registration.
pub fn register_btc_chain_head_collector_certification_descriptors(
    registry: &mut mfm_certify::CertificationRegistry,
) -> mfm_certify::Result<()> {
    register_btc_collectors_certification_descriptors(registry)
}

/// Compatibility alias for chain-head operation registry snapshot.
pub fn btc_chain_head_collector_operation_registry(
) -> mfm_program::Result<mfm_program::OperationRegistrySnapshot> {
    btc_collectors_operation_registry()
}

/// Compatibility alias for chain-head state registry snapshot.
pub fn btc_chain_head_collector_state_registry(
) -> mfm_program::Result<mfm_program::StateRegistrySnapshot> {
    btc_collectors_state_registry()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_program::StateSpec;

    #[test]
    fn default_chain_head_config_is_valid() {
        validate_btc_chain_head_collector_config(&BtcChainHeadCollectorConfig::default())
            .expect("default config");
    }

    #[test]
    fn default_balance_config_is_valid() {
        validate_btc_address_balance_collector_config(&BtcAddressBalanceCollectorConfig::default())
            .expect("default balance config");
    }

    #[test]
    fn multi_address_draft_shares_one_joint_tip_node() {
        let config = BtcAddressBalanceCollectorConfig {
            addresses: vec![
                "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned(),
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_owned(),
            ],
            ..BtcAddressBalanceCollectorConfig::default()
        };
        let draft = btc_address_balance_collector_program_draft(config).expect("draft");
        let nodes = draft.state_nodes();
        let joint_tip_nodes = nodes
            .iter()
            .filter(|node| node.key.as_str() == "resolve_joint_tip")
            .count();
        assert_eq!(joint_tip_nodes, 1, "must resolve joint tip exactly once");
        // tip + 2*(observe+record) + assemble = 1 + 4 + 1 = 6
        assert_eq!(nodes.len(), 6);
        let observe_count = nodes
            .iter()
            .filter(|node| node.key.as_str().starts_with("observe_address_balance_"))
            .count();
        assert_eq!(observe_count, 2);
        // Both observe nodes take the same joint tip cell as input.
        let tip_cell = nodes
            .iter()
            .find(|node| node.key.as_str() == "resolve_joint_tip")
            .expect("tip")
            .output_cell_id
            .as_str();
        for node in nodes
            .iter()
            .filter(|node| node.key.as_str().starts_with("observe_address_balance_"))
        {
            let rendered = format!("{:?}", node.input);
            assert!(
                rendered.contains(tip_cell),
                "observe must depend on shared joint tip cell: {rendered}"
            );
        }
    }

    #[test]
    fn chain_head_record_states_advertise_exact_fact_descriptor_allow_lists() {
        let chain_head = RecordBtcChainHeadFactState::emitted_fact_descriptors()
            .expect("chain-head descriptors");
        let checkpoint = RecordCollectorCheckpointState::emitted_fact_descriptors()
            .expect("checkpoint descriptors");
        let balance = RecordBtcAddressBalanceFactState::emitted_fact_descriptors()
            .expect("balance descriptors");

        assert_eq!(chain_head.len(), 1);
        assert_eq!(checkpoint.len(), 1);
        assert_eq!(balance.len(), 1);
        assert_ne!(chain_head[0].descriptor_hash, balance[0].descriptor_hash);
    }
}
