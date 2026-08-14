use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{AccessCapabilityContract, ProposedStateOutcome};
use mfm_evm::{
    EvmAccessState, EvmBalanceAsset, EvmBalanceContext, EvmBalanceRequest, EvmBalanceSource,
    EvmCapability, EvmDomainError, EvmPureState, EvmReadEvidence, EvmReadIntent, EvmReadValue,
    EvmState,
};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct OpaqueContinuation {
    marker: String,
}

type Check = EvmState<1, 0, OpaqueContinuation>;
type InitialAnchor = EvmState<1, 1, OpaqueContinuation>;
type SelectAsset = EvmState<1, 2, OpaqueContinuation>;
type NativeBalance = EvmState<1, 3, OpaqueContinuation>;
type TokenDecimals = EvmState<1, 4, OpaqueContinuation>;
type TokenBalance = EvmState<1, 5, OpaqueContinuation>;
type ConfirmAnchor = EvmState<1, 6, OpaqueContinuation>;
type Consolidate = EvmState<1, 7, OpaqueContinuation>;

fn prepare<S, C>(input: &S::Input) -> Result<C::Intent, EvmDomainError>
where
    S: EvmAccessState<C>,
    C: AccessCapabilityContract,
{
    S::prepare(input)
}

fn interpret<S, C>(
    input: S::Input,
    evidence: &C::Evidence,
) -> ProposedStateOutcome<S::Output, S::Failure>
where
    S: EvmAccessState<C>,
    C: AccessCapabilityContract,
{
    S::interpret(input, evidence)
}

fn evaluate<S: EvmPureState>(input: S::Input) -> ProposedStateOutcome<S::Output, S::Failure> {
    S::evaluate(input)
}

fn source(id: &str, token: Option<&str>) -> Value {
    json!({
        "source_id": id,
        "chain_id": 1,
        "address": "0x1111111111111111111111111111111111111111",
        "token": token,
    })
}

fn anchor(number: &str) -> Value {
    json!({
        "number": number,
        "hash": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    })
}

fn anchor_value() -> EvmReadValue {
    EvmReadValue::Anchor {
        number: "100".to_owned(),
        hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
    }
}

fn route_ref() -> mfm_ids::ContentRef {
    mfm_program::nominal_contract_ref::<EvmBalanceRequest>().expect("route reference")
}

fn result(source: Value, anchor: Value) -> Value {
    json!({
        "source": source,
        "decimals": 18,
        "raw_units": "1000000000000000000",
        "anchor": anchor,
    })
}

fn context(sources: Vec<Value>, completed: Vec<Value>, work: Value) -> Value {
    json!({
        "request": {"sources": sources, "decimals": 18},
        "caller_continuation": {"marker": "opaque"},
        "metadata": {
            "collection_ordinal": 2,
            "correlation": "collection-2",
            "route_ref": mfm_program::nominal_contract_ref::<EvmBalanceRequest>()
                .expect("route reference"),
        },
        "completed": completed,
        "work": work,
    })
}

fn assert_context(value: Value) {
    serde_json::from_value::<EvmBalanceContext<OpaqueContinuation>>(value)
        .expect("valid cumulative context");
}

fn assert_rejected(value: Value) {
    assert!(serde_json::from_value::<EvmBalanceContext<OpaqueContinuation>>(value).is_err());
}

