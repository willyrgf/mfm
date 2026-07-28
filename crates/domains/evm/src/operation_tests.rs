use super::*;
use crate::state::{EvmBalanceAsset, EvmBalanceSource};
use alloy_primitives::address;
use mfm_program::{
    build_root_with_registries, BridgeKey, BridgePolicy, OperationKey, PublicOutputKey,
    RootBuilder, ScopeKey, StateSpec,
};
use mfm_program_derive::PublicOutputs;

#[path = "operation_tests/recoverability_prototype.rs"]
mod recoverability_prototype;

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
fn balance_operation_has_the_exact_deterministic_one_state_topology() {
    assert_eq!(
        EvmBalanceCollectionOperation::version()
            .expect("operation version")
            .as_str(),
        "mfm.evm.operation.balance_collection.v1"
    );
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
    assert_eq!(first.state_nodes().len(), 1);
    assert_eq!(first.operation_lineage().len(), 1);
    assert_eq!(
        first.operation_lineage()[0].operation_kind,
        EvmBalanceCollectionOperation::kind().expect("operation kind")
    );
    assert_eq!(
        first.state_nodes()[0].state_kind,
        CollectEvmBalancesState::kind().expect("collect kind")
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
            let second = root
                .scope()
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

    assert_eq!(draft.state_nodes().len(), 2);
    assert_eq!(draft.operation_lineage().len(), 2);
    assert!(draft
        .operation_lineage()
        .iter()
        .all(|operation| operation.operation_name == EvmBalanceCollectionOperation::name()));
    mfm_certify::certify_program_draft(&draft).expect("multi-parent draft certifies");
}
