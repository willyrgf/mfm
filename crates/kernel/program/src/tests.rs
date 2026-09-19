use crate::*;
#[allow(dead_code)]
#[path = "../tests/phase_a/contracts.rs"]
mod contracts;
use contracts::{Add, Deployed};

struct Resources;
impl ProgramEnvironment for Resources {
    type Sources = Pure<Add>;
}

#[test]
fn current_program_commits_input_and_rejects_retired_or_forged_contracts() {
    let entry = EntryPointId::new("mfm.test/current-program@1").unwrap();
    let first = compile(
        entry.clone(),
        &Pure::<Add>::default(),
        &Deployed { value: 7 },
        &Resources,
        ProgramLimits::new(2),
    )
    .unwrap();
    let changed = compile(
        entry,
        &Pure::<Add>::default(),
        &Deployed { value: 8 },
        &Resources,
        ProgramLimits::new(2),
    )
    .unwrap();
    assert_ne!(first.content_ref(), changed.content_ref());
    assert_eq!(
        first.initial_value_ref(),
        mfm_values::Object::from_value(&Deployed { value: 7 })
            .unwrap()
            .value_ref()
    );
    assert_eq!(
        load(first.canonical_bytes(), &Resources)
            .unwrap()
            .content_ref(),
        first.content_ref()
    );
    let current: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
    for case in 0..8 {
        let mut wire = current.clone();
        match case {
            0 => wire["domain"] = serde_json::json!("mfm.program.v8"),
            1 => wire["declarations"][0]["classifier"] = serde_json::json!({}),
            2 => wire["declarations"][0]["handler"]["abi"]["input"] = serde_json::json!({}),
            3 => {
                wire["declarations"][0]["handler"]["params"] =
                    serde_json::to_value(PolicyParams::new(&Deployed { value: 42 }).unwrap())
                        .unwrap()
            }
            4 => wire["declarations"][0]["execution"]["bound"] = serde_json::json!(1024),
            5 => {
                wire["declarations"][0]["execution"]["bounds"] =
                    serde_json::json!({"prepare": 1024})
            }
            6 => wire["declarations"][0]["recovery_targets"] = serde_json::json!([2]),
            7 => {
                wire["declarations"][0]["input_contract_ref"] =
                    serde_json::to_value(nominal_contract_ref::<NoParams>().unwrap()).unwrap()
            }
            _ => unreachable!(),
        }
        let canonical = PlainCanonicalJsonBytes::from_json_str(&wire.to_string()).unwrap();
        assert!(
            load(canonical.as_bytes(), &Resources).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn standard_recovery_uses_phase_and_exactly_one_declared_eligible_target() {
    let first = RecoveryTarget {
        position: mfm_ids::StatePosition::new(0).unwrap(),
    };
    let second = RecoveryTarget {
        position: mfm_ids::StatePosition::new(1).unwrap(),
    };
    for phase in [
        ExecutionPhase::Pure,
        ExecutionPhase::Read,
        ExecutionPhase::EffectPending,
        ExecutionPhase::EffectSettled,
    ] {
        for classification in [
            Classification::Retryable,
            Classification::OutcomeUnknown,
            Classification::InputInvalidated,
            Classification::Permanent,
        ] {
            for (declared, eligible) in [
                (vec![], vec![]),
                (vec![first], vec![]),
                (vec![first], vec![first]),
                (vec![first, second], vec![first]),
            ] {
                let context = RecoveryContext::new(
                    phase,
                    RecoveryAllowances::new(2, 1),
                    3,
                    &declared,
                    &eligible,
                );
                let expected = match (classification, phase) {
                    (
                        Classification::Retryable,
                        ExecutionPhase::Read | ExecutionPhase::EffectPending,
                    ) => RecoveryRequest::RetryState,
                    (
                        Classification::InputInvalidated,
                        ExecutionPhase::Pure | ExecutionPhase::Read,
                    ) if declared.len() == 1 && eligible.contains(&first) => {
                        RecoveryRequest::Restart(first)
                    }
                    _ => RecoveryRequest::Stop,
                };
                assert_eq!(
                    StandardRecovery::handle(&NoParams, classification, &context).unwrap(),
                    expected
                );
                assert_eq!(
                    Stop::handle(&NoParams, classification, &context).unwrap(),
                    RecoveryRequest::Stop
                );
            }
        }
    }
}