#[test]
fn every_work_variant_reenters_the_complete_context_contract() {
    let native = source("native", None);
    let token = source("token", Some("0x2222222222222222222222222222222222222222"));
    let block = anchor("100");
    let native_work = vec![
        json!({"kind": "check_chain_identity", "source": native}),
        json!({"kind": "read_initial_anchor", "source": native, "checked_chain_id": 1}),
        json!({"kind": "select_asset", "source": native, "checked_chain_id": 1, "initial_anchor": block}),
        json!({"kind": "read_native_balance", "source": native, "checked_chain_id": 1, "initial_anchor": block}),
        json!({"kind": "confirm_anchor", "source": native, "checked_chain_id": 1, "initial_anchor": block, "source_decimals": 18, "raw_balance": "1"}),
    ];
    for work in native_work {
        assert_context(context(vec![native.clone()], Vec::new(), work));
    }
    for work in [
        json!({"kind": "read_token_decimals", "source": token, "checked_chain_id": 1, "initial_anchor": block}),
        json!({"kind": "read_token_balance", "source": token, "checked_chain_id": 1, "initial_anchor": block, "token_decimals": 18}),
    ] {
        assert_context(context(vec![token.clone()], Vec::new(), work));
    }
    assert_context(context(
        vec![native.clone()],
        vec![result(native, block)],
        json!({"kind": "complete"}),
    ));
}

#[test]
fn hostile_work_and_prefix_combinations_fail_before_interpretation() {
    let first_native = source("first", None);
    let second_native = source("second", None);
    let first_token = source(
        "first-token",
        Some("0x2222222222222222222222222222222222222222"),
    );
    let second_token = source(
        "second-token",
        Some("0x3333333333333333333333333333333333333333"),
    );
    let block = anchor("100");
    let prefix = vec![result(first_native.clone(), block.clone())];
    for work in [
        json!({"kind": "check_chain_identity", "source": first_native}),
        json!({"kind": "read_initial_anchor", "source": first_native, "checked_chain_id": 1}),
        json!({"kind": "select_asset", "source": first_native, "checked_chain_id": 1, "initial_anchor": block}),
        json!({"kind": "read_native_balance", "source": first_native, "checked_chain_id": 1, "initial_anchor": block}),
        json!({"kind": "confirm_anchor", "source": first_native, "checked_chain_id": 1, "initial_anchor": block, "source_decimals": 18, "raw_balance": "1"}),
    ] {
        assert_rejected(context(
            vec![source("first", None), second_native.clone()],
            prefix.clone(),
            work,
        ));
    }
    let token_prefix = vec![result(first_token.clone(), block.clone())];
    for work in [
        json!({"kind": "read_token_decimals", "source": first_token, "checked_chain_id": 1, "initial_anchor": block}),
        json!({"kind": "read_token_balance", "source": first_token, "checked_chain_id": 1, "initial_anchor": block, "token_decimals": 18}),
    ] {
        assert_rejected(context(
            vec![
                source(
                    "first-token",
                    Some("0x2222222222222222222222222222222222222222"),
                ),
                second_token.clone(),
            ],
            token_prefix.clone(),
            work,
        ));
    }
    assert_rejected(context(
        vec![source("first", None), second_native],
        prefix,
        json!({"kind": "complete"}),
    ));
    assert_rejected(context(
        vec![source(
            "token",
            Some("0x2222222222222222222222222222222222222222"),
        )],
        Vec::new(),
        json!({"kind": "read_native_balance", "source": source("token", Some("0x2222222222222222222222222222222222222222")), "checked_chain_id": 1, "initial_anchor": anchor("100")}),
    ));
    assert_rejected(context(
        vec![source("native", None)],
        Vec::new(),
        json!({"kind": "read_token_decimals", "source": source("native", None), "checked_chain_id": 1, "initial_anchor": anchor("100")}),
    ));
    assert_rejected(context(
        vec![source(
            "token",
            Some("0x2222222222222222222222222222222222222222"),
        )],
        Vec::new(),
        json!({"kind": "read_token_balance", "source": source("token", Some("0x2222222222222222222222222222222222222222")), "checked_chain_id": 1, "initial_anchor": anchor("100"), "token_decimals": 31}),
    ));
    let mut missing_route = context(
        vec![source("native", None)],
        Vec::new(),
        json!({"kind": "check_chain_identity", "source": source("native", None)}),
    );
    missing_route["metadata"]
        .as_object_mut()
        .expect("metadata object")
        .remove("route_ref");
    assert_rejected(missing_route);
}

