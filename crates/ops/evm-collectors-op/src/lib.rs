#![warn(missing_docs)]
//! Deterministic EVM collector family operations.
//!
//! This monocrate owns EVM collector topologies only (native balance at cutover).
//! It performs no live IO and does not register app assembly, adapters, transports,
//! storage, binaries, or recurring scheduler policy.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_evm_collectors::{
//!     evm_native_balance_program_draft, EvmNativeBalanceConfig,
//! };
//!
//! let draft = evm_native_balance_program_draft(
//!     EvmNativeBalanceConfig::default(),
//! )
//! .unwrap();
//! assert!(draft.state_nodes().len() >= 3);
//! ```

use std::num::NonZeroU64;

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{
    build_root_with_registries, NoContext, NonEmptyHandles, Operation, OperationExpansion,
    OperationKey, PublicOutputKey, RootBuilder, ScopeKey, StateKey,
};
use mfm_program_derive::{MfmConfig, OperationOutput, PublicOutputs};
pub use mfm_states_evm::{
    default_native_decimals, AssembleEvmNativeBalanceBatchConfig,
    AssembleEvmNativeBalanceBatchInput, AssembleEvmNativeBalanceBatchInputHandles,
    AssembleEvmNativeBalanceBatchState, EvmAddressNativeBalanceObservation,
    EvmAddressNativeBalanceSnapshotFact, EvmJointTip, EvmNativeBalanceBatchSummary,
    ObserveEvmNativeBalanceConfig, ObserveEvmNativeBalanceInput,
    ObserveEvmNativeBalanceInputHandles, ObserveEvmNativeBalanceState,
    RecordEvmNativeBalanceFactConfig, RecordEvmNativeBalanceFactInput,
    RecordEvmNativeBalanceFactInputHandles, RecordEvmNativeBalanceFactState,
    ResolveEvmJointTipConfig, ResolveEvmJointTipInput, ResolveEvmJointTipInputHandles,
    ResolveEvmJointTipState,
};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.evm";
const BALANCE_OP_KIND_NAME: &str = "evm_native_balance";
const BALANCE_OP_VERSION: &str = "mfm.evm.operation.evm_native_balance.v1";
const BALANCE_ROOT_SCOPE: &str = "evm_native_balance";
const BALANCE_OP_KEY: &str = "evm_native_balance";
const BALANCE_PUBLIC_OUTPUT_KEY: &str = "balance_batch";

/// Planning config for a multi-account EVM native balance collector batch.
///
/// Multi-subject same-network batches **must** share one joint tip resolved once
/// in expand (F26).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.evm.operation.config.evm_native_balance",
    validate = "validate_evm_native_balance_config"
)]
pub struct EvmNativeBalanceConfig {
    /// Semantic network id.
    pub network: String,
    /// Expected EVM chain id.
    pub chain_id: u64,
    /// Account addresses to observe (at least one).
    pub accounts: Vec<String>,
    /// Coverage claim written on success (default `configured_only`).
    pub coverage: String,
    /// Native token decimals (typically 18).
    pub decimals: u8,
    /// Maximum source reads for joint-tip resolution.
    pub max_source_reads: NonZeroU64,
}

impl Default for EvmNativeBalanceConfig {
    fn default() -> Self {
        Self {
            network: "ethereum-mainnet".to_owned(),
            chain_id: 1,
            accounts: vec!["0x0000000000000000000000000000000000000001".to_owned()],
            coverage: "configured_only".to_owned(),
            decimals: default_native_decimals(),
            max_source_reads: NonZeroU64::new(1).expect("non-zero static value"),
        }
    }
}

impl EvmNativeBalanceConfig {
    /// Builds the joint-tip resolve config for this batch.
    pub fn joint_tip_config(&self) -> ResolveEvmJointTipConfig {
        ResolveEvmJointTipConfig {
            network: self.network.clone(),
            chain_id: self.chain_id,
            max_source_reads: self.max_source_reads,
        }
    }

    /// Builds an observe config for one account in this batch.
    pub fn observe_config_for_account(&self, account: &str) -> ObserveEvmNativeBalanceConfig {
        ObserveEvmNativeBalanceConfig {
            network: self.network.clone(),
            chain_id: self.chain_id,
            account: account.to_owned(),
            coverage: self.coverage.clone(),
            decimals: self.decimals,
            max_source_reads: NonZeroU64::new(1).expect("non-zero static value"),
        }
    }
}

/// Validates multi-account native balance collector planning config.
pub fn validate_evm_native_balance_config(config: &EvmNativeBalanceConfig) -> Result<(), String> {
    if config.accounts.is_empty() {
        return Err("accounts must contain at least one address".to_owned());
    }
    let mut seen = std::collections::BTreeSet::new();
    for account in &config.accounts {
        if !seen.insert(account.as_str()) {
            return Err(format!("duplicate account in batch: {account}"));
        }
        mfm_states_evm::validate_observe_evm_native_balance_config(
            &config.observe_config_for_account(account),
        )?;
    }
    mfm_states_evm::validate_resolve_evm_joint_tip_config(&config.joint_tip_config())?;
    Ok(())
}

