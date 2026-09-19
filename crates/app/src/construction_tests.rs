//! Real compiler rejections cross the Application transport boundary without losing their causes.
use crate::{construction_failure, SerializableClientError};
use mfm_capabilities::{CallbackFailure, ReadAdapter, ReadCapabilityContract, ReadImplementation};
use mfm_ids::{ContentRef, DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::*;
use mfm_values::{InvocationDiagnostic, Object};
use std::{marker::PhantomData, result::Result};

struct ReadCapability;
impl ReadCapabilityContract for ReadCapability {
    type Intent = NoParams;
    type Evidence = NoParams;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.app/read@1")?)
    }
    fn bind_evidence(
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &NoParams,
    ) -> Result<(), InvocationDiagnostic> {
        Ok(())
    }
}
struct Pass;
impl State for Pass {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.app/pass@1")?)
    }
}
impl PureState for Pass {
    fn evaluate(
        _: NoParams,
    ) -> Result<ProposedStateOutcome<NoParams, Never>, InvocationDiagnostic> {
        panic!("construction cannot evaluate")
    }
}
impl ReadState<ReadCapability> for Pass {
    fn prepare(_: &NoParams) -> Result<NoParams, InvocationDiagnostic> {
        panic!("construction cannot prepare")
    }
    fn interpret(
        _: NoParams,
        _: &NoParams,
    ) -> Result<ProposedStateOutcome<NoParams, Never>, InvocationDiagnostic> {
        panic!("construction cannot interpret")
    }
}
impl ReadSelection<ReadCapability> for Pass {
    type ExpandedInput = NoParams;
    type ExpandedOutput = NoParams;
}
struct Native;
impl ReadImplementation<ReadCapability> for Native {
    type Binding = NoParams;
    type NativeIntent = NoParams;
    type NativeEvidence = NoParams;
    type OperationalError = Never;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.app/native@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &NoParams,
    ) -> Result<NoParams, CallbackFailure> {
        panic!("construction cannot encode an intent")
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &NoParams,
        _: &NoParams,
        _: &Object,
    ) -> Result<NoParams, CallbackFailure> {
        panic!("construction cannot project evidence")
    }
}
impl InjectRead<Pass, ReadCapability> for Native {
    type Prefix = Identity<NoParams>;
    type Suffix = Identity<NoParams>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Default::default(), Default::default()))
    }
}
impl ResolveReadBinding<NoParams, ReadCapability> for Native {
    fn binding(_: &NoParams) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
}
struct Resources<F, const MISSING: bool = false>(PhantomData<fn() -> F>);
impl<F, const MISSING: bool> ProgramEnvironment for Resources<F, MISSING> {
    type Sources = Read<Pass, ReadCapability>;
}
impl<F, const MISSING: bool> CapabilityFamily<ReadCapability> for Resources<F, MISSING> {
    type Implementations = F;
}
impl<F, const MISSING: bool> Resolve<NoParams, ReadCapability> for Resources<F, MISSING> {
    fn implementation(_: &NoParams) -> mfm_program::Result<StableId> {
        Ok(StableId::new(if MISSING {
            "mfm.test.app/uninstalled@1"
        } else {
            "mfm.test.app/native@1"
        })?)
    }
}
struct Adapter;
impl ReadAdapter<NoParams, NoParams, Never> for Adapter {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        _: &'a NoParams,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<NoParams, mfm_capabilities::AdapterError<Never>>,
                > + Send
                + 'a,
        >,
    > {
        panic!("construction cannot enter provider IO")
    }
}
impl<F, const MISSING: bool> BindRead<ReadCapability, Native> for Resources<F, MISSING> {
    type Adapter = Adapter;
    fn bind_read(&self, _: &NoParams) -> Result<Adapter, InvocationDiagnostic> {
        Ok(Adapter)
    }
}
struct Empty;
impl ProgramEnvironment for Empty {
    type Sources = Identity<NoParams>;
}
struct Marker;
impl CheckpointMarker for Marker {
    type Context = NoParams;
}
struct Defaults;
impl OperationDefaults for Defaults {
    type Handler = Stop;
    type Targets = (Marker,);
}
impl ResolveDefaults<NoParams> for Defaults {
    fn resolve(_: &NoParams) -> mfm_program::Result<PolicyValues<Stop>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: None,
            restarts: Some(1),
        })
    }
}

#[test]
fn construction_causes_retain_selection_association_and_checkpoint_facts_in_transport() {
    let entry = EntryPointId::new("mfm.test.app/construction@1").unwrap();
    let installed = Resources::<(Native,)>(PhantomData);
    let unsupported = Resources::<(Native,), true>(PhantomData);
    let duplicate = Resources::<(Native, Native)>(PhantomData);
    let source = Read::<Pass, ReadCapability>::default();
    let program = compile(
        entry.clone(),
        &source,
        &NoParams,
        &installed,
        ProgramLimits::new(0),
    )
    .unwrap();
    type Forward = Operation<(Pure<Pass>, Checkpoint<Marker>, Pure<Pass>), Defaults>;
    let cases = [
        (
            "select_native",
            compile(
                entry.clone(),
                &source,
                &NoParams,
                &unsupported,
                ProgramLimits::new(0),
            )
            .err()
            .unwrap(),
        ),
        (
            "select_native",
            compile(
                entry.clone(),
                &source,
                &NoParams,
                &duplicate,
                ProgramLimits::new(0),
            )
            .err()
            .unwrap(),
        ),
        (
            "associate",
            load(program.canonical_bytes(), &Empty).err().unwrap(),
        ),
        (
            "lower_checkpoint",
            compile(
                entry,
                &Forward::default(),
                &NoParams,
                &Empty,
                ProgramLimits::new(1),
            )
            .err()
            .unwrap(),
        ),
    ];
    for (index, (operation, error)) in cases.into_iter().enumerate() {
        let ProgramError::Diagnostic(cause) = &error else {
            panic!("structured compiler rejection")
        };
        assert_eq!(cause.operation(), operation);
        assert_eq!(cause.details().as_value()["position"], 0);
        match index {
            0 => assert_eq!(
                cause.details().as_value()["selected"],
                "mfm.test.app/uninstalled@1"
            ),
            1 => assert_eq!(
                cause.details().as_value()["implementation"],
                "mfm.test.app/native@1"
            ),
            2 => assert_eq!(
                cause.details().as_value()["state"],
                serde_json::to_value(&program.declarations()[0]).unwrap()
            ),
            _ => assert_eq!(cause.details().as_value()["target_position"], 1),
        }
        let expected = serde_json::to_value(&error).unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([91; 32]));
        let error = construction_failure(&run, "construct", &error);
        let wire = serde_json::to_value(
            SerializableClientError::for_run(&error, "construction rejected").unwrap(),
        )
        .unwrap();
        assert_eq!(wire["run_id"], run.as_str());
        assert_eq!(wire["diagnostic"]["details"], expected);
        assert!(wire.get("invocation").is_none());
        assert!(wire.get("candidate").is_none());
    }
}
