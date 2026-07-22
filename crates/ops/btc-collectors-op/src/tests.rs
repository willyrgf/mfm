use super::*;
use mfm_program::{
    build_root_with_registries, BridgeKey, BridgePolicy, OperationKey, PublicOutputKey,
    RootBuilder, ScopeKey, StateSpec as _,
};
use mfm_program_derive::PublicOutputs;

const LEGACY_MAIN: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
const SEGWIT_MAIN: &str = "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw";

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.bitcoin.test.balance_operation_outputs")]
struct BalanceOperationOutputs<'program, 'scope> {
    receipt: mfm_program::Handle<'program, 'scope, BitcoinBalanceCollectionReceipt>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.bitcoin.test.multi_parent_outputs")]
struct MultiParentOutputs<'program, 'scope> {
    first: mfm_program::Handle<'program, 'scope, BitcoinBalanceCollectionReceipt>,
    second: mfm_program::Handle<'program, 'scope, BitcoinBalanceCollectionReceipt>,
}

fn config() -> BitcoinBalanceCollectionConfig {
    BitcoinBalanceCollectionConfig::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        vec![LEGACY_MAIN.to_owned(), SEGWIT_MAIN.to_owned()],
    )
    .expect("config")
}

#[test]
fn balance_operation_has_the_exact_deterministic_one_state_topology() {
    let build = || {
        build_root_with_registries(
            ScopeKey::new("balance_operation_test")?,
            bitcoin_collectors_state_registry()?,
            bitcoin_collectors_operation_registry()?,
            |root: &mut RootBuilder<'_, '_>| {
                let output = root.scope().call::<BitcoinBalanceCollectionOperation, _>(
                    OperationKey::new("collect")?,
                    BitcoinBalanceCollectionOperation,
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
        BitcoinBalanceCollectionOperation::kind().expect("operation kind")
    );
    assert_eq!(
        first.state_nodes()[0].state_kind,
        CollectBitcoinBalancesState::kind().expect("collect kind")
    );
    assert_eq!(first.public_output_spec().outputs().len(), 1);
    assert_eq!(
        first.public_output_spec().outputs()[0]
            .public_field_path()
            .as_str(),
        "receipt"
    );
    assert_eq!(
        first.state_nodes()[0].fact_descriptor_allowlist.len(),
        1,
        "the read state automatically declares its fact batch"
    );
    mfm_certify::certify_program_draft(&first).expect("operation draft certifies");
}

#[test]
fn multiple_parent_scopes_compose_the_same_operation_topology() {
    let draft = build_root_with_registries(
        ScopeKey::new("multi_parent").expect("root key"),
        bitcoin_collectors_state_registry().expect("state registry"),
        bitcoin_collectors_operation_registry().expect("operation registry"),
        |root: &mut RootBuilder<'_, '_>| {
            let first = root
                .scope()
                .child_scope(ScopeKey::new("first_parent")?, |child| {
                    let output = child.scope().call::<BitcoinBalanceCollectionOperation, _>(
                        OperationKey::new("collect")?,
                        BitcoinBalanceCollectionOperation,
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
                    let output = child.scope().call::<BitcoinBalanceCollectionOperation, _>(
                        OperationKey::new("collect")?,
                        BitcoinBalanceCollectionOperation,
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
    .expect("draft");

    assert_eq!(draft.operation_lineage().len(), 2);
    assert_eq!(draft.state_nodes().len(), 2);
    assert!(draft
        .state_nodes()
        .iter()
        .all(|node| node.state_descriptor_name == "mfm.bitcoin.collect_balances"));
    mfm_certify::certify_program_draft(&draft).expect("composed draft certifies");
}

#[test]
fn config_rejects_noncanonical_order_duplicates_and_empty_demand() {
    for addresses in [
        Vec::new(),
        vec![SEGWIT_MAIN.to_owned(), LEGACY_MAIN.to_owned()],
        vec![LEGACY_MAIN.to_owned(), LEGACY_MAIN.to_owned()],
    ] {
        assert!(BitcoinBalanceCollectionConfig::new(
            "bitcoin-mainnet",
            "main",
            "public-bitcoin-core",
            addresses,
        )
        .is_err());
    }
}
