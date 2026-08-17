use mfm_evm::EvmPhysicalTarget;
use mfm_evm::{EvmBalanceCollectionCompletion, EvmBalanceContext, EvmBalanceFailure};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_portfolio::{
    plan_snapshot, ConsolidatePortfolio, EnterPortfolioCollection, InitializePortfolio,
    MapEvmBalanceFailure, PortfolioConfig, PortfolioContinuation, PortfolioError,
    PortfolioSnapshotFailure, PortfolioSnapshotInput, PortfolioSnapshotOutput,
    PortfolioSnapshotSelector, ResumePortfolioCollection,
};
use mfm_program::{Declaration, PureState, State};

#[test]
fn semantic_portfolio_states_preserve_exact_contracts() {
    fn assert_state<S, I, O>()
    where
        S: State<Input = I, Output = O, Failure = PortfolioSnapshotFailure> + PureState,
        I: mfm_values::MfmValue,
        O: mfm_values::MfmValue,
    {
    }

    assert_state::<InitializePortfolio, PortfolioSnapshotInput, PortfolioContinuation>();
    assert_state::<
        EnterPortfolioCollection,
        PortfolioContinuation,
        EvmBalanceContext<PortfolioContinuation>,
    >();
    assert_state::<
        ResumePortfolioCollection,
        EvmBalanceCollectionCompletion<PortfolioContinuation>,
        PortfolioContinuation,
    >();
    assert_state::<MapEvmBalanceFailure, EvmBalanceFailure, PortfolioSnapshotOutput>();
    assert_state::<ConsolidatePortfolio, PortfolioContinuation, PortfolioSnapshotOutput>();

    assert_eq!(
        [
            InitializePortfolio::state_id().expect("initialize"),
            EnterPortfolioCollection::state_id().expect("enter"),
            ResumePortfolioCollection::state_id().expect("resume"),
            MapEvmBalanceFailure::state_id().expect("failure mapper"),
            ConsolidatePortfolio::state_id().expect("consolidate"),
        ]
        .map(|id| id.as_str().to_owned()),
        [
            "mfm.portfolio.state.initialize@1",
            "mfm.portfolio.state.enter-collection@1",
            "mfm.portfolio.state.resume-collection@1",
            "mfm.portfolio.state.map-evm-failure@1",
            "mfm.portfolio.state.consolidate@1",
        ]
    );
}

fn endpoint(byte: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.endpoint",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        )
        .expect("schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            DigestBytes::from_array([byte; 32]),
        ),
    )
    .expect("endpoint")
}

fn target(chain_id: u64, byte: u8) -> EvmPhysicalTarget {
    EvmPhysicalTarget::new(chain_id, endpoint(byte)).expect("target")
}

fn selector() -> PortfolioSnapshotSelector {
    serde_json::from_value(serde_json::json!({
        "target": "portfolio-example",
        "quote": "usd"
    }))
    .expect("selector")
}

fn config_with_sources(count: usize) -> serde_json::Value {
    let sources = (0..count)
        .map(|index| {
            serde_json::json!({
                "source_id": format!("wallet.{index}"),
                "chain_id": 1,
                "address": format!("0x{index:040x}"),
                "token": null
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [{
            "correlation": "native-collection",
            "request": { "sources": sources, "decimals": 18 }
        }]
    })
}

fn two_collection_config() -> serde_json::Value {
    serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [
            {
                "correlation": "chain-one",
                "request": {
                    "sources": [{
                        "source_id": "wallet.one",
                        "chain_id": 1,
                        "address": "0x0000000000000000000000000000000000000001",
                        "token": null
                    }],
                    "decimals": 18
                }
            },
            {
                "correlation": "chain-two",
                "request": {
                    "sources": [{
                        "source_id": "wallet.two",
                        "chain_id": 2,
                        "address": "0x0000000000000000000000000000000000000002",
                        "token": null
                    }],
                    "decimals": 18
                }
            }
        ]
    })
}

