#![warn(missing_docs)]
//! Deterministic EVM collector family operations.
//!
//! This crate owns internal EVM collector topology. Its callers supply the semantic
//! `NetworkConfig`; the operation derives every child read bound, coverage claim,
//! chain id, and native scale from state-owned policy and that network value.

use std::num::NonZeroU64;

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig};
use mfm_program::{NoContext, NonEmptyHandles, Operation, OperationExpansion, StateKey};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput};
pub use mfm_states_evm::{
    AssembleEvmNativeBalanceBatchConfig, AssembleEvmNativeBalanceBatchInput,
    AssembleEvmNativeBalanceBatchInputHandles, AssembleEvmNativeBalanceBatchState,
    EvmAddressNativeBalanceObservation, EvmAddressNativeBalanceSnapshotFact, EvmJointTip,
    EvmNativeBalanceBatchSummary, ObserveEvmNativeBalanceConfig, ObserveEvmNativeBalanceInput,
    ObserveEvmNativeBalanceInputHandles, ObserveEvmNativeBalanceState,
    RecordEvmNativeBalanceFactConfig, RecordEvmNativeBalanceFactInput,
    RecordEvmNativeBalanceFactInputHandles, RecordEvmNativeBalanceFactState,
    ResolveEvmJointTipConfig, ResolveEvmJointTipInput, ResolveEvmJointTipInputHandles,
    ResolveEvmJointTipState, EVM_JOINT_TIP_SOURCE_READS, EVM_NATIVE_BALANCE_COVERAGE,
    EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS,
};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.evm";
const BALANCE_OP_KIND_NAME: &str = "evm_native_balance";
const BALANCE_OP_VERSION: &str = "mfm.evm.operation.evm_native_balance.v1";

/// Planning config for a multi-account EVM native balance collector batch.
///
/// Multi-subject same-network batches **must** share one joint tip resolved once
/// in expand.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "evm-native-balance-config",
    schema = "mfm.evm.operation.config.evm_native_balance",
    validate = "validate_evm_native_balance_config"
)]
pub struct EvmNativeBalanceConfig {
    /// Exact semantic EVM network, including the only authored native scale.
    pub network: NetworkConfig,
    /// Account addresses to observe (at least one).
    pub accounts: Vec<String>,
}

impl EvmNativeBalanceConfig {
    /// Builds the joint-tip resolve config for this batch.
    pub fn joint_tip_config(&self) -> Result<ResolveEvmJointTipConfig, String> {
        let (network, chain_id) = self.evm_network_parts()?;
        let max_source_reads = NonZeroU64::new(EVM_JOINT_TIP_SOURCE_READS)
            .ok_or_else(|| "EVM joint-tip source-read policy must be non-zero".to_owned())?;
        Ok(ResolveEvmJointTipConfig {
            network: network.to_owned(),
            chain_id,
            max_source_reads,
        })
    }

    /// Builds an observe config for one account in this batch.
    pub fn observe_config_for_account(
        &self,
        account: &str,
    ) -> Result<ObserveEvmNativeBalanceConfig, String> {
        let _ = self.evm_network_parts()?;
        let max_source_reads = NonZeroU64::new(EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS)
            .ok_or_else(|| {
                "EVM native observation source-read policy must be non-zero".to_owned()
            })?;
        Ok(ObserveEvmNativeBalanceConfig {
            network: self.network.clone(),
            account: account.to_owned(),
            coverage: EVM_NATIVE_BALANCE_COVERAGE.to_owned(),
            max_source_reads,
        })
    }

    fn evm_network_parts(&self) -> Result<(&str, u64), String> {
        if self.network.family() != NetworkFamilyConfig::Evm {
            return Err("EVM native balance collector requires an EVM network".to_owned());
        }
        let chain_id = self
            .network
            .chain_id_u64()
            .ok_or_else(|| "EVM network is missing chain_id".to_owned())?;
        if self.network.native_decimals().is_none() {
            return Err("EVM network is missing native_decimals".to_owned());
        }
        Ok((self.network.network_id().as_str(), chain_id))
    }
}

/// Validates multi-account native balance collector planning config.
pub fn validate_evm_native_balance_config(config: &EvmNativeBalanceConfig) -> Result<(), String> {
    if config.accounts.is_empty() {
        return Err("accounts must contain at least one address".to_owned());
    }
    let mut seen = std::collections::BTreeSet::new();
    for account in &config.accounts {
        mfm_states_evm::validate_observe_evm_native_balance_config(
            &config.observe_config_for_account(account)?,
        )?;
        if !seen.insert(account.as_str()) {
            return Err(format!("duplicate account in batch: {account}"));
        }
    }
    mfm_states_evm::validate_resolve_evm_joint_tip_config(&config.joint_tip_config()?)?;
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
        // Resolve the joint tip once for the entire same-network batch.
        let joint_tip = builder.state::<ResolveEvmJointTipState, _>(
            StateKey::new("resolve_joint_tip")?,
            NoContext,
            config
                .joint_tip_config()
                .map_err(mfm_program::PlanError::Key)?,
            ResolveEvmJointTipInputHandles {},
        )?;

        let mut fact_handles = Vec::with_capacity(config.accounts.len());
        for (index, account) in config.accounts.iter().enumerate() {
            let observe = builder.state::<ObserveEvmNativeBalanceState, _>(
                StateKey::new(format!("observe_native_balance_{index}"))?,
                NoContext,
                config
                    .observe_config_for_account(account)
                    .map_err(mfm_program::PlanError::Key)?,
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
#[path = "evm_collectors_tests.rs"]
mod tests;
