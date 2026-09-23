//! Available handler values must be checked before any native resource is bound.
#[allow(dead_code)]
#[path = "native_construction/support.rs"]
mod support;
use mfm_ids::{EntryPointId, StableId, StatePosition};
use mfm_program::*;
use mfm_program_derive::MfmValue;
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use support::{Alternate, Configured, Observation, ProviderScript, Resources, Support};

static DECODES: AtomicUsize = AtomicUsize::new(0);

// Deliberately non-Clone; checked parameters are captured once, not copied for invocation.
#[derive(Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Bounds {
    low: u64,
    high: u64,
}
impl<'de> Deserialize<'de> for Bounds {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            low: u64,
            high: u64,
        }
        let wire = Wire::deserialize(decoder)?;
        DECODES.fetch_add(1, Ordering::SeqCst);
        if wire.low == 99 {
            panic!("unreviewed parameter decoder payload");
        }
        if wire.low > wire.high {
            return Err(serde::de::Error::custom("low must not exceed high"));
        }
        Ok(Self {
            low: wire.low,
            high: wire.high,
        })
    }
}
struct WithinBounds;
impl Handler for WithinBounds {
    type Params = Bounds;
    fn implementation_id() -> Result<StableId> {
        Ok(StableId::new("mfm.test/checked-handler@1")?)
    }
    fn handle(
        params: &Bounds,
        _: Classification,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, InvocationDiagnostic> {
        Ok(if params.low < params.high {
            RecoveryRequest::RetryState
        } else {
            RecoveryRequest::Stop
        })
    }
}
struct Defaults;
impl OperationDefaults for Defaults {
    type Handler = WithinBounds;
    type Targets = ();
}
impl ResolveDefaults<Bounds> for Defaults {
    fn resolve(params: &Bounds) -> Result<PolicyValues<WithinBounds>> {
        Ok(PolicyValues {
            handler: Some(Bounds {
                low: params.low,
                high: params.high,
            }),
            retries: Some(1),
            restarts: None,
        })
    }
}
struct Definition {
    bounds: Bounds,
}
impl OperationDefinition for Definition {
    type Body = ResolvedRead<Support, Observation, Alternate>;
}
impl Plan<Configured> for Definition {
    type Config = Bounds;
    fn plan<'a>(&'a self, input: &'a Configured) -> Result<(&'a Bounds, Self::Body)> {
        Ok((
            &self.bounds,
            ResolvedRead::new(Configured { value: input.value }),
        ))
    }
}
struct Environment(Resources<true>);
impl ProgramEnvironment for Environment {
    type Sources = (
        ResolvedRead<Support, Observation, Alternate>,
        Operation<Definition, Defaults>,
        ResolvedRead<Support, Observation, CheckedBinding>,
    );
}
impl BindRead<Observation, Alternate> for Environment {
    type Adapter = <Resources<true> as BindRead<Observation, Alternate>>::Adapter;
    fn bind_read(
        &self,
        binding: &Configured,
    ) -> std::result::Result<Self::Adapter, InvocationDiagnostic> {
        <Resources<true> as BindRead<Observation, Alternate>>::bind_read(&self.0, binding)
    }
}

