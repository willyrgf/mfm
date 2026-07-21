#![warn(missing_docs)]
//! Deterministic reusable EVM balance collection topology.
//!
//! [`EvmBalanceCollectionOperation`] always expands to one external read followed by one atomic
//! fact record and exports only the resulting receipt. Parent operations compose it directly.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_evm_collectors::EvmBalanceCollectionOperation;
//! use mfm_program::Operation as _;
//!
//! assert_eq!(EvmBalanceCollectionOperation::name(), "mfm.evm.balance_collection");
//! ```

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{NoContext, Operation, OperationExpansion, StateKey};
use mfm_program_derive::OperationOutput;
pub use mfm_states_evm::{
    CollectEvmBalancesState, EvmBalanceAsset, EvmBalanceCollectionConfig,
    EvmBalanceCollectionReceipt, EvmBalanceSource, RecordEvmBalanceFactsInputHandles,
    RecordEvmBalanceFactsState,
};

const OP_NAMESPACE: &str = "mfm.evm";
const OP_KIND_NAME: &str = "balance_collection";
const OP_VERSION: &str = "mfm.evm.operation.balance_collection.v1";

/// Output of one reusable EVM balance collection operation.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.operation_outputs.balance_collection")]
pub struct EvmBalanceCollectionOutputs<'program, 'scope> {
    /// Checked receipt returned by the atomic fact-recording state.
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

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub evm_collectors_state_registry,
    operation_registry: pub evm_collectors_operation_registry,
    certification: pub register_evm_collectors_certification_descriptors,
    includes: [],
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
    use mfm_program::{
        build_root_with_registries, BridgeKey, BridgePolicy, OperationKey, PublicOutputKey,
        RootBuilder, ScopeKey, StateSpec,
    };
    use mfm_program_derive::PublicOutputs;

    #[derive(PublicOutputs)]
    #[mfm(schema = "mfm.evm.test.balance_operation_outputs")]
    struct BalanceOperationOutputs<'program, 'scope> {
        receipt: mfm_program::Handle<'program, 'scope, EvmBalanceCollectionReceipt>,
    }

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
    fn balance_operation_has_the_exact_deterministic_two_state_topology() {
        let build = || {
            build_root_with_registries(
                ScopeKey::new("balance_operation_test")?,
                evm_collectors_state_registry()?,
                evm_collectors_operation_registry()?,
                |root: &mut RootBuilder<'_, '_>| {
                    let output = root.scope().call::<EvmBalanceCollectionOperation, _>(
                        OperationKey::new("collect")?,
                        EvmBalanceCollectionOperation,
                        config(),
                        (),
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("receipt")?,
                        &BalanceOperationOutputs {
                            receipt: output.receipt,
                        },
                    )
                },
            )
        };
        let first = build().expect("first draft");
        let second = build().expect("second draft");
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
        mfm_certify::certify_program_draft(&first).expect("operation draft certifies");
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
