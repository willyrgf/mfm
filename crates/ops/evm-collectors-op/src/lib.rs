#![warn(missing_docs)]
//! Deterministic reusable EVM balance collection topology.
//!
//! [`EvmBalanceCollectionOperation`] always expands to one external read followed by one atomic
//! fact record and exports only the resulting receipt. The cycle helpers wrap that same operation
//! for scheduler-owned internal runs; they are not application entry points.
//!
//! # Examples
//!
//! ```rust
//! use alloy_primitives::Address;
//! use mfm_op_evm_collectors::{
//!     evm_balance_collection_cycle_program_draft, EvmBalanceAsset,
//!     EvmBalanceCollectionConfig, EvmBalanceSource,
//! };
//!
//! let source = EvmBalanceSource::new(Address::ZERO, EvmBalanceAsset::Native)?;
//! let config = EvmBalanceCollectionConfig::new("ethereum-mainnet", 1, 18, vec![source])?;
//! let draft = evm_balance_collection_cycle_program_draft(config)?;
//! assert_eq!(draft.state_nodes().len(), 2);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{
    build_root_with_registries, NoContext, Operation, OperationExpansion, OperationKey,
    PublicOutputKey, RootBuilder, ScopeKey, StateKey, TypedProgramLaunchPlan,
};
use mfm_program_derive::{OperationOutput, PublicOutputs};
pub use mfm_states_evm::{
    CollectEvmBalancesState, EvmBalanceAsset, EvmBalanceCollectionConfig,
    EvmBalanceCollectionReceipt, EvmBalanceSource, RecordEvmBalanceFactsInputHandles,
    RecordEvmBalanceFactsState,
};

const OP_NAMESPACE: &str = "mfm.evm";
const OP_KIND_NAME: &str = "balance_collection";
const OP_VERSION: &str = "mfm.evm.operation.balance_collection.v1";
const CYCLE_ROOT_SCOPE: &str = "evm_balance_collection_cycle";
const CYCLE_OPERATION_KEY: &str = "evm_balance_collection";
const CYCLE_PUBLIC_OUTPUT_KEY: &str = "evm_balance_collection_receipt";

/// Output of one reusable EVM balance collection operation.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.operation_outputs.balance_collection")]
pub struct EvmBalanceCollectionOutputs<'program, 'scope> {
    /// Checked receipt returned by the atomic fact-recording state.
    pub receipt: mfm_program::Handle<'program, 'scope, EvmBalanceCollectionReceipt>,
}

/// Internal scheduler-cycle root outputs.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.internal_cycle_outputs.balance_collection")]
pub struct EvmBalanceCollectionCycleOutputs<'program, 'scope> {
    /// Checked collection receipt; the internal observation batch remains private.
    pub receipt: mfm_program::Handle<'program, 'scope, EvmBalanceCollectionReceipt>,
}

/// Reusable two-state EVM balance collection operation.
pub struct EvmBalanceCollectionOperation;

impl Operation for EvmBalanceCollectionOperation {
    type Config = EvmBalanceCollectionConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = EvmBalanceCollectionOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.evm.operation:balance_collection"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.balance_collection"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let batch = builder.state::<CollectEvmBalancesState, _>(
            StateKey::new("collect_balances")?,
            NoContext,
            config.clone(),
            (),
        )?;
        let receipt = builder.state::<RecordEvmBalanceFactsState, _>(
            StateKey::new("record_balance_facts")?,
            NoContext,
            config,
            RecordEvmBalanceFactsInputHandles { batch },
        )?;
        Ok(EvmBalanceCollectionOutputs { receipt })
    }
}

/// Builds one scheduler-owned internal EVM balance collection cycle draft.
pub fn evm_balance_collection_cycle_program_draft(
    config: EvmBalanceCollectionConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(CYCLE_ROOT_SCOPE)?,
        evm_collectors_state_registry()?,
        evm_collectors_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let output = root.scope().call::<EvmBalanceCollectionOperation, _>(
                OperationKey::new(CYCLE_OPERATION_KEY)?,
                EvmBalanceCollectionOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(CYCLE_PUBLIC_OUTPUT_KEY)?,
                &EvmBalanceCollectionCycleOutputs {
                    receipt: output.receipt,
                },
            )
        },
    )
}