#[test]
fn typed_parameters_reject_before_any_binding_and_decode_once_per_constructed_handler() {
    let bound = Arc::new(Mutex::new(Vec::new()));
    let resources = Environment(Resources {
        family: std::marker::PhantomData,
        bindings: Arc::clone(&bound),
        primary_available: false,
        execution: ProviderScript::Forbidden,
    });
    let entry = EntryPointId::new("mfm.test/checked-handler@1").unwrap();
    let source = (
        ResolvedRead::<Support, Observation, Alternate>::new(Configured { value: 0 }),
        Operation::<_, Defaults>::from(Definition {
            bounds: Bounds { low: 1, high: 2 },
        }),
    );
    let program = compile(
        entry.clone(),
        &source,
        &Configured { value: 1 },
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    assert_eq!(*bound.lock().unwrap(), [0, 1]);
    assert_eq!(DECODES.load(Ordering::SeqCst), 1);
    bound.lock().unwrap().clear();
    let cold = load(program.canonical_bytes(), &resources).unwrap();
    assert_eq!(cold.content_ref(), program.content_ref());
    assert_eq!(DECODES.load(Ordering::SeqCst), 2);
    let context = RecoveryContext::new(
        ExecutionPhase::Read,
        RecoveryAllowances::new(1, 0),
        1,
        &[],
        &[],
    );
    for executable in [&program, &cold] {
        for _ in 0..3 {
            assert_eq!(
                executable
                    .executable(StatePosition::new(1).unwrap())
                    .unwrap()
                    .request(Classification::Retryable, &context)
                    .unwrap(),
                RecoveryRequest::RetryState
            );
        }
    }
    assert_eq!(DECODES.load(Ordering::SeqCst), 2);
    for (low, high, reason) in [(2, 1, "low must not exceed high"), (99, 100, "panicked")] {
        bound.lock().unwrap().clear();
        let source = (
            ResolvedRead::<Support, Observation, Alternate>::new(Configured { value: 0 }),
            Operation::<_, Defaults>::from(Definition {
                bounds: Bounds { low, high },
            }),
        );
        let Err(ProgramError::Diagnostic(cause)) = compile(
            entry.clone(),
            &source,
            &Configured { value: 1 },
            &resources,
            ProgramLimits::new(1),
        ) else {
            panic!("invalid fresh parameters accepted");
        };
        let diagnostic = serde_json::to_string(&cause).unwrap();
        assert!(diagnostic.contains(reason), "{diagnostic}");
        assert!(!diagnostic.contains("unreviewed parameter decoder payload"));
        assert!(bound.lock().unwrap().is_empty());

        let mut wire: serde_json::Value =
            serde_json::from_slice(program.canonical_bytes()).unwrap();
        wire["declarations"][1]["handler"]["params"] =
            serde_json::to_value(PolicyParams::new(&Bounds { low, high }).unwrap()).unwrap();
        let canonical =
            mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&wire.to_string()).unwrap();
        let Err(ProgramError::Diagnostic(cause)) = load(canonical.as_bytes(), &resources) else {
            panic!("invalid cold parameters accepted");
        };
        let diagnostic = serde_json::to_string(&cause).unwrap();
        assert!(diagnostic.contains(reason), "{diagnostic}");
        assert!(!diagnostic.contains("unreviewed parameter decoder payload"));
        assert!(bound.lock().unwrap().is_empty());
    }
    // A typed-invalid binding in a later occurrence also prevents earlier resource attachment.
    type CheckedRead = ResolvedRead<Support, Observation, CheckedBinding>;
    let source = (
        CheckedRead::new(Bounds { low: 0, high: 1 }),
        CheckedRead::new(Bounds { low: 1, high: 2 }),
    );
    let program = compile(
        entry.clone(),
        &source,
        &Configured { value: 1 },
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    bound.lock().unwrap().clear();
    let source = (
        CheckedRead::new(Bounds { low: 0, high: 1 }),
        CheckedRead::new(Bounds { low: 2, high: 1 }),
    );
    assert!(compile(
        entry,
        &source,
        &Configured { value: 1 },
        &resources,
        ProgramLimits::new(0)
    )
    .is_err());
    assert!(bound.lock().unwrap().is_empty());
    let mut wire: serde_json::Value = serde_json::from_slice(program.canonical_bytes()).unwrap();
    let old = wire["declarations"][1]["execution"]["binding_ref"].clone();
    let invalid = mfm_values::Object::from_value(&Bounds { low: 2, high: 1 }).unwrap();
    wire["declarations"][1]["execution"]["binding_ref"] =
        serde_json::to_value(invalid.value_ref()).unwrap();
    let bindings = wire["bindings"].as_array_mut().unwrap();
    let index = bindings
        .iter()
        .position(|value| value["value_ref"] == old)
        .unwrap();
    bindings[index] = serde_json::to_value(&invalid).unwrap();
    bindings.sort_by_key(|value| {
        serde_json::from_value::<mfm_values::Object>(value.clone())
            .unwrap()
            .value_ref()
            .clone()
    });
    let canonical =
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&wire.to_string()).unwrap();
    let Err(ProgramError::Diagnostic(cause)) = load(canonical.as_bytes(), &resources) else {
        panic!("invalid stored binding accepted");
    };
    assert!(serde_json::to_string(&cause)
        .unwrap()
        .contains("low must not exceed high"));
    assert!(bound.lock().unwrap().is_empty());
}

struct CheckedBinding;
impl mfm_capabilities::ReadImplementation<Observation> for CheckedBinding {
    type Binding = Bounds;
    type NativeIntent = Configured;
    type NativeEvidence = support::Observed;
    type OperationalError = Never;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test/checked-binding@1")?)
    }
    fn encode_intent(
        _: &mfm_ids::ContentRef,
        _: &mfm_ids::ContentRef,
        _: &Bounds,
        _: &Configured,
    ) -> std::result::Result<Configured, mfm_capabilities::CallbackFailure> {
        panic!("construction must not prepare a Read")
    }
    fn project_evidence(
        _: &mfm_ids::ContentRef,
        _: &mfm_ids::ContentRef,
        _: &Bounds,
        _: &mfm_ids::ContentRef,
        _: &Configured,
        _: &mfm_ids::ContentRef,
        _: &Configured,
        _: &support::Observed,
        _: &mfm_values::Object,
    ) -> std::result::Result<support::Observed, mfm_capabilities::CallbackFailure> {
        panic!("construction must not project evidence")
    }
}
impl InjectRead<Support, Observation> for CheckedBinding {
    type Prefix = Identity<Configured>;
    type Suffix = Identity<Configured>;
    fn surround(_: &Bounds) -> Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
impl BindRead<Observation, CheckedBinding> for Environment {
    type Adapter = Self;
    fn bind_read(
        &self,
        binding: &Bounds,
    ) -> std::result::Result<Self::Adapter, InvocationDiagnostic> {
        self.0.bindings.lock().unwrap().push(binding.low);
        Ok(Self(Resources {
            family: std::marker::PhantomData,
            bindings: Arc::clone(&self.0.bindings),
            primary_available: false,
            execution: ProviderScript::Forbidden,
        }))
    }
}
impl mfm_capabilities::ReadAdapter<Configured, support::Observed, Never> for Environment {
    fn invoke<'a>(
        &'a self,
        _: &'a mfm_ids::ContentRef,
        _: &'a mfm_ids::ContentRef,
        _: &'a Configured,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<
                        support::Observed,
                        mfm_capabilities::AdapterError<Never>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        panic!("construction must not invoke a provider")
    }
}