#[test]
fn planner_binds_every_read_to_the_exact_target() {
    let config: PortfolioConfig = serde_json::from_value(config_with_sources(2)).expect("config");
    let target = target(1, 2);
    let expected_binding = target.binding_ref().expect("binding");
    let (program, input) =
        plan_snapshot(selector(), &config, std::slice::from_ref(&target)).expect("plan");
    let bindings = program
        .declarations()
        .iter()
        .filter_map(|declaration| match declaration {
            Declaration::State(state) => state.execution().binding_ref(),
            Declaration::Match(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(bindings.len(), 12);
    assert!(bindings.iter().all(|binding| *binding == &expected_binding));
    let input = serde_json::to_value(input).expect("input");
    assert_eq!(
        input["collections"][0]["route_ref"],
        serde_json::to_value(&expected_binding).expect("binding json")
    );
}

#[test]
fn repeated_owned_children_keep_distinct_bindings_and_exact_parent_topology() {
    let config: PortfolioConfig =
        serde_json::from_value(two_collection_config()).expect("two collections");
    let targets = [target(1, 2), target(2, 3)];
    let first_binding = targets[0].binding_ref().expect("first binding");
    let second_binding = targets[1].binding_ref().expect("second binding");
    let (program, _) = plan_snapshot(selector(), &config, &targets).expect("two-child plan");
    assert_eq!(program.declarations().len(), 26);

    let bindings = program
        .declarations()
        .iter()
        .filter_map(|declaration| match declaration {
            Declaration::State(state) => state.execution().binding_ref(),
            Declaration::Match(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(bindings.len(), 12);
    assert!(bindings[..6]
        .iter()
        .all(|binding| *binding == &first_binding));
    assert!(bindings[6..]
        .iter()
        .all(|binding| *binding == &second_binding));

    let state = |index: usize| {
        let Declaration::State(state) = &program.declarations()[index] else {
            panic!("declaration {index} must be a State");
        };
        state
    };
    assert_eq!(state(10).next_index(), Some(11));
    assert_eq!(state(11).next_index(), Some(13));
    assert_eq!(state(12).next_index(), None);
    assert_eq!(state(22).next_index(), Some(23));
    assert_eq!(state(23).next_index(), Some(25));
    assert_eq!(state(24).next_index(), None);
    for index in [2, 3, 4, 6, 7, 8, 9, 10] {
        assert_eq!(state(index).failure_next_index(), Some(12));
    }
    for index in [14, 15, 16, 18, 19, 20, 21, 22] {
        assert_eq!(state(index).failure_next_index(), Some(24));
    }
}

#[test]
fn planner_rejects_missing_unsorted_and_duplicate_targets() {
    let config: PortfolioConfig = serde_json::from_value(config_with_sources(1)).expect("config");
    assert!(matches!(
        plan_snapshot(selector(), &config, &[]),
        Err(PortfolioError::Program)
    ));
    assert!(matches!(
        plan_snapshot(selector(), &config, &[target(2, 2), target(1, 3)]),
        Err(PortfolioError::Program)
    ));
    assert!(matches!(
        plan_snapshot(selector(), &config, &[target(1, 2), target(1, 3)]),
        Err(PortfolioError::Program)
    ));
}

#[test]
fn endpoint_change_changes_program_and_admitted_context() {
    let config: PortfolioConfig = serde_json::from_value(config_with_sources(1)).expect("config");
    let (first_program, first_input) =
        plan_snapshot(selector(), &config, &[target(1, 2)]).expect("first");
    let (second_program, second_input) =
        plan_snapshot(selector(), &config, &[target(1, 3)]).expect("second");
    assert_ne!(first_program.content_ref(), second_program.content_ref());
    assert_ne!(
        serde_json::to_value(first_input).expect("first input"),
        serde_json::to_value(second_input).expect("second input")
    );
}

#[test]
fn portfolio_program_and_c0_capacity_contract() {
    let maximum: PortfolioConfig =
        serde_json::from_value(config_with_sources(64)).expect("maximum config");
    let (program, _) = plan_snapshot(selector(), &maximum, &[target(1, 2)]).expect("maximum plan");
    assert_eq!(program.declarations().len(), 518);
    assert!(serde_json::from_value::<PortfolioConfig>(config_with_sources(65)).is_err());
}

#[test]
fn representative_program_identity_is_stable() {
    let config: PortfolioConfig =
        serde_json::from_value(config_with_sources(2)).expect("representative config");
    let (program, _) =
        plan_snapshot(selector(), &config, &[target(1, 2)]).expect("representative plan");
    assert_eq!(program.canonical_bytes().len(), 36_560);
    assert_eq!(
        serde_json::to_string(program.content_ref()).expect("content ref JSON"),
        r#"{"content_digest":"content:sha256-v1:59c503d1d8faa1a7afc33d9bb0bcad42d053e7cecb8571023bbc0cef42ab4beb","schema_id":"schema:mfm-program-document:2:sha256-jcs-v1:fc3c33ba3470e25a166df52597c4b45a723dd6d0823d1318938c29225323076c"}"#
    );
}
