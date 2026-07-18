use super::*;

use alloy_primitives::{address, b256};

const CONTRACT: Address = address!("1111111111111111111111111111111111111111");
const CALLER: Address = address!("2222222222222222222222222222222222222222");
const TARGET: Address = address!("3333333333333333333333333333333333333333");
const ANCHOR_HASH: B256 = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

fn call_check(expected_return: &[u8]) -> EvmContractCallCheck {
    EvmContractCallCheck::new(
        CALLER,
        TARGET,
        U256::from(7),
        [0xde, 0xad],
        U256::from(75_000),
        vec![EvmAccessListEntry::new(
            TARGET,
            vec![B256::from([0x44; 32])],
        )],
        expected_return,
    )
    .expect("call check")
}

fn config(code: &[u8], calls: Vec<EvmContractCallCheck>) -> EvmContractValidationConfig {
    EvmContractValidationConfig::new("ethereum-mainnet", 1, keccak256(code), calls)
        .expect("validation config")
}

fn target() -> EvmContractValidationTarget {
    EvmContractValidationTarget::new(CONTRACT, EvmBlockAnchor::new(U256::from(100), ANCHOR_HASH))
        .expect("validation target")
}

fn plan(code: &[u8], calls: Vec<EvmContractCallCheck>) -> EvmContractValidationPlan {
    EvmContractValidationPlan::from_config_and_target(&config(code, calls), &target())
        .expect("validation plan")
}

