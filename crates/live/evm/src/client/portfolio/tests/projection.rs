use super::*;
use mfm_chain::balance::{
    BalanceContext, CandidateBalance, ConsolidateBalanceCollection, DecimalScale, ObserveBalance,
    ObserveBalanceFailure, PreparedBalance,
};
use mfm_chain::ObservationPoint;
use mfm_portfolio::{EnterPortfolioCollection, InitializePortfolio, PortfolioContinuation};
use mfm_program::{ProposedStateOutcome, PureState, State};
use mfm_values::Unsigned256;

fn project<S: State>(
    program: &mfm_program::Program,
    input: &S::Input,
    original: &S::Failure,
    expected: &str,
) {
    let identity = mfm_program::state_implementation_ref::<S>().unwrap();
    let declaration = program
        .declarations()
        .iter()
        .find(|state| state.state_implementation_ref() == &identity)
        .unwrap();
    let projected = snapshot_failure(
        declaration,
        &Object::from_value(input).unwrap(),
        &Object::from_value(original).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(projected).unwrap(),
        serde_json::json!({"kind":"collection_failed","value":{"ordinal":0,"code":expected}})
    );
}

// Exercise each retained input decoder and failure owner directly. Runtime persistence is covered
// by the maintained caller cases; no fault/continuation cross-product or provider replay is needed.
#[test]
fn every_retained_stage_projects_its_own_typed_input_and_original() {
    type K = PortfolioContinuation;
    let mut wire = serde_json::to_value(configuration(false)).unwrap();
    wire["input"]["portfolio"]["collections"][0]["request"]["decimals"] = 2.into();
    let config: EvmPortfolioConfig = serde_json::from_value(wire.clone()).unwrap();
    let input = admit_snapshot(&config, None).unwrap();
    let provider = Arc::new(Provider(AtomicUsize::new(0), None));
    let program = compile(
        config.entry_point_id().unwrap(),
        &mfm_portfolio::PortfolioSnapshotOperation::default(),
        &input,
        &resources("expected", provider.clone()),
        ProgramLimits::new(0),
    )
    .unwrap();
    // Projection checks exact type/context agreement, not this Program's admitted starting value.
    // Construct the checked completion directly: aggregate overflow needs no provider activity.
    let sources = wire["input"]["portfolio"]["collections"][0]["request"]["sources"]
        .as_array_mut()
        .unwrap();
    for ordinal in 2..9 {
        let mut token = sources[1].clone();
        token["source_id"] = format!("token-{ordinal}").into();
        sources.push(token);
    }
    let config: EvmPortfolioConfig = serde_json::from_value(wire).unwrap();
    let input = admit_snapshot(&config, None).unwrap();
    let ProposedStateOutcome::Success {
        output: continuation,
    } = InitializePortfolio::evaluate(input).unwrap()
    else {
        panic!("checked input initializes the collection")
    };
    let ProposedStateOutcome::Success {
        output: mut context,
    } = EnterPortfolioCollection::evaluate(continuation).unwrap()
    else {
        panic!("checked continuation enters the collection")
    };
    let anchor = EvmBlockAnchor {
        number: EvmU256::from_u64(9),
        hash: EvmHash::from_bytes([3; 32]),
    };
    let point = ObservationPoint::new(
        context.request().sources()[0].target().ledger().clone(),
        Object::from_value(&EvmBlockPoint::new(
            anchor.number.clone(),
            anchor.hash.clone(),
        ))
        .unwrap(),
    );
    let maximum = Unsigned256::new(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .unwrap();
    for ordinal in 0..9 {
        if ordinal >= 2 {
            let prepared =
                PreparedBalance::new(context, point.clone(), DecimalScale::new(0).unwrap())
                    .unwrap();
            context = CandidateBalance::new(prepared, maximum.clone())
                .append_confirmed()
                .unwrap()
                .unwrap();
            continue;
        }
        let token = ordinal != 0;
        for reason in [
            EvmObservationRejection::Rejected,
            EvmObservationRejection::SafeFailure,
            EvmObservationRejection::ChainMismatch,
        ] {
            project::<CheckEvmBalanceChain<K>>(
                &program,
                &context,
                &EvmBalanceFailure::ObservationRejected { reason },
                "chain_identity_unavailable",
            );
        }
        let checked = EvmChainChecked::new(context, std::num::NonZeroU64::new(1).unwrap()).unwrap();
        let rejected = EvmBalanceFailure::ObservationRejected {
            reason: EvmObservationRejection::Rejected,
        };
        if token {
            project::<ReadInitialEvmBalanceAnchor<K, EvmTokenBalance>>(
                &program,
                &checked,
                &rejected,
                "observation_unavailable",
            );
            let anchored = EvmTokenAnchored::new(checked, anchor.clone()).unwrap();
            project::<ReadEvmTokenDecimals<K>>(
                &program,
                &anchored,
                &rejected,
                "observation_unavailable",
            );
            context = Object::from_value(anchored.checked().context())
                .unwrap()
                .decode::<BalanceContext<K>>()
                .unwrap();
        } else {
            project::<ReadInitialEvmBalanceAnchor<K, EvmNativeBalance>>(
                &program,
                &checked,
                &rejected,
                "observation_unavailable",
            );
            context = Object::from_value(checked.context())
                .unwrap()
                .decode::<BalanceContext<K>>()
                .unwrap();
        }
        let prepared =
            PreparedBalance::new(context, point.clone(), DecimalScale::new(0).unwrap()).unwrap();
        for (failure, code) in [
            (
                ObserveBalanceFailure::ObservationUnavailable,
                "observation_unavailable",
            ),
            (ObserveBalanceFailure::IntegrityBlocked, "integrity_blocked"),
        ] {
            project::<ObserveBalance<K>>(&program, &prepared, &failure, code);
        }
        let candidate = CandidateBalance::new(prepared, maximum.clone());
        project::<ConfirmEvmBalanceAnchor<K>>(
            &program,
            &candidate,
            &rejected,
            "observation_unavailable",
        );
        project::<ConfirmEvmBalanceAnchor<K>>(
            &program,
            &candidate,
            &EvmBalanceFailure::IntegrityBlocked,
            "integrity_blocked",
        );
        let changed: EvmBalanceFailure = serde_json::from_value(serde_json::json!({
            "anchor_changed":{"previous":anchor,"observed":{"number":anchor.number,"hash":EvmHash::from_bytes([4;32])}}
        })).unwrap();
        project::<ConfirmEvmBalanceAnchor<K>>(&program, &candidate, &changed, "anchor_changed");
        context = candidate.append_confirmed().unwrap().unwrap();
    }
    let retained = Object::from_value(&context).unwrap();
    let ProposedStateOutcome::Failure { failure } =
        ConsolidateBalanceCollection::<K>::evaluate(context).unwrap()
    else {
        panic!("nine individually admitted 80-digit amounts exceed the total bound")
    };
    project::<ConsolidateBalanceCollection<K>>(
        &program,
        &retained.decode().unwrap(),
        &failure,
        "observation_unavailable",
    );
    assert_eq!(provider.0.load(Ordering::SeqCst), 0);
}

#[test]
fn admission_owns_route_order_and_commits_to_the_selected_endpoint() {
    let mut wire = serde_json::to_value(configuration(false)).unwrap();
    let mut second = wire["input"]["portfolio"]["collections"][0].clone();
    second["correlation"] = "second".into();
    for source in second["request"]["sources"].as_array_mut().unwrap() {
        source["chain_id"] = 2.into();
        source["source_id"] = format!("second-{}", source["source_id"].as_str().unwrap()).into();
    }
    wire["input"]["portfolio"]["collections"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let routes = serde_json::json!([{"chain_id":1,"endpoint_id":"expected"},{"chain_id":2,"endpoint_id":"other"}]);
    wire["input"]["routes"] = routes.clone();
    let config: EvmPortfolioConfig = serde_json::from_value(wire.clone()).unwrap();
    let input = admit_snapshot(&config, None).unwrap();
    let provider = Arc::new(Provider(AtomicUsize::new(0), None));
    let installed = PortfolioResources::new(
        [(1, "expected"), (2, "other"), (2, "replacement")]
            .into_iter()
            .map(|(chain, endpoint)| {
                (
                    EvmBalanceRoute::new(
                        std::num::NonZeroU64::new(chain).unwrap(),
                        EvmEndpoint::new(endpoint).unwrap(),
                    ),
                    provider.clone() as Arc<dyn EvmReadProvider>,
                )
            })
            .collect(),
        vec![],
    )
    .unwrap();
    let operation = mfm_portfolio::PortfolioSnapshotOperation::default();
    let program = compile(
        config.entry_point_id().unwrap(),
        &operation,
        &input,
        &installed,
        ProgramLimits::new(0),
    )
    .unwrap();
    wire["input"]["routes"][1]["endpoint_id"] = "replacement".into();
    let replaced: EvmPortfolioConfig = serde_json::from_value(wire.clone()).unwrap();
    let replacement = admit_snapshot(&replaced, None).unwrap();
    let changed = compile(
        replaced.entry_point_id().unwrap(),
        &operation,
        &replacement,
        &installed,
        ProgramLimits::new(0),
    )
    .unwrap();
    assert_ne!(program.content_ref(), changed.content_ref());
    assert_ne!(
        Object::from_value(&input).unwrap().value_ref(),
        Object::from_value(&replacement).unwrap().value_ref()
    );
    for invalid in [
        serde_json::json!([]),
        serde_json::json!([routes[0]]),
        serde_json::json!([routes[1], routes[0]]),
        serde_json::json!([routes[0], routes[0]]),
    ] {
        wire["input"]["routes"] = invalid;
        let invalid: EvmPortfolioConfig = serde_json::from_value(wire.clone()).unwrap();
        assert!(admit_snapshot(&invalid, None).is_err());
    }
    assert_eq!(provider.0.load(Ordering::SeqCst), 0);
}