#[test]
fn a_later_source_cannot_switch_the_collection_anchor() {
    let first = source("first", None);
    let second = source("second", None);
    assert_rejected(context(
        vec![first.clone(), second.clone()],
        vec![result(first, anchor("100"))],
        json!({
            "kind": "select_asset",
            "source": second,
            "checked_chain_id": 1,
            "initial_anchor": anchor("101"),
        }),
    ));
    assert_rejected(context(
        vec![source("native", None)],
        Vec::new(),
        json!({
            "kind": "confirm_anchor",
            "source": source("native", None),
            "checked_chain_id": 1,
            "initial_anchor": anchor("100"),
            "source_decimals": 17,
            "raw_balance": "1",
        }),
    ));
}

#[test]
fn completed_prefix_rejects_the_deleted_derived_scaled_amount() {
    let source = source("native", None);
    let mut completed = result(source.clone(), anchor("100"));
    completed["amount_scaled"] = Value::String("2".to_owned());
    assert_rejected(context(
        vec![source],
        vec![completed],
        json!({"kind": "complete"}),
    ));
}

fn returned(_intent: &EvmReadIntent, value: EvmReadValue) -> EvmReadEvidence {
    EvmReadEvidence::Returned { value }
}

fn rejected(_intent: &EvmReadIntent) -> EvmReadEvidence {
    EvmReadEvidence::Rejected
}

fn success<T, F>(outcome: ProposedStateOutcome<T, F>) -> T {
    match outcome {
        ProposedStateOutcome::Success { output, .. } => output,
        ProposedStateOutcome::Failure { .. } => panic!("expected state success"),
    }
}

fn assert_failure<T, F>(outcome: ProposedStateOutcome<T, F>) {
    assert!(matches!(outcome, ProposedStateOutcome::Failure { .. }));
}

fn balance_request(token: Option<&str>) -> EvmBalanceRequest {
    EvmBalanceRequest::new(
        vec![EvmBalanceSource {
            source_id: "source".to_owned(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".to_owned(),
            token: token.map(str::to_owned),
        }],
        18,
    )
    .expect("request")
}

fn initial_context(token: Option<&str>) -> EvmBalanceContext<OpaqueContinuation> {
    EvmBalanceContext::new(
        balance_request(token),
        OpaqueContinuation {
            marker: "opaque".to_owned(),
        },
        2,
        "collection-2".to_owned(),
        mfm_program::nominal_contract_ref::<EvmBalanceRequest>().expect("route reference"),
    )
    .expect("context")
}

fn canonical(value: &Value) -> String {
    PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(value).expect("JSON"))
        .expect("canonical JSON")
        .as_str()
        .to_owned()
}

