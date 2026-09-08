#![allow(dead_code)]
//! Signature/finite-selection proof only. TypeId stands in for exact content ABI refs.
//! Run with the pinned Nix shell; see ../rfc-recovery-api-sketch.md.
//! MfmValue is a marker stub: this does not prove wire, hashing, codecs, or Runtime safety.
//! Assembly methods and root-mapping lowering are signature stubs, not implementations.
//! This file is a documentation example outside the Cargo workspace.
use std::{any::TypeId, marker::PhantomData};

trait MfmValue: Send + Sync + 'static {}
impl<T: Send + Sync + 'static> MfmValue for T {}
#[derive(Debug)]
struct BuildError;
#[derive(Debug)]
struct InternalError;
type BuildResult<T> = Result<T, BuildError>;
enum Never {}
struct ContentRef;
struct ConclusionBound;
enum AdapterError<E> {
    Operational(E),
    Invariant(InternalError),
}
trait ReadCapability: Send + Sync + 'static {
    type Intent: MfmValue;
    type Evidence: MfmValue;
    type OperationalError: MfmValue;
}
trait State: Send + Sync + 'static {
    type Input: MfmValue;
    type Output: MfmValue;
    type Failure: MfmValue;
}
trait PureState: State {}
trait ReadState<C: ReadCapability>: State {
    type AdapterContext: MfmValue;
    fn adapter_context(
        input: &Self::Input,
        intent: &C::Intent,
        error: &C::OperationalError,
    ) -> Result<Self::AdapterContext, InternalError>;
}
enum Incident<D, E, C> {
    Domain(D),
    Adapter { original: E, context: C },
}
mod sealed {
    pub trait Incident {}
}
trait IncidentContract: sealed::Incident + Send + Sync + 'static {
    type Domain: MfmValue;
    type Error: MfmValue;
    type Context: MfmValue;
}
impl<D: MfmValue, E: MfmValue, C: MfmValue> sealed::Incident for Incident<D, E, C> {}
impl<D: MfmValue, E: MfmValue, C: MfmValue> IncidentContract for Incident<D, E, C> {
    type Domain = D;
    type Error = E;
    type Context = C;
}

