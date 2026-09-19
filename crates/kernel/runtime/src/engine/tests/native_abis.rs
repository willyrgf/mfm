#[allow(dead_code)]
#[path = "../../../../program/tests/native_construction/support.rs"]
mod support;
use crate::Runtime;
use mfm_ids::{DigestBytes, EntryPointId, RunId};
use mfm_program::{compile, load, ProgramLimits, Read};
use mfm_store::MemoryStore;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use support::*;

// The same configuration-selected, recursively injected ABI fixture is used by compiler tests.
// Both alternatives now cross Runtime, persistence and cold original/result inspection.
#[tokio::test]
async fn selected_native_abis_execute_and_retain_distinct_originals_through_cold_loading() {
    for selected in [1, 2] {
        for fail in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let resources = Resources::<false> {
                family: std::marker::PhantomData,
                bindings: Arc::new(Mutex::new(Vec::new())),
                primary_available: selected == 1,
                execution: ProviderScript::Forbidden,
            };
            let input = Configured { value: selected };
            let program = compile(
                EntryPointId::new("mfm.proof/native-runtime@1").unwrap(),
                &Read::<Observe, Observation>::default(),
                &input,
                &resources,
                ProgramLimits::new(0),
            )
            .unwrap();
            let document = program.canonical_bytes().to_vec();
            drop(program);
            drop(resources);
            let executing = Resources::<true> {
                family: std::marker::PhantomData,
                bindings: Arc::new(Mutex::new(Vec::new())),
                primary_available: selected == 1,
                execution: if fail {
                    ProviderScript::Failure(calls.clone())
                } else {
                    ProviderScript::Success(calls.clone())
                },
            };
            let program = load(&document, &executing).unwrap();
            let runtime = Runtime::new(Arc::new(MemoryStore::new()));
            let run = RunId::from_digest(DigestBytes::from_array(
                [140 + selected as u8 * 2 + u8::from(fail); 32],
            ));
            let result = runtime
                .execute(run.clone(), &program, &input)
                .await
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            if fail {
                let original = result.failure().unwrap().failure().original();
                if selected == 1 {
                    assert_eq!(original.decode::<PrimaryFailure>().unwrap().code, 73);
                    assert!(original.decode::<AlternateFailure>().is_err());
                } else {
                    assert_eq!(
                        original.decode::<AlternateFailure>().unwrap().code,
                        "alternate-91"
                    );
                    assert!(original.decode::<PrimaryFailure>().is_err());
                }
            } else {
                assert_eq!(
                    result
                        .success()
                        .unwrap()
                        .decode::<Observed>()
                        .unwrap()
                        .value,
                    selected
                );
                assert!(result.success().unwrap().decode::<Deployed>().is_err());
            }
            drop(program);
            drop(executing);
            let inspecting = Resources::<true> {
                family: std::marker::PhantomData,
                bindings: Arc::new(Mutex::new(Vec::new())),
                primary_available: selected == 1,
                execution: ProviderScript::Forbidden,
            };
            let document = runtime.program_document(&run).await.unwrap();
            let cold = load(document.canonical_bytes(), &inspecting).unwrap();
            let observed = runtime.read(&run, &cold).await.unwrap();
            let resumed = runtime.resume(&run, &cold).await.unwrap();
            for view in [observed, resumed] {
                assert_eq!(view.success(), result.success());
                assert_eq!(
                    view.failure().map(|report| report.canonical_bytes()),
                    result.failure().map(|report| report.canonical_bytes())
                );
            }
        }
    }
}