#[test]
fn native_and_token_paths_have_the_exact_stage_order_and_preserve_match_payload() {
    let context = initial_context(None);
    let intent = prepare::<Check, EvmCapability<2>>(&context).expect("chain intent");
    assert_eq!(intent.route_ref(), Some(&route_ref()));
    let context = success(interpret::<Check, EvmCapability<2>>(
        context,
        &returned(&intent, EvmReadValue::ChainId(1)),
    ));
    let intent = prepare::<InitialAnchor, EvmCapability<6>>(&context).expect("anchor intent");
    let context = success(interpret::<InitialAnchor, EvmCapability<6>>(
        context,
        &returned(&intent, anchor_value()),
    ));
    let expected_match_context = canonical(&serde_json::to_value(&context).expect("context JSON"));
    let asset = success(evaluate::<SelectAsset>(context));
    let asset_json = serde_json::to_value(&asset).expect("asset JSON");
    let payload = canonical(asset_json.get("value").expect("Match payload"));
    assert_eq!(
        sha256_digest_bytes(payload.as_bytes()),
        sha256_digest_bytes(expected_match_context.as_bytes())
    );
    let EvmBalanceAsset::Native(context) = asset else {
        panic!("native source must select native arm");
    };
    let intent = prepare::<NativeBalance, EvmCapability<7>>(&context).expect("native intent");
    let context = success(interpret::<NativeBalance, EvmCapability<7>>(
        context,
        &returned(
            &intent,
            EvmReadValue::RawUnits("1000000000000000000".to_owned()),
        ),
    ));
    let intent = prepare::<ConfirmAnchor, EvmCapability<6>>(&context).expect("confirm intent");
    let context = success(interpret::<ConfirmAnchor, EvmCapability<6>>(
        context,
        &returned(&intent, anchor_value()),
    ));
    assert!(matches!(
        serde_json::to_value(&context).expect("complete context")["work"]["kind"],
        Value::String(ref value) if value == "complete"
    ));
    let completion = success(evaluate::<Consolidate>(context));
    let (_, collection_ordinal, _, _, _, _, _) = completion.into_parts();
    assert_eq!(collection_ordinal, 2);

    let context = initial_context(Some("0x2222222222222222222222222222222222222222"));
    let intent = prepare::<Check, EvmCapability<2>>(&context).expect("chain intent");
    let context = success(interpret::<Check, EvmCapability<2>>(
        context,
        &returned(&intent, EvmReadValue::ChainId(1)),
    ));
    let intent = prepare::<InitialAnchor, EvmCapability<6>>(&context).expect("anchor intent");
    let context = success(interpret::<InitialAnchor, EvmCapability<6>>(
        context,
        &returned(&intent, anchor_value()),
    ));
    let asset = success(evaluate::<SelectAsset>(context));
    let EvmBalanceAsset::Token(context) = asset else {
        panic!("token source must select token arm");
    };
    let intent = prepare::<TokenDecimals, EvmCapability<7>>(&context).expect("decimals intent");
    let context = success(interpret::<TokenDecimals, EvmCapability<7>>(
        context,
        &returned(&intent, EvmReadValue::TokenDecimals(18)),
    ));
    let intent = prepare::<TokenBalance, EvmCapability<7>>(&context).expect("token intent");
    let context = success(interpret::<TokenBalance, EvmCapability<7>>(
        context,
        &returned(&intent, EvmReadValue::RawUnits("1".to_owned())),
    ));
    let intent = prepare::<ConfirmAnchor, EvmCapability<6>>(&context).expect("confirm intent");
    let context = success(interpret::<ConfirmAnchor, EvmCapability<6>>(
        context,
        &returned(&intent, anchor_value()),
    ));
    assert!(matches!(
        serde_json::to_value(&context).expect("complete context")["work"]["kind"],
        Value::String(ref value) if value == "complete"
    ));
}

