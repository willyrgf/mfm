#![warn(missing_docs)]
//! Deterministic Bitcoin chain-head collector cycle operation.
//!
//! This crate owns only the one-cycle collector topology. It performs no live IO and does not
//! register app assembly, adapters, transports, storage, binaries, or recurring scheduler policy.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_btc_chain_head_collector::{
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
    build_root_with_registries, CanonicalSeed, Handle, NoContext, Operation, OperationExpansion,
    OperationKey, PublicOutputKey, RootBuilder, ScopeKey, SeedKey, StateKey,
};
use mfm_program_derive::{MfmConfig, OperationOutput, PublicOutputs};
pub use mfm_states_btc::{
    BtcChainHeadFact, BtcChainHeadObservation, BtcChainHeadObservationContext,
    CollectorCheckpointFact, LoadedCollectorCheckpoint, ObserveBtcChainHeadConfig,
    ObserveBtcChainHeadInput, ObserveBtcChainHeadInputHandles, ObserveBtcChainHeadState,
    QueryCollectorCheckpointConfig, QueryCollectorCheckpointInput,
    QueryCollectorCheckpointInputHandles, QueryCollectorCheckpointState,
    RecordBtcChainHeadFactConfig, RecordBtcChainHeadFactInput, RecordBtcChainHeadFactInputHandles,
    RecordBtcChainHeadFactState, RecordCollectorCheckpointConfig, RecordCollectorCheckpointInput,
    RecordCollectorCheckpointInputHandles, RecordCollectorCheckpointState,
};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.bitcoin";
const OP_KIND_NAME: &str = "btc_chain_head_collector_cycle";
const OP_VERSION: &str = "mfm.bitcoin.operation.btc_chain_head_collector_cycle.v1";
const ROOT_SCOPE: &str = "btc_chain_head_collector";
const OP_KEY: &str = "btc_chain_head_collector_cycle";
const PUBLIC_OUTPUT_KEY: &str = "collector_checkpoint";
const OBSERVATION_CONTEXT_SEED_KEY: &str = "observation_context";

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
            store_scope: self.store_scope.clone(),
        }
    }

    /// Builds the chain-head observation state config for this cycle.
    pub fn observe_chain_head_config(&self) -> ObserveBtcChainHeadConfig {
        ObserveBtcChainHeadConfig {
            network: self.network.clone(),
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
        .request()
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

/// Root public outputs for the cycle.
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
            OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(
                b"mfm.bitcoin.operation:btc_chain_head_collector_cycle",
            ),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(OP_VERSION)
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

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub btc_chain_head_collector_state_registry,
    operation_registry: pub btc_chain_head_collector_operation_registry,
    certification: pub register_btc_chain_head_collector_certification_descriptors,
    states: [
        QueryCollectorCheckpointState,
        ObserveBtcChainHeadState,
        RecordBtcChainHeadFactState,
        RecordCollectorCheckpointState,
    ],
    operations: [BtcChainHeadCollectorCycleOperation],
}

/// Builds a typed program draft for one Bitcoin chain-head collector cycle.
pub fn btc_chain_head_collector_cycle_program_draft(
    config: BtcChainHeadCollectorConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        btc_chain_head_collector_state_registry()?,
        btc_chain_head_collector_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let observation_context = root.seed(
                SeedKey::new(OBSERVATION_CONTEXT_SEED_KEY)?,
                CanonicalSeed::from_value(&BtcChainHeadObservationContext {
                    observed_at_unix_ms: None,
                })?,
            )?;
            let result = root
                .scope()
                .call::<BtcChainHeadCollectorCycleOperation, _>(
                    OperationKey::new(OP_KEY)?,
                    BtcChainHeadCollectorCycleOperation,
                    config,
                    observation_context,
                )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &BtcChainHeadCollectorCyclePublicOutputs {
                    collector_checkpoint: result.collector_checkpoint,
                },
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_program::StateSpec;

    #[test]
    fn default_config_is_valid() {
        validate_btc_chain_head_collector_config(&BtcChainHeadCollectorConfig::default())
            .expect("default config");
    }

    #[test]
    fn config_reuses_state_validation() {
        let config = BtcChainHeadCollectorConfig {
            head_kind: "confirmed".to_owned(),
            ..BtcChainHeadCollectorConfig::default()
        };

        let error = validate_btc_chain_head_collector_config(&config).expect_err("invalid");
        assert!(error.contains("confirmed head observations require confirmation_depth"));
    }

    #[test]
    fn record_states_advertise_exact_fact_descriptor_allow_lists() {
        let chain_head = RecordBtcChainHeadFactState::emitted_fact_descriptors()
            .expect("chain-head descriptors");
        let checkpoint = RecordCollectorCheckpointState::emitted_fact_descriptors()
            .expect("checkpoint descriptors");

        assert!(QueryCollectorCheckpointState::emitted_fact_descriptors()
            .expect("query descriptors")
            .is_empty());
        assert!(ObserveBtcChainHeadState::emitted_fact_descriptors()
            .expect("observe descriptors")
            .is_empty());
        assert_eq!(chain_head.len(), 1);
        assert_eq!(checkpoint.len(), 1);
        assert_ne!(chain_head[0].descriptor_hash, checkpoint[0].descriptor_hash);
    }
}