/// Builds a launch plan for one scheduler-owned internal collection cycle.
pub fn evm_balance_collection_cycle_program_launch_plan(
    config: EvmBalanceCollectionConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    TypedProgramLaunchPlan::from_draft(evm_balance_collection_cycle_program_draft(config)?)
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub evm_collectors_state_registry,
    operation_registry: pub evm_collectors_operation_registry,
    certification: pub register_evm_collectors_certification_descriptors,
    states: [
        CollectEvmBalancesState,
        RecordEvmBalanceFactsState,
    ],
    operations: [
        EvmBalanceCollectionOperation,
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use mfm_program::{BridgeKey, BridgePolicy, StateSpec};

    #[derive(PublicOutputs)]
    #[mfm(schema = "mfm.evm.test.multi_parent_outputs")]
    struct MultiParentOutputs<'program, 'scope> {
        first: mfm_program::Handle<'program, 'scope, EvmBalanceCollectionReceipt>,
        second: mfm_program::Handle<'program, 'scope, EvmBalanceCollectionReceipt>,
    }

    fn config() -> EvmBalanceCollectionConfig {
        EvmBalanceCollectionConfig::new(
            "ethereum-mainnet",
            1,
            18,
            vec![EvmBalanceSource::new(
                address!("000000000000000000000000000000000000dead"),
                EvmBalanceAsset::Native,
            )
            .expect("source")],
        )
        .expect("config")
    }

    #[test]
    fn operation_and_cycle_share_the_exact_two_state_topology() {
        let first = evm_balance_collection_cycle_program_draft(config()).expect("first draft");
        let second = evm_balance_collection_cycle_program_draft(config()).expect("second draft");
        assert_eq!(first, second);
        assert_eq!(first.state_nodes().len(), 2);
        assert_eq!(first.operation_lineage().len(), 1);
        assert_eq!(
            first.operation_lineage()[0].operation_kind,
            EvmBalanceCollectionOperation::kind().expect("operation kind")
        );
        assert_eq!(
            first.state_nodes()[0].state_kind,
            CollectEvmBalancesState::kind().expect("collect kind")
        );
        assert_eq!(
            first.state_nodes()[1].state_kind,
            RecordEvmBalanceFactsState::kind().expect("record kind")
        );
        assert_eq!(first.public_output_spec().outputs().len(), 1);
        assert_eq!(
            first.public_output_spec().outputs()[0]
                .public_field_path()
                .as_str(),
            "receipt"
        );
        mfm_certify::certify_program_draft(&first).expect("cycle certifies");
        let launch =
            evm_balance_collection_cycle_program_launch_plan(config()).expect("cycle launch plan");
        assert_eq!(launch.draft, first);
    }

    #[test]
    fn multiple_parent_scopes_compose_the_same_operation_topology() {
        let draft = build_root_with_registries(
            ScopeKey::new("multi_parent").expect("root key"),
            evm_collectors_state_registry().expect("state registry"),
            evm_collectors_operation_registry().expect("operation registry"),
            |root: &mut RootBuilder<'_, '_>| {
                let first = root
                    .scope()
                    .child_scope(ScopeKey::new("first_parent")?, |child| {
                        let output = child.scope().call::<EvmBalanceCollectionOperation, _>(
                            OperationKey::new("collect")?,
                            EvmBalanceCollectionOperation,
                            config(),
                            (),
                        )?;
                        let receipt = child.export_to_parent(
                            BridgeKey::new("receipt")?,
                            output.receipt,
                            BridgePolicy::same_run_same_value(),
                        )?;
                        child.bridge_to_parent(receipt)
                    })?;
                let second =
                    root.scope()
                        .child_scope(ScopeKey::new("second_parent")?, |child| {
                            let output = child.scope().call::<EvmBalanceCollectionOperation, _>(
                                OperationKey::new("collect")?,
                                EvmBalanceCollectionOperation,
                                config(),
                                (),
                            )?;
                            let receipt = child.export_to_parent(
                                BridgeKey::new("receipt")?,
                                output.receipt,
                                BridgePolicy::same_run_same_value(),
                            )?;
                            child.bridge_to_parent(receipt)
                        })?;
                root.bind_public_outputs(
                    PublicOutputKey::new("receipts")?,
                    &MultiParentOutputs { first, second },
                )
            },
        )
        .expect("multi-parent draft");

        assert_eq!(draft.state_nodes().len(), 4);
        assert_eq!(draft.operation_lineage().len(), 2);
        assert!(draft.operation_lineage().iter().all(|operation| {
            operation.operation_name == EvmBalanceCollectionOperation::name()
        }));
        mfm_certify::certify_program_draft(&draft).expect("multi-parent draft certifies");
    }
}