/// Output handles produced by one EVM native balance collector batch.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.operation_outputs.evm_native_balance")]
pub struct EvmNativeBalanceOutputs<'program, 'scope> {
    /// Joint tip shared by every subject in the batch.
    pub joint_tip: mfm_program::Handle<'program, 'scope, EvmJointTip>,
    /// Batch summary after shared-tip verification.
    pub batch_summary: mfm_program::Handle<'program, 'scope, EvmNativeBalanceBatchSummary>,
}

/// Root public outputs for the native balance collector.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.public_outputs.evm_native_balance")]
pub struct EvmNativeBalancePublicOutputs<'program, 'scope> {
    /// Batch summary produced by the collector.
    pub batch_summary: mfm_program::Handle<'program, 'scope, EvmNativeBalanceBatchSummary>,
}

/// Deterministic multi-account EVM native balance collector operation.
pub struct EvmNativeBalanceOperation;

impl Operation for EvmNativeBalanceOperation {
    type Config = EvmNativeBalanceConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = EvmNativeBalanceOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            BALANCE_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.evm.operation:evm_native_balance"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(BALANCE_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.evm_native_balance"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        // F26: resolve joint tip once for the entire same-network batch.
        let joint_tip = builder.state::<ResolveEvmJointTipState, _>(
            StateKey::new("resolve_joint_tip")?,
            NoContext,
            config.joint_tip_config(),
            ResolveEvmJointTipInputHandles {},
        )?;

        let mut fact_handles = Vec::with_capacity(config.accounts.len());
        for (index, account) in config.accounts.iter().enumerate() {
            let observe = builder.state::<ObserveEvmNativeBalanceState, _>(
                StateKey::new(format!("observe_native_balance_{index}"))?,
                NoContext,
                config.observe_config_for_account(account),
                ObserveEvmNativeBalanceInputHandles {
                    joint_tip: joint_tip.clone(),
                },
            )?;
            let fact = builder.state::<RecordEvmNativeBalanceFactState, _>(
                StateKey::new(format!("record_native_balance_{index}"))?,
                NoContext,
                RecordEvmNativeBalanceFactConfig {},
                RecordEvmNativeBalanceFactInputHandles {
                    observation: observe,
                },
            )?;
            fact_handles.push(fact);
        }

        let balance_facts = NonEmptyHandles::try_from_vec(fact_handles)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let batch_summary = builder.state::<AssembleEvmNativeBalanceBatchState, _>(
            StateKey::new("assemble_native_balance_batch")?,
            NoContext,
            AssembleEvmNativeBalanceBatchConfig {},
            AssembleEvmNativeBalanceBatchInputHandles {
                joint_tip: joint_tip.clone(),
                balance_facts,
            },
        )?;

        Ok(EvmNativeBalanceOutputs {
            joint_tip,
            batch_summary,
        })
    }
}

/// Builds a typed program draft for a multi-account EVM native balance collector batch.
pub fn evm_native_balance_program_draft(
    config: EvmNativeBalanceConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(BALANCE_ROOT_SCOPE)?,
        evm_collectors_state_registry()?,
        evm_collectors_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let result = root.scope().call::<EvmNativeBalanceOperation, _>(
                OperationKey::new(BALANCE_OP_KEY)?,
                EvmNativeBalanceOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(BALANCE_PUBLIC_OUTPUT_KEY)?,
                &EvmNativeBalancePublicOutputs {
                    batch_summary: result.batch_summary,
                },
            )
        },
    )
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub evm_collectors_state_registry,
    operation_registry: pub evm_collectors_operation_registry,
    certification: pub register_evm_collectors_certification_descriptors,
    states: [
        ResolveEvmJointTipState,
        ObserveEvmNativeBalanceState,
        RecordEvmNativeBalanceFactState,
        AssembleEvmNativeBalanceBatchState,
    ],
    operations: [EvmNativeBalanceOperation],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        validate_evm_native_balance_config(&EvmNativeBalanceConfig::default()).expect("default");
    }

    #[test]
    fn multi_account_draft_shares_one_joint_tip_node() {
        let config = EvmNativeBalanceConfig {
            accounts: vec![
                "0x0000000000000000000000000000000000000001".to_owned(),
                "0x0000000000000000000000000000000000000002".to_owned(),
            ],
            ..EvmNativeBalanceConfig::default()
        };
        let draft = evm_native_balance_program_draft(config).expect("draft");
        let nodes = draft.state_nodes();
        let joint_tip_nodes = nodes
            .iter()
            .filter(|node| node.key.as_str() == "resolve_joint_tip")
            .count();
        assert_eq!(joint_tip_nodes, 1);
        // tip + 2*(observe+record) + assemble = 6
        assert_eq!(nodes.len(), 6);
        let tip_cell = nodes
            .iter()
            .find(|node| node.key.as_str() == "resolve_joint_tip")
            .expect("tip")
            .output_cell_id
            .as_str();
        for node in nodes
            .iter()
            .filter(|node| node.key.as_str().starts_with("observe_native_balance_"))
        {
            let rendered = format!("{:?}", node.input);
            assert!(
                rendered.contains(tip_cell),
                "observe must depend on shared joint tip: {rendered}"
            );
        }
    }
}
