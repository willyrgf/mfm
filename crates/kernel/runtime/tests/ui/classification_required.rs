use mfm_capabilities::{CallbackFailure, ReadAdapter, ReadCapabilityContract, ReadImplementation};
use mfm_ids::{ContentRef, EntryPointId, StableId};
use mfm_program::*;
use mfm_values::{InvocationDiagnostic, Object};

struct Observation;
impl ReadCapabilityContract for Observation {
    type Intent = NoParams;
    type Evidence = NoParams;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.unclassified@1")?)
    }
    fn bind_evidence(
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &NoParams,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        Ok(())
    }
}
struct Observe;
impl State for Observe {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.unclassified-read@1")?)
    }
}
impl ReadState<Observation> for Observe {
    fn prepare(_: &NoParams) -> std::result::Result<NoParams, InvocationDiagnostic> {
        Ok(NoParams)
    }
    fn interpret(
        input: NoParams,
        _: &NoParams,
    ) -> std::result::Result<ProposedStateOutcome<NoParams, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl ReadSelection<Observation> for Observe {
    type ExpandedInput = NoParams;
    type ExpandedOutput = NoParams;
}
struct Native;
impl ReadImplementation<Observation> for Native {
    type Binding = NoParams;
    type NativeIntent = NoParams;
    type NativeEvidence = NoParams;
    type OperationalError = NoParams;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.unclassified-native@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &NoParams,
    ) -> std::result::Result<NoParams, CallbackFailure> {
        Ok(NoParams)
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
    ) -> std::result::Result<NoParams, CallbackFailure> {
        Ok(NoParams)
    }
}
impl InjectRead<Observe, Observation> for Native {
    type Prefix = Identity<NoParams>;
    type Suffix = Identity<NoParams>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
type Source = ResolvedRead<Observe, Observation, Native>;
struct Resources;
impl ProgramEnvironment for Resources {
    type Sources = Source;
}
impl BindRead<Observation, Native> for Resources {
    type Adapter = Self;
    fn bind_read(&self, _: &NoParams) -> std::result::Result<Self, InvocationDiagnostic> {
        Ok(Self)
    }
}
impl ReadAdapter<NoParams, NoParams, NoParams> for Resources {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        _: &'a NoParams,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<
                        NoParams,
                        mfm_capabilities::AdapterError<NoParams>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Ok(NoParams) })
    }
}
fn native_remains_reusable<I: ReadImplementation<Observation>>() {}
fn main() {
    native_remains_reusable::<Native>();
    compile(
        EntryPointId::new("mfm.test/unclassified@1").unwrap(),
        &Source::new(NoParams),
        &NoParams,
        &Resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    load(b"{}", &Resources).unwrap();
}
