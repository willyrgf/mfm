//! Native construction proof using the fixture shared with actual Runtime execution.
#[allow(dead_code)]
#[path = "native_construction/support.rs"]
mod support;
use mfm_ids::EntryPointId;
use mfm_program::*;
use std::sync::{Arc, Mutex};
use support::*;

#[test]
fn configured_native_abis_and_resolved_support_cold_load_without_resolution() {
    for selected in [1, 2] {
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let resources = Resources::<false> {
            family: std::marker::PhantomData,
            bindings: Arc::clone(&recorded),
            execution: ProviderScript::Forbidden,
            primary_available: selected == 1,
        };
        let program = compile(
            EntryPointId::new("mfm.proof/native@1").unwrap(),
            &Read::<Observe, Observation>::default(),
            &Configured { value: selected },
            &resources,
            ProgramLimits::new(0),
        )
        .unwrap();
        assert_eq!(*recorded.lock().unwrap(), [0, selected]);
        assert_eq!(program.bindings().len(), 2);
        let Execution::Read { abi, .. } = program.declarations()[1].execution() else {
            panic!("missing designated Read")
        };
        if selected == 1 {
            assert_eq!(
                abi.native_request(),
                &nominal_contract_ref::<Prepared>().unwrap()
            );
            assert_eq!(
                abi.native_evidence(),
                &nominal_contract_ref::<Deployed>().unwrap()
            );
        } else {
            assert_eq!(
                abi.native_request(),
                &nominal_contract_ref::<Configured>().unwrap()
            );
            assert_eq!(
                abi.native_evidence(),
                &nominal_contract_ref::<Observed>().unwrap()
            );
        }
        recorded.lock().unwrap().clear();
        // This environment has no Resolve implementation; only stored descriptors and public bindings remain.
        let cold_resources = Resources::<true> {
            family: std::marker::PhantomData,
            bindings: Arc::clone(&recorded),
            execution: ProviderScript::Forbidden,
            primary_available: selected == 1,
        };
        let cold = load(program.canonical_bytes(), &cold_resources).unwrap();
        assert_eq!(*recorded.lock().unwrap(), [0, selected]);
        assert_eq!(cold.canonical_bytes(), program.canonical_bytes());
        assert_eq!(cold.content_ref(), program.content_ref());
        recorded.lock().unwrap().clear();
        let mut forged: serde_json::Value =
            serde_json::from_slice(program.canonical_bytes()).unwrap();
        forged["declarations"][1]["execution"]["abi"]["native_evidence"] =
            serde_json::to_value(nominal_contract_ref::<Prepared>().unwrap()).unwrap();
        let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&forged).unwrap(),
        )
        .unwrap();
        let Err(ProgramError::Diagnostic(cause)) = load(canonical.as_bytes(), &cold_resources)
        else {
            panic!("association cause")
        };
        assert_eq!(cause.operation(), "associate");
        assert_eq!(cause.details().as_value()["reason"], "state_not_installed");
        assert_eq!(cause.details().as_value()["position"], 1);
        assert_eq!(
            cause.details().as_value()["state"],
            forged["declarations"][1]
        );
        assert!(recorded.lock().unwrap().is_empty());
        let mut wrong_mode: serde_json::Value =
            serde_json::from_slice(program.canonical_bytes()).unwrap();
        wrong_mode["declarations"][1]["execution"]["kind"] = serde_json::json!("effect");
        let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&wrong_mode).unwrap(),
        )
        .unwrap();
        assert!(load(canonical.as_bytes(), &cold_resources).is_err());
        assert!(recorded.lock().unwrap().is_empty());
        let mut missing: serde_json::Value =
            serde_json::from_slice(program.canonical_bytes()).unwrap();
        missing["bindings"].as_array_mut().unwrap().pop();
        let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&missing).unwrap(),
        )
        .unwrap();
        assert!(load(canonical.as_bytes(), &cold_resources).is_err());
        assert!(recorded.lock().unwrap().is_empty());
    }
}

#[test]
fn unsupported_and_duplicate_family_selections_fail_before_resource_binding() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let duplicate = Resources::<false, (Primary, Primary)> {
        family: std::marker::PhantomData,
        bindings: Arc::clone(&recorded),
        execution: ProviderScript::Forbidden,
        primary_available: true,
    };
    let Err(ProgramError::Diagnostic(cause)) = compile(
        EntryPointId::new("mfm.proof/duplicate@1").unwrap(),
        &Read::<Observe, Observation>::default(),
        &Configured { value: 1 },
        &duplicate,
        ProgramLimits::new(0),
    ) else {
        panic!("selection cause")
    };
    assert_eq!(cause.operation(), "select_native");
    assert_eq!(
        cause.details().as_value()["reason"],
        "duplicate_implementation"
    );
    assert_eq!(
        cause.details().as_value()["implementation"],
        "proof.primary@1"
    );
    assert_eq!(cause.details().as_value()["position"], 0);

    let unsupported = Resources::<false> {
        family: std::marker::PhantomData,
        bindings: Arc::clone(&recorded),
        execution: ProviderScript::Forbidden,
        primary_available: true,
    };
    let Err(ProgramError::Diagnostic(cause)) = compile(
        EntryPointId::new("mfm.proof/unsupported@1").unwrap(),
        &Read::<Observe, Observation>::default(),
        &Configured { value: 3 },
        &unsupported,
        ProgramLimits::new(0),
    ) else {
        panic!("selection cause")
    };
    assert_eq!(cause.operation(), "select_native");
    assert_eq!(
        cause.details().as_value()["reason"],
        "unsupported_implementation"
    );
    assert_eq!(
        cause.details().as_value()["selected"],
        "proof.uninstalled@1"
    );
    assert_eq!(
        cause.details().as_value()["available"],
        serde_json::json!(["proof.primary@1", "proof.alternate@1"])
    );
    assert_eq!(cause.details().as_value()["position"], 0);

    assert!(recorded.lock().unwrap().is_empty());
}