#[test]
fn every_balance_read_failure_stops_its_current_path() {
    let context = initial_context(None);
    let intent = prepare::<Check, EvmCapability<2>>(&context).expect("chain intent");
    assert_failure(interpret::<Check, EvmCapability<2>>(
        context,
        &rejected(&intent),
    ));

    let context = initial_context(None);
    let intent = prepare::<Check, EvmCapability<2>>(&context).expect("chain intent");
    let context = success(interpret::<Check, EvmCapability<2>>(
        context,
        &returned(&intent, EvmReadValue::ChainId(1)),
    ));
    let intent = prepare::<InitialAnchor, EvmCapability<6>>(&context).expect("anchor intent");
    assert_failure(interpret::<InitialAnchor, EvmCapability<6>>(
        context,
        &rejected(&intent),
    ));

    let context = initial_context(None);
    let intent = prepare::<Check, EvmCapability<2>>(&context).expect("chain intent");
    let context = success(interpret::<Check, EvmCapability<2>>(
        context,
        &returned(&intent, EvmReadValue::ChainId(1)),
    ));
    let intent = prepare::<InitialAnchor, EvmCapability<6>>(&context).expect("anchor intent");
    let context = success(interpret::<InitialAnchor, EvmCapability<6>>(
        context,
        &returned(&intent, anchor_value()),
    ));
    let asset = success(evaluate::<SelectAsset>(context));
    let EvmBalanceAsset::Native(context) = asset else {
        panic!("native source must select native arm");
    };
    let intent = prepare::<NativeBalance, EvmCapability<7>>(&context).expect("native intent");
    assert_failure(interpret::<NativeBalance, EvmCapability<7>>(
        context,
        &rejected(&intent),
    ));

    let context = initial_context(Some("0x2222222222222222222222222222222222222222"));
    let intent = prepare::<Check, EvmCapability<2>>(&context).expect("chain intent");
    let context = success(interpret::<Check, EvmCapability<2>>(
        context,
        &returned(&intent, EvmReadValue::ChainId(1)),
    ));
    let intent = prepare::<InitialAnchor, EvmCapability<6>>(&context).expect("anchor intent");
    let context = success(interpret::<InitialAnchor, EvmCapability<6>>(
        context,
        &returned(&intent, anchor_value()),
    ));
    let asset = success(evaluate::<SelectAsset>(context));
    let EvmBalanceAsset::Token(context) = asset else {
        panic!("token source must select token arm");
    };
    let intent = prepare::<TokenDecimals, EvmCapability<7>>(&context).expect("decimal intent");
    assert_failure(interpret::<TokenDecimals, EvmCapability<7>>(
        context,
        &rejected(&intent),
    ));

    let context = initial_context(Some("0x2222222222222222222222222222222222222222"));
    let intent = prepare::<Check, EvmCapability<2>>(&context).expect("chain intent");
    let context = success(interpret::<Check, EvmCapability<2>>(
        context,
        &returned(&intent, EvmReadValue::ChainId(1)),
    ));
    let intent = prepare::<InitialAnchor, EvmCapability<6>>(&context).expect("anchor intent");
    let context = success(interpret::<InitialAnchor, EvmCapability<6>>(
        context,
        &returned(&intent, anchor_value()),
    ));
    let asset = success(evaluate::<SelectAsset>(context));
    let EvmBalanceAsset::Token(context) = asset else {
        panic!("token source must select token arm");
    };
    let intent = prepare::<TokenDecimals, EvmCapability<7>>(&context).expect("decimal intent");
    let context = success(interpret::<TokenDecimals, EvmCapability<7>>(
        context,
        &returned(&intent, EvmReadValue::TokenDecimals(18)),
    ));
    let intent = prepare::<TokenBalance, EvmCapability<7>>(&context).expect("token intent");
    assert_failure(interpret::<TokenBalance, EvmCapability<7>>(
        context,
        &rejected(&intent),
    ));

    let context = initial_context(None);
    let intent = prepare::<Check, EvmCapability<2>>(&context).expect("chain intent");
    let context = success(interpret::<Check, EvmCapability<2>>(
        context,
        &returned(&intent, EvmReadValue::ChainId(1)),
    ));
    let intent = prepare::<InitialAnchor, EvmCapability<6>>(&context).expect("anchor intent");
    let context = success(interpret::<InitialAnchor, EvmCapability<6>>(
        context,
        &returned(&intent, anchor_value()),
    ));
    let asset = success(evaluate::<SelectAsset>(context));
    let EvmBalanceAsset::Native(context) = asset else {
        panic!("native source must select native arm");
    };
    let intent = prepare::<NativeBalance, EvmCapability<7>>(&context).expect("native intent");
    let context = success(interpret::<NativeBalance, EvmCapability<7>>(
        context,
        &returned(&intent, EvmReadValue::RawUnits("1".to_owned())),
    ));
    let intent = prepare::<ConfirmAnchor, EvmCapability<6>>(&context).expect("confirm intent");
    assert_failure(interpret::<ConfirmAnchor, EvmCapability<6>>(
        context,
        &rejected(&intent),
    ));
}