trait ValueMap: Send + Sync + 'static {
    type Input: MfmValue;
    type Output: MfmValue;
    type Params: MfmValue;
    fn apply(p: &Self::Params, v: Self::Input) -> Result<Self::Output, InternalError>;
}
struct Identity<T>(PhantomData<T>);
impl<T: MfmValue> ValueMap for Identity<T> {
    type Input = T;
    type Output = T;
    type Params = ();
    fn apply(_: &(), v: T) -> Result<T, InternalError> {
        Ok(v)
    }
}
// The framework owns this composition. Neither map receives ownership of the error.
fn map_incident<E, DM: ValueMap, CM: ValueMap>(
    v: Incident<DM::Input, E, CM::Input>,
    dp: &DM::Params,
    cp: &CM::Params,
) -> Result<Incident<DM::Output, E, CM::Output>, InternalError> {
    Ok(match v {
        Incident::Domain(d) => Incident::Domain(DM::apply(dp, d)?),
        Incident::Adapter { original, context } => Incident::Adapter {
            original,
            context: CM::apply(cp, context)?,
        },
    })
}
#[derive(Clone, Copy)]
enum Assessment {
    Recoverable,
    Nonrecoverable,
}
enum ExecutionPhase {
    Pure,
    Read,
    EffectPending,
    EffectSettled,
}
struct RecoveryContext<'a> {
    phase: ExecutionPhase,
    targets: &'a [RecoveryTarget],
}
struct RecoveryTarget {
    position: u16,
}
enum RecoveryRequest {
    RetryState,
    Restart(RecoveryTarget),
    Stop,
}
trait Classifier<I: IncidentContract>: Send + Sync + 'static {
    type Params: MfmValue;
    fn classify(
        p: &Self::Params,
        cause: &I,
        cx: &RecoveryContext<'_>,
    ) -> Result<Assessment, InternalError>;
}
trait Handler<I: IncidentContract>: Send + Sync + 'static {
    type Params: MfmValue;
    fn handle(
        p: &Self::Params,
        cause: &I,
        a: Assessment,
        cx: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, InternalError>;
}
// Private descriptors: production uses exact schema refs, implementation IDs,
// canonical checked parameters, and bounds. They hold no executable callbacks.
#[derive(Clone)]
struct ClassifierDesc {
    source: TypeId,
    target: TypeId,
    name: &'static str,
}
#[derive(Clone)]
struct HandlerDesc {
    input: TypeId,
    name: &'static str,
}
#[derive(Clone, Default)]
struct Classifiers(Vec<ClassifierDesc>);
impl Classifiers {
    fn bind<E, DM, CM, K>(&mut self, _: DM::Params, _: CM::Params, _: K::Params) -> BuildResult<()>
    where
        E: MfmValue,
        DM: ValueMap,
        CM: ValueMap,
        K: Classifier<Incident<DM::Output, E, CM::Output>>,
    {
        let source = TypeId::of::<Incident<DM::Input, E, CM::Input>>();
        if self.0.iter().any(|b| b.source == source) {
            return Err(BuildError);
        }
        self.0.push(ClassifierDesc {
            source,
            target: TypeId::of::<Incident<DM::Output, E, CM::Output>>(),
            name: std::any::type_name::<K>(),
        });
        Ok(())
    }
}
#[derive(Clone, Default)]
struct Handlers(Vec<HandlerDesc>);
impl Handlers {
    fn bind<I: IncidentContract, H: Handler<I>>(&mut self, _: H::Params) -> BuildResult<()> {
        let input = TypeId::of::<I>();
        if self.0.iter().any(|b| b.input == input) {
            return Err(BuildError);
        }
        self.0.push(HandlerDesc {
            input,
            name: std::any::type_name::<H>(),
        });
        Ok(())
    }
}
#[derive(Clone, Default)]
struct Defaults {
    classifiers: Option<Classifiers>,
    handlers: Option<Handlers>,
}
#[derive(Default)]
struct Occurrence {
    classifiers: Option<Classifiers>,
    handlers: Option<Handlers>,
}
#[derive(Debug)]
struct Resolved {
    classifier: &'static str,
    handler: &'static str,
}
struct OperationExpansion<I, O, F> {
    defaults: Defaults,
    records: Vec<Resolved>,
    ty: PhantomData<(I, O, F)>,
}
trait Operation {
    type Input: MfmValue;
    type Output: MfmValue;
    type Failure: MfmValue;
    fn expand(
        &self,
        e: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> BuildResult<()>;
}
impl<I: MfmValue, O: MfmValue, F: MfmValue> OperationExpansion<I, O, F> {
    fn new(defaults: Defaults) -> Self {
        Self {
            defaults,
            records: vec![],
            ty: PhantomData,
        }
    }
    fn classifiers(&mut self, c: Classifiers) {
        self.defaults.classifiers = Some(c)
    }
    fn handlers(&mut self, h: Handlers) {
        self.defaults.handlers = Some(h)
    }
    fn append<T: IncidentContract>(&mut self, o: Occurrence) -> BuildResult<()> {
        let cs = o
            .classifiers
            .as_ref()
            .or(self.defaults.classifiers.as_ref())
            .ok_or(BuildError)?;
        let hs = o
            .handlers
            .as_ref()
            .or(self.defaults.handlers.as_ref())
            .ok_or(BuildError)?;
        let c =
            cs.0.iter()
                .find(|c| c.source == TypeId::of::<T>())
                .ok_or(BuildError)?;
        let h =
            hs.0.iter()
                .find(|h| h.input == c.target)
                .ok_or(BuildError)?;
        self.records.push(Resolved {
            classifier: c.name,
            handler: h.name,
        });
        Ok(())
    }
    fn pure<S, M>(
        &mut self,
        _: M::Params,
        policy: Occurrence,
        _: ConclusionBound,
    ) -> BuildResult<()>
    where
        S: PureState,
        M: ValueMap<Input = S::Failure, Output = F>,
    {
        self.append::<Incident<S::Failure, Never, ()>>(policy)
    }
    fn read<S, C, M>(
        &mut self,
        _: &ContentRef,
        _: M::Params,
        policy: Occurrence,
        _: ConclusionBound,
    ) -> BuildResult<()>
    where
        S: ReadState<C>,
        C: ReadCapability,
        M: ValueMap<Input = S::Failure, Output = F>,
    {
        self.append::<Incident<S::Failure, C::OperationalError, S::AdapterContext>>(policy)
    }
    fn operation<C: Operation, M: ValueMap<Input = C::Failure, Output = F>>(
        &mut self,
        c: &C,
        _: M::Params,
    ) -> BuildResult<()> {
        let mut child =
            OperationExpansion::<C::Input, C::Output, C::Failure>::new(self.defaults.clone());
        c.expand(&mut child)?;
        // Production appends the explicit M descriptor to child root-failure paths.
        self.records.extend(child.records);
        Ok(())
    }
}
struct Assembly;
impl Assembly {
    // Bodies are stubs. Production installs monomorphized decoding/map/classify
    // and handle trampolines; keys contain exact complete ABIs, never TypeId.
    fn register_classifier<E, DM, CM, K>(&mut self)
    where
        E: MfmValue,
        DM: ValueMap,
        CM: ValueMap,
        K: Classifier<Incident<DM::Output, E, CM::Output>>,
    {
    }
    fn register_handler<I: IncidentContract, H: Handler<I>>(&mut self) {}
    fn register_map<M: ValueMap>(&mut self) {}
    fn register_read<S: ReadState<C>, C: ReadCapability>(&mut self) {}
}
struct PortfolioFailure;
struct EvmFailure;
struct EvmContext;
struct ObservationUnavailable {
    code: u8,
}
struct EvmReadCapability;
impl ReadCapability for EvmReadCapability {
    type Intent = ();
    type Evidence = ();
    type OperationalError = ObservationUnavailable;
}
struct PortfolioState;
impl State for PortfolioState {
    type Input = ();
    type Output = ();
    type Failure = PortfolioFailure;
}
impl PureState for PortfolioState {}
struct EvmRead;
impl State for EvmRead {
    type Input = ();
    type Output = ();
    type Failure = EvmFailure;
}
impl ReadState<EvmReadCapability> for EvmRead {
    type AdapterContext = EvmContext;
    fn adapter_context(
        _: &(),
        _: &(),
        _: &ObservationUnavailable,
    ) -> Result<EvmContext, InternalError> {
        Ok(EvmContext)
    }
}
type PortfolioIncident = Incident<PortfolioFailure, Never, ()>;
type EvmIncident = Incident<EvmFailure, ObservationUnavailable, EvmContext>;
type PortfolioReadIncident = Incident<PortfolioFailure, ObservationUnavailable, EvmContext>;
struct Stop;
impl<I: IncidentContract> Handler<I> for Stop {
    type Params = ();
    fn handle(
        _: &(),
        _: &I,
        _: Assessment,
        _: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, InternalError> {
        Ok(RecoveryRequest::Stop)
    }
}
struct RetryRead;
impl Handler<EvmIncident> for RetryRead {
    type Params = ();
    fn handle(
        _: &(),
        _: &EvmIncident,
        _: Assessment,
        _: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, InternalError> {
        Ok(RecoveryRequest::RetryState)
    }
}
struct PortfolioClassifier;
impl Classifier<PortfolioIncident> for PortfolioClassifier {
    type Params = ();
    fn classify(
        _: &(),
        _: &PortfolioIncident,
        _: &RecoveryContext<'_>,
    ) -> Result<Assessment, InternalError> {
        Ok(Assessment::Nonrecoverable)
    }
}
struct EvmClassifier;
impl Classifier<EvmIncident> for EvmClassifier {
    type Params = ();
    fn classify(
        _: &(),
        _: &EvmIncident,
        _: &RecoveryContext<'_>,
    ) -> Result<Assessment, InternalError> {
        Ok(Assessment::Recoverable)
    }
}
struct PortfolioReadClassifier;
impl Classifier<PortfolioReadIncident> for PortfolioReadClassifier {
    type Params = ();
    fn classify(
        _: &(),
        _: &PortfolioReadIncident,
        _: &RecoveryContext<'_>,
    ) -> Result<Assessment, InternalError> {
        Ok(Assessment::Nonrecoverable)
    }
}
struct EvmToPortfolio;
impl ValueMap for EvmToPortfolio {
    type Input = EvmFailure;
    type Output = PortfolioFailure;
    type Params = ();
    fn apply(_: &(), _: EvmFailure) -> Result<PortfolioFailure, InternalError> {
        Ok(PortfolioFailure)
    }
}
struct EvmOperation;
impl Operation for EvmOperation {
    type Input = ();
    type Output = ();
    type Failure = EvmFailure;
    fn expand(&self, e: &mut OperationExpansion<(), (), EvmFailure>) -> BuildResult<()> {
        let mut classifiers = Classifiers::default();
        classifiers.bind::<ObservationUnavailable,Identity<EvmFailure>,Identity<EvmContext>,EvmClassifier>((),(),())?;
        e.classifiers(classifiers); // Child overrides classifier; parent handler is inherited.
        e.read::<EvmRead, EvmReadCapability, Identity<EvmFailure>>(
            &ContentRef,
            (),
            Occurrence::default(),
            ConclusionBound,
        )?;
        let mut handlers = Handlers::default();
        handlers.bind::<EvmIncident, RetryRead>(())?;
        e.read::<EvmRead, EvmReadCapability, Identity<EvmFailure>>(
            &ContentRef,
            (),
            Occurrence {
                handlers: Some(handlers),
                ..Occurrence::default()
            },
            ConclusionBound,
        )
    }
}
struct EvmInheritOperation;
impl Operation for EvmInheritOperation {
    type Input = ();
    type Output = ();
    type Failure = EvmFailure;
    fn expand(&self, e: &mut OperationExpansion<(), (), EvmFailure>) -> BuildResult<()> {
        e.read::<EvmRead, EvmReadCapability, Identity<EvmFailure>>(
            &ContentRef,
            (),
            Occurrence::default(),
            ConclusionBound,
        )
    }
}
struct PortfolioOperation;
impl Operation for PortfolioOperation {
    type Input = ();
    type Output = ();
    type Failure = PortfolioFailure;
    fn expand(&self, e: &mut OperationExpansion<(), (), PortfolioFailure>) -> BuildResult<()> {
        let mut classifiers = Classifiers::default();
        classifiers.bind::<Never, Identity<PortfolioFailure>, Identity<()>, PortfolioClassifier>(
            (),
            (),
            (),
        )?;
        classifiers.bind::<ObservationUnavailable,EvmToPortfolio,Identity<EvmContext>,PortfolioReadClassifier>((),(),())?;
        e.classifiers(classifiers);
        let mut handlers = Handlers::default();
        handlers.bind::<PortfolioIncident, Stop>(())?;
        handlers.bind::<PortfolioReadIncident, Stop>(())?;
        handlers.bind::<EvmIncident, Stop>(())?; // Explicit monomorphization; no future generic instantiation.
        e.handlers(handlers);
        e.pure::<PortfolioState, Identity<PortfolioFailure>>(
            (),
            Occurrence::default(),
            ConclusionBound,
        )?;
        e.operation::<EvmOperation, EvmToPortfolio>(&EvmOperation, ())?;
        e.operation::<EvmInheritOperation, EvmToPortfolio>(&EvmInheritOperation, ())
    }
}
fn main() -> BuildResult<()> {
    let mut e = OperationExpansion::<(), (), PortfolioFailure>::new(Defaults::default());
    PortfolioOperation.expand(&mut e)?;
    assert_eq!(e.records.len(), 4);
    assert!(e.records[0].classifier.ends_with("PortfolioClassifier"));
    assert!(e.records[1].classifier.ends_with("EvmClassifier"));
    assert!(e.records[1].handler.ends_with("Stop"));
    assert!(e.records[2].classifier.ends_with("EvmClassifier"));
    assert!(e.records[2].handler.ends_with("RetryRead"));
    assert!(e.records[3].classifier.ends_with("PortfolioReadClassifier"));
    assert!(e.records[3].handler.ends_with("Stop"));
    // Neither original error nor mapped domain/context types need Clone.
    let error = ObservationUnavailable { code: 7 };
    let context = EvmRead::adapter_context(&(), &(), &error).expect("typed State context");
    let original: EvmIncident = Incident::Adapter {
        original: error,
        context,
    };
    let mapped = map_incident::<ObservationUnavailable, EvmToPortfolio, Identity<EvmContext>>(
        original,
        &(),
        &(),
    )
    .unwrap();
    assert!(matches!(
        mapped,
        Incident::Adapter {
            original: ObservationUnavailable { code: 7 },
            ..
        }
    ));
    let domain: EvmIncident = Incident::Domain(EvmFailure);
    let mapped = map_incident::<ObservationUnavailable, EvmToPortfolio, Identity<EvmContext>>(
        domain,
        &(),
        &(),
    )
    .unwrap();
    assert!(matches!(mapped, Incident::Domain(PortfolioFailure)));
    // No fallback or generic instantiation repairs absent coverage.
    let mut missing_handlers = Handlers::default();
    missing_handlers.bind::<PortfolioIncident, Stop>(())?;
    let mut missing = OperationExpansion::<(), (), EvmFailure>::new(Defaults {
        handlers: Some(missing_handlers),
        ..Defaults::default()
    });
    assert!(EvmOperation.expand(&mut missing).is_err());
    let mut missing_classifiers = Classifiers::default();
    missing_classifiers
        .bind::<Never, Identity<PortfolioFailure>, Identity<()>, PortfolioClassifier>((), (), ())?;
    let mut covered_handlers = Handlers::default();
    covered_handlers.bind::<EvmIncident, Stop>(())?;
    let mut missing = OperationExpansion::<(), (), EvmFailure>::new(Defaults {
        classifiers: Some(missing_classifiers),
        handlers: Some(covered_handlers),
    });
    assert!(EvmInheritOperation.expand(&mut missing).is_err());
    let mut duplicate_handlers = Handlers::default();
    duplicate_handlers.bind::<EvmIncident, Stop>(())?;
    assert!(duplicate_handlers.bind::<EvmIncident, Stop>(()).is_err());
    let mut duplicate_classifiers = Classifiers::default();
    duplicate_classifiers
        .bind::<ObservationUnavailable, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>(
            (),
            (),
            (),
        )?;
    assert!(duplicate_classifiers
        .bind::<ObservationUnavailable, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>(
            (),
            (),
            ()
        )
        .is_err());
    let mut a = Assembly;
    a.register_read::<EvmRead, EvmReadCapability>();
    a.register_classifier::<Never, Identity<PortfolioFailure>, Identity<()>, PortfolioClassifier>();
    a.register_classifier::<ObservationUnavailable,Identity<EvmFailure>,Identity<EvmContext>,EvmClassifier>();
    a.register_classifier::<ObservationUnavailable,EvmToPortfolio,Identity<EvmContext>,PortfolioReadClassifier>();
    a.register_handler::<PortfolioIncident, Stop>();
    a.register_handler::<PortfolioReadIncident, Stop>();
    a.register_handler::<EvmIncident, Stop>();
    a.register_handler::<EvmIncident, RetryRead>();
    a.register_map::<EvmToPortfolio>();
    Ok(())
}
