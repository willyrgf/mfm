use super::*;

use alloy_primitives::{address, b256};

const CONTRACT: Address = address!("1111111111111111111111111111111111111111");
const CALLER: Address = address!("2222222222222222222222222222222222222222");
const TARGET: Address = address!("3333333333333333333333333333333333333333");
const ANCHOR_HASH: B256 = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

fn call_check(expected_return: &[u8]) -> EvmContractCallCheck {
    call_check_with_material(
        &[0xde, 0xad],
        vec![EvmAccessListEntry::new(
            TARGET,
            vec![B256::from([0x44; 32])],
        )],
        expected_return,
    )
}

fn call_check_with_material(
    calldata: &[u8],
    access_list: Vec<EvmAccessListEntry>,
    expected_return: &[u8],
) -> EvmContractCallCheck {
    EvmContractCallCheck::new(
        CALLER,
        TARGET,
        U256::from(7),
        calldata,
        U256::from(75_000),
        access_list,
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

fn maximum_target() -> EvmContractValidationTarget {
    EvmContractValidationTarget::new(CONTRACT, EvmBlockAnchor::new(U256::MAX, ANCHOR_HASH))
        .expect("maximum validation target")
}

fn plan(code: &[u8], calls: Vec<EvmContractCallCheck>) -> EvmContractValidationPlan {
    EvmContractValidationPlan::from_config_and_target(&config(code, calls), &target())
        .expect("validation plan")
}

fn source(network: &str, chain_id: u64) -> EvmSessionEvidence {
    source_with_ref(network, chain_id, "primary")
}

fn source_with_ref(network: &str, chain_id: u64, source_ref: &str) -> EvmSessionEvidence {
    let binding =
        EvmNetworkBinding::new(LocalPublicId::new(network).expect("network id"), chain_id)
            .expect("network binding");
    EvmSessionEvidence::new(
        &binding,
        LocalPublicId::new(source_ref).expect("source"),
        LocalPublicId::new(mfm_evm_capabilities::EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
            .expect("implementation"),
    )
}

fn evidence(
    plan: &EvmContractValidationPlan,
    code_bytes: &[u8],
    returns: &[&[u8]],
) -> EvmContractValidationEvidence {
    let code = EvmCode {
        bytes: Bytes::copy_from_slice(code_bytes),
        hash: keccak256(code_bytes),
    };
    let mut builder =
        EvmContractValidationEvidenceBuilder::new(plan, code, &source("ethereum-mainnet", 1))
            .expect("validation evidence builder");
    for returned in returns {
        builder
            .push_call_response(Bytes::copy_from_slice(returned))
            .expect("call response");
    }
    builder
        .finish(EvmBlockAnchor::new(U256::from(100), ANCHOR_HASH))
        .expect("validation evidence")
}

fn unchecked_code_evidence(
    plan: &EvmContractValidationPlan,
    code: &[u8],
) -> EvmContractValidationEvidence {
    EvmContractValidationEvidence {
        observations: vec![
            EvmContractValidationObservation::Code {
                address: plan.address.clone(),
                anchor_hash: plan.anchor.hash().to_owned(),
                code: canonical_bytes(code),
            },
            EvmContractValidationObservation::AnchorByNumber {
                block: plan.anchor.clone(),
            },
        ],
        session: source("ethereum-mainnet", 1),
    }
}

fn exact_return_budget_calls() -> Vec<EvmContractCallCheck> {
    assert_eq!(
        EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES % EVM_CALL_MAX_RESPONSE_BYTES,
        0
    );
    let count = EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES / EVM_CALL_MAX_RESPONSE_BYTES;
    let expected = vec![0x5a; EVM_CALL_MAX_RESPONSE_BYTES];
    (0..count)
        .map(|_| call_check_with_material(&[], Vec::new(), &expected))
        .collect()
}

fn maximum_artifact_policy(code: &[u8]) -> EvmContractValidationConfig {
    let expected = vec![0x5a; EVM_CALL_MAX_RESPONSE_BYTES];
    let calldata = vec![0xab; EVM_TRANSACTION_DATA_MAX_BYTES];
    let mut access_list = (0..EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_ENTRIES)
        .map(|index| {
            EvmAccessListEntry::new(
                Address::from_word(U256::from(index).into()),
                (0..16)
                    .map(|key| B256::from(U256::from(key).to_be_bytes::<32>()))
                    .collect(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        access_list
            .iter()
            .map(|entry| entry.storage_keys().len())
            .sum::<usize>(),
        EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_STORAGE_KEYS
    );

    let count = EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES / EVM_CALL_MAX_RESPONSE_BYTES;
    let mut calls = (0..count)
        .map(|index| {
            EvmContractCallCheck::new(
                CALLER,
                TARGET,
                U256::MAX,
                if index
                    < EVM_CONTRACT_VALIDATION_MAX_TOTAL_CALLDATA_BYTES
                        / EVM_TRANSACTION_DATA_MAX_BYTES
                {
                    calldata.as_slice()
                } else {
                    &[] as &[u8]
                },
                U256::MAX,
                if index == 0 {
                    std::mem::take(&mut access_list)
                } else {
                    Vec::new()
                },
                &expected,
            )
            .expect("maximum call check")
        })
        .collect::<Vec<_>>();
    calls.extend((count..EVM_CONTRACT_VALIDATION_MAX_CALLS).map(|_| {
        EvmContractCallCheck::new(CALLER, TARGET, U256::MAX, [], U256::MAX, Vec::new(), [])
            .expect("maximum empty-return call check")
    }));
    EvmContractValidationConfig::new("a".repeat(128), u64::MAX, keccak256(code), calls)
        .expect("maximum validation config")
}

fn canonical_len<T: Serialize>(value: &T) -> usize {
    let json = serde_json::to_string(value).expect("serialize persisted value");
    PlainCanonicalJsonBytes::from_json_str(&json)
        .expect("canonical persisted value")
        .as_bytes()
        .len()
}

#[test]
fn reducer_validates_plan_bound_calls_code_and_final_anchor() {
    let code = [0x60, 0x00, 0x60, 0x01];
    let expected_return = [0x12, 0x34];
    let plan = plan(&code, vec![call_check(&expected_return)]);

    let builder = EvmContractValidationEvidenceBuilder::new(
        &plan,
        EvmCode {
            bytes: Bytes::copy_from_slice(&code),
            hash: keccak256(code),
        },
        &source("ethereum-mainnet", 1),
    )
    .expect("evidence builder");
    let request = builder
        .next_call_request()
        .expect("next call request")
        .expect("call");
    assert_eq!(request.from(), CALLER);
    assert_eq!(request.to(), TARGET);
    assert_eq!(request.value(), U256::from(7));
    assert_eq!(request.gas_limit(), U256::from(75_000));
    assert_eq!(request.access_list().len(), 1);
    assert_eq!(request.max_response_bytes(), expected_return.len());
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
        vec![0; EVM_CALL_MAX_RESPONSE_BYTES + 1],
    )
    .is_err());
    let calls = vec![call_check(&[]); EVM_CONTRACT_VALIDATION_MAX_CALLS + 1];
    assert!(
        EvmContractValidationConfig::new("ethereum-mainnet", 1, B256::from([1; 32]), calls,)
            .is_err()
    );
}

#[test]
fn config_admits_exact_return_budget_and_rejects_plus_one() {
    let exact = exact_return_budget_calls();
    EvmContractValidationConfig::new("ethereum-mainnet", 1, B256::from([1; 32]), exact.clone())
        .expect("exact aggregate return budget");

    let mut plus_one = exact;
    plus_one.push(call_check_with_material(&[], Vec::new(), &[0x01]));
    let error =
        EvmContractValidationConfig::new("ethereum-mainnet", 1, B256::from([1; 32]), plus_one)
            .expect_err("aggregate return budget plus one");
    assert!(error
        .to_string()
        .contains("expected returns exceeded their aggregate bound"));
}

#[test]
fn config_rejects_aggregate_call_material_plus_one() {
    let calldata = vec![0xab; EVM_TRANSACTION_DATA_MAX_BYTES];
    let exact_count =
        EVM_CONTRACT_VALIDATION_MAX_TOTAL_CALLDATA_BYTES / EVM_TRANSACTION_DATA_MAX_BYTES;
    let mut calls = (0..exact_count)
        .map(|_| call_check_with_material(&calldata, Vec::new(), &[]))
        .collect::<Vec<_>>();
    EvmContractValidationConfig::new("ethereum-mainnet", 1, B256::from([1; 32]), calls.clone())
        .expect("exact aggregate calldata budget");

    calls.push(call_check_with_material(&[0x01], Vec::new(), &[]));
    let error = EvmContractValidationConfig::new("ethereum-mainnet", 1, B256::from([1; 32]), calls)
        .expect_err("aggregate calldata budget plus one");
    assert!(error
        .to_string()
        .contains("calldata exceeded its aggregate bound"));
}

#[test]
fn maximum_policy_builds_reduces_and_fits_canonical_artifacts() {
    let code = vec![0x60; EVM_CONTRACT_CODE_MAX_BYTES];
    let config = maximum_artifact_policy(&code);
    let plan = EvmContractValidationPlan::from_config_and_target(&config, &maximum_target())
        .expect("maximum validation plan");
    assert_eq!(
        config
            .calls()
            .iter()
            .map(|call| call.expected_return_len().expect("expected return length"))
            .sum::<usize>(),
        EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES
    );
    assert_eq!(
        config
            .calls()
            .iter()
            .map(|call| {
                canonical_bytes_len(call.calldata(), EVM_TRANSACTION_DATA_MAX_BYTES)
                    .expect("calldata length")
            })
            .sum::<usize>(),
        EVM_CONTRACT_VALIDATION_MAX_TOTAL_CALLDATA_BYTES
    );
    assert_eq!(
        config
            .calls()
            .iter()
            .map(|call| call.access_list().len())
            .sum::<usize>(),
        EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_ENTRIES
    );
    assert_eq!(
        config
            .calls()
            .iter()
            .flat_map(EvmContractCallCheck::access_list)
            .map(|entry| entry.storage_keys().len())
            .sum::<usize>(),
        EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_STORAGE_KEYS
    );
    let mut builder = EvmContractValidationEvidenceBuilder::new(
        &plan,
        EvmCode {
            bytes: Bytes::copy_from_slice(&code),
            hash: keccak256(&code),
        },
        &source_with_ref(config.network_id(), u64::MAX, &"s".repeat(128)),
    )
    .expect("maximum evidence builder");
    for call in plan.calls() {
        builder
            .push_call_response(Bytes::from(
                parse_bytes(call.expected_return(), EVM_CALL_MAX_RESPONSE_BYTES)
                    .expect("expected return"),
            ))
            .expect("exact-bound response");
    }
    let evidence = builder
        .finish(EvmBlockAnchor::new(U256::MAX, ANCHOR_HASH))
        .expect("maximum evidence");
    let built = validate_evm_contract(&plan, &evidence).expect("maximum reduced evidence");
    assert_eq!(built.address(), plan.address());

    for (name, byte_len) in [
        ("config", canonical_len(&config)),
        ("plan", canonical_len(&plan)),
        ("evidence", canonical_len(&evidence)),
    ] {
        assert!(
            byte_len <= MAX_CANONICAL_ARTIFACT_BYTES,
            "worst-case {name} artifact was {byte_len} bytes"
        );
    }
}

#[test]
fn reducer_rejects_empty_mismatched_or_oversized_code() {
    let expected_code = [0x60, 0x00];
    let validation_plan = plan(&expected_code, Vec::new());
    assert!(validate_evm_contract(
        &validation_plan,
        &unchecked_code_evidence(&validation_plan, &[])
    )
    .is_err());
    assert!(validate_evm_contract(
        &validation_plan,
        &unchecked_code_evidence(&validation_plan, &[0x60, 0x01])
    )
    .is_err());

    let oversized = vec![0x60; EVM_CONTRACT_CODE_MAX_BYTES + 1];
    let oversized_plan = plan(&oversized, Vec::new());
    assert!(validate_evm_contract(
        &oversized_plan,
        &unchecked_code_evidence(&oversized_plan, &oversized),
    )
    .is_err());
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
fn reducer_rejects_changed_request_return_source_or_canonical_block() {
    let code = [0x60, 0x00];
    let expected_return = [0x01];
    let plan = plan(&code, vec![call_check(&expected_return)]);
    let valid = evidence(&plan, &code, &[&expected_return]);

    let mut changed_request = valid.clone();
    let EvmContractValidationObservation::Call { request_digest, .. } =
        &mut changed_request.observations[1]
    else {
        panic!("call observation");
    };
    request_digest.push('0');
    assert!(validate_evm_contract(&plan, &changed_request).is_err());

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
fn deserialized_over_budget_plan_and_evidence_fail_reduction() {
    let code = vec![0x60; EVM_CONTRACT_CODE_MAX_BYTES];
    let mut calls = exact_return_budget_calls();
    calls.push(call_check_with_material(&[], Vec::new(), &[0x01]));

    let valid_plan = plan(&code, Vec::new());
    let mut plan_value = serde_json::to_value(&valid_plan).expect("plan JSON");
    plan_value["calls"] = serde_json::to_value(&calls).expect("forged calls JSON");
    let forged_plan: EvmContractValidationPlan =
        serde_json::from_value(plan_value).expect("deserialize forged plan");

    let mut observations = vec![EvmContractValidationObservation::Code {
        address: forged_plan.address.clone(),
        anchor_hash: forged_plan.anchor.hash().to_owned(),
        code: canonical_bytes(&code),
    }];
    for (index, call) in calls.iter().enumerate() {
        observations.push(EvmContractValidationObservation::Call {
            call_index: u64::try_from(index).expect("call index"),
            request_digest: validation_call_request_digest(call, forged_plan.anchor())
                .expect("request digest"),
            return_data: call.expected_return.clone(),
        });
    }
    observations.push(EvmContractValidationObservation::AnchorByNumber {
        block: forged_plan.anchor.clone(),
    });
    let evidence_value = serde_json::to_value(EvmContractValidationEvidence {
        observations,
        session: source("ethereum-mainnet", 1),
    })
    .expect("evidence JSON");
    let forged_evidence: EvmContractValidationEvidence =
        serde_json::from_value(evidence_value).expect("deserialize forged evidence");

    let error = validate_evm_contract(&forged_plan, &forged_evidence)
        .expect_err("over-budget replay evidence");
    assert!(error
        .to_string()
        .contains("expected returns exceeded their aggregate bound"));
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