fn source(network: &str, chain_id: u64) -> EvmSessionEvidence {
    let binding =
        EvmNetworkBinding::new(LocalPublicId::new(network).expect("network id"), chain_id)
            .expect("network binding");
    EvmSessionEvidence::new(
        &binding,
        LocalPublicId::new("primary").expect("source"),
        LocalPublicId::new(mfm_evm_capabilities::EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
            .expect("implementation"),
    )
}

fn evidence(
    plan: &EvmContractValidationPlan,
    code_bytes: &[u8],
    returns: &[&[u8]],
) -> EvmContractValidationEvidence {
    let (address, selector) = plan.code_request().expect("code request");
    let code = EvmCode {
        bytes: Bytes::copy_from_slice(code_bytes),
        hash: keccak256(code_bytes),
    };
    let requests = plan.call_requests().expect("call requests");
    let calls = requests
        .into_iter()
        .zip(returns.iter())
        .map(|(request, returned)| (request, Bytes::copy_from_slice(returned)))
        .collect::<Vec<_>>();
    EvmContractValidationEvidence::from_observations(
        address,
        &selector,
        &code,
        &calls,
        &EvmBlock {
            number: U256::from(100),
            hash: ANCHOR_HASH,
        },
        &source("ethereum-mainnet", 1),
    )
    .expect("validation evidence")
}

#[test]
fn reducer_validates_full_call_context_code_and_final_anchor() {
    let code = [0x60, 0x00, 0x60, 0x01];
    let expected_return = [0x12, 0x34];
    let plan = plan(&code, vec![call_check(&expected_return)]);

    let request = &plan.call_requests().expect("calls")[0];
    assert_eq!(request.from(), CALLER);
    assert_eq!(request.to(), TARGET);
    assert_eq!(request.value(), U256::from(7));
    assert_eq!(request.gas_limit(), U256::from(75_000));
    assert_eq!(request.access_list().len(), 1);
    assert!(matches!(
        request.block(),
        EvmBlockSelector::ExactHash(hash) if *hash == ANCHOR_HASH
    ));

    let evidence = evidence(&plan, &code, &[&expected_return]);
    let verified = validate_evm_contract(&plan, &evidence).expect("verified contract");
    assert_eq!(verified.address(), format!("{CONTRACT:#x}"));
    assert_eq!(verified.anchor(), plan.anchor());
    assert_eq!(
        verified.observed_runtime_code_hash(),
        format!("{:#x}", keccak256(code))
    );
    assert_eq!(
        verified.validation_plan_digest(),
        validation_plan_digest(&plan).expect("plan digest")
    );
}

#[test]
fn validation_config_rejects_weak_or_unbounded_policy() {
    assert!(EvmContractValidationConfig::new(
        "ethereum-mainnet",
        0,
        B256::from([1; 32]),
        Vec::new(),
    )
    .is_err());
    assert!(
        EvmContractValidationConfig::new("ethereum-mainnet", 1, keccak256([]), Vec::new(),)
            .is_err()
    );
    assert!(
        EvmContractCallCheck::new(CALLER, TARGET, U256::ZERO, [], U256::ZERO, Vec::new(), [],)
            .is_err()
    );
    assert!(EvmContractCallCheck::new(
        CALLER,
        TARGET,
        U256::ZERO,
        [],
        U256::from(1),
        Vec::new(),
        vec![0; EVM_TRANSACTION_DATA_MAX_BYTES + 1],
    )
    .is_err());
    let calls = vec![call_check(&[]); EVM_CONTRACT_VALIDATION_MAX_CALLS + 1];
    assert!(
        EvmContractValidationConfig::new("ethereum-mainnet", 1, B256::from([1; 32]), calls,)
            .is_err()
    );
}

#[test]
fn reducer_rejects_empty_mismatched_or_oversized_code() {
    let expected_code = [0x60, 0x00];
    let validation_plan = plan(&expected_code, Vec::new());
    assert!(
        validate_evm_contract(&validation_plan, &evidence(&validation_plan, &[], &[])).is_err()
    );
    assert!(validate_evm_contract(
        &validation_plan,
        &evidence(&validation_plan, &[0x60, 0x01], &[])
    )
    .is_err());

    let oversized = vec![0x60; EVM_CONTRACT_CODE_MAX_BYTES + 1];
    let oversized_plan = plan(&oversized, Vec::new());
    assert!(
        validate_evm_contract(&oversized_plan, &evidence(&oversized_plan, &oversized, &[]),)
            .is_err()
    );
}

#[test]
fn reducer_rejects_missing_reordered_duplicated_or_extra_evidence() {
    let code = [0x60, 0x00];
    let expected_return = [0x01];
    let plan = plan(&code, vec![call_check(&expected_return)]);
    let valid = evidence(&plan, &code, &[&expected_return]);

    let mut missing = valid.clone();
    missing.observations.remove(1);
    assert!(validate_evm_contract(&plan, &missing).is_err());

    let mut reordered = valid.clone();
    reordered.observations.swap(0, 1);
    assert!(validate_evm_contract(&plan, &reordered).is_err());

    let mut duplicated = valid.clone();
    duplicated
        .observations
        .insert(1, duplicated.observations[0].clone());
    assert!(validate_evm_contract(&plan, &duplicated).is_err());

    let mut extra = valid.clone();
    extra
        .observations
        .push(extra.observations.last().expect("anchor").clone());
    assert!(validate_evm_contract(&plan, &extra).is_err());
}

#[test]
fn reducer_rejects_changed_call_context_return_source_or_canonical_block() {
    let code = [0x60, 0x00];
    let expected_return = [0x01];
    let plan = plan(&code, vec![call_check(&expected_return)]);
    let valid = evidence(&plan, &code, &[&expected_return]);

    let mut changed_context = valid.clone();
    let EvmContractValidationObservation::Call { context, .. } =
        &mut changed_context.observations[1]
    else {
        panic!("call observation");
    };
    context.target = canonical_address(CONTRACT);
    assert!(validate_evm_contract(&plan, &changed_context).is_err());

    let mut changed_return = valid.clone();
    let EvmContractValidationObservation::Call { return_data, .. } =
        &mut changed_return.observations[1]
    else {
        panic!("call observation");
    };
    *return_data = "0x02".to_owned();
    assert!(validate_evm_contract(&plan, &changed_return).is_err());

    let mut changed_source = valid.clone();
    changed_source.session = source("other", 1);
    assert!(validate_evm_contract(&plan, &changed_source).is_err());

    let mut changed_anchor = valid;
    let EvmContractValidationObservation::AnchorByNumber { block } =
        changed_anchor.observations.last_mut().expect("anchor")
    else {
        panic!("anchor observation");
    };
    *block = EvmBlockAnchor::new(U256::from(100), B256::from([0xbb; 32]));
    assert!(validate_evm_contract(&plan, &changed_anchor).is_err());
}

#[test]
fn validation_persisted_values_reject_unknown_fields() {
    let code = [0x60, 0x00];
    let validation_plan = plan(&code, Vec::new());
    let validation_evidence = evidence(&validation_plan, &code, &[]);

    let mut config_value = serde_json::to_value(config(&code, Vec::new())).expect("config JSON");
    config_value
        .as_object_mut()
        .expect("object")
        .insert("endpoint".to_owned(), serde_json::json!("forbidden"));
    assert!(serde_json::from_value::<EvmContractValidationConfig>(config_value).is_err());

    let mut plan_value = serde_json::to_value(validation_plan).expect("plan JSON");
    plan_value
        .as_object_mut()
        .expect("object")
        .insert("endpoint".to_owned(), serde_json::json!("forbidden"));
    assert!(serde_json::from_value::<EvmContractValidationPlan>(plan_value).is_err());

    let mut evidence_value = serde_json::to_value(validation_evidence).expect("evidence JSON");
    evidence_value
        .as_object_mut()
        .expect("object")
        .insert("endpoint".to_owned(), serde_json::json!("forbidden"));
    assert!(serde_json::from_value::<EvmContractValidationEvidence>(evidence_value).is_err());
}
