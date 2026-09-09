use super::*;
use mfm_ids::StableId;
use mfm_program::{
    Classifiers, ExecutionPhase, Handlers, Identity, NoContext, NoParams, ProgramError,
    RecoveryAllowances, StateExecutionError, Stop,
};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmFailure {
    source: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioFailure {
    collection: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum ProviderError {
    Unavailable,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmContext {
    source: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioContext {
    collection: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Offset {
    value: u64,
}

struct MapFailure;
impl ValueMap for MapFailure {
    type Input = EvmFailure;
    type Output = PortfolioFailure;
    type Params = Offset;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("test.recovery.failure-map@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn apply(
        params: &Offset,
        value: EvmFailure,
    ) -> std::result::Result<PortfolioFailure, StateExecutionError> {
        Ok(PortfolioFailure {
            collection: value
                .source
                .checked_add(params.value)
                .ok_or(StateExecutionError)?,
        })
    }
}

struct MapContext;
impl ValueMap for MapContext {
    type Input = EvmContext;
    type Output = PortfolioContext;
    type Params = Offset;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("test.recovery.context-map@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn apply(
        params: &Offset,
        value: EvmContext,
    ) -> std::result::Result<PortfolioContext, StateExecutionError> {
        Ok(PortfolioContext {
            collection: value
                .source
                .checked_add(params.value)
                .ok_or(StateExecutionError)?,
        })
    }
}

type EvmIncident = Incident<EvmFailure, ProviderError, EvmContext>;
type PortfolioIncident = Incident<PortfolioFailure, ProviderError, PortfolioContext>;

struct PortfolioClassifier;
impl Classifier<PortfolioIncident> for PortfolioClassifier {
    type Params = Offset;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("test.recovery.portfolio-classifier@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
    fn classify(
        params: &Offset,
        incident: &PortfolioIncident,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<Assessment, StateExecutionError> {
        let collection = match incident {
            Incident::Domain(value) => value.collection,
            Incident::Adapter {
                original: ProviderError::Unavailable,
                context,
            } => context.collection,
        };
        Ok(if collection == params.value {
            Assessment::Recoverable
        } else {
            Assessment::Nonrecoverable
        })
    }
}

struct EvmClassifier;
impl Classifier<EvmIncident> for EvmClassifier {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("test.recovery.evm-classifier@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn classify(
        _: &NoParams,
        _: &EvmIncident,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<Assessment, StateExecutionError> {
        Ok(Assessment::Recoverable)
    }
}

struct RetryRead;
impl<I: IncidentContract> Handler<I> for RetryRead {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("test.recovery.retry-read@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        _: &I,
        assessment: Assessment,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(
            if assessment == Assessment::Recoverable && context.phase() == ExecutionPhase::Read {
                RecoveryRequest::RetryState
            } else {
                RecoveryRequest::Stop
            },
        )
    }
}

struct ConflictingEvmClassifier;
impl Classifier<EvmIncident> for ConflictingEvmClassifier {
    type Params = NoParams;

    fn implementation_id() -> mfm_program::Result<StableId> {
        EvmClassifier::implementation_id()
    }

    fn classify(
        _: &NoParams,
        _: &EvmIncident,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<Assessment, StateExecutionError> {
        Ok(Assessment::Nonrecoverable)
    }
}

#[test]
fn recovery_association_qualifies_real_parameters_and_preserves_nonclone_originals() {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder
        .register_classifier::<ProviderError, MapFailure, MapContext, PortfolioClassifier>()
        .unwrap();
    builder
        .register_handler::<PortfolioIncident, RetryRead>()
        .unwrap();
    let assembly = builder.finish();

    let mut classifiers = Classifiers::new();
    classifiers
        .bind::<ProviderError, MapFailure, MapContext, PortfolioClassifier>(
            Offset { value: 10 },
            Offset { value: 10 },
            Offset { value: 17 },
        )
        .unwrap();
    let mut handlers = Handlers::new();
    handlers
        .bind::<PortfolioIncident, RetryRead>(NoParams)
        .unwrap();
    let classifier = classifiers
        .binding(&mfm_program::IncidentAbi::of::<EvmIncident>().unwrap())
        .unwrap();
    let handler = handlers.binding(&classifier.abi().mapped()).unwrap();
    let policy = assembly
        .inner
        .associate_recovery(classifier, handler)
        .unwrap();
    let context = RecoveryContext::new(ExecutionPhase::Read, RecoveryAllowances::new(2, 1), 3, &[]);

    let original = qualify_hot(EvmFailure { source: 7 }).unwrap();
    let retained = original.canonical.as_bytes().to_vec();
    let original_ref = original.value_ref.clone();
    assert_eq!(
        policy
            .request(QualifiedIncident::Domain(original), &context)
            .unwrap(),
        (Assessment::Recoverable, RecoveryRequest::RetryState)
    );
    // The report path starts from original canonical bytes, independently of consuming policy maps.
    let codec = assembly
        .inner
        .values
        .get(&nominal_contract_ref::<EvmFailure>().unwrap())
        .unwrap();
    let decoded = take::<EvmFailure>(codec.qualify(&original_ref, &retained).unwrap()).unwrap();
    assert_eq!(decoded.source, 7);
    assert!(matches!(
        policy.request(
            QualifiedIncident::Domain(qualify_hot(EvmFailure { source: u64::MAX }).unwrap()),
            &context,
        ),
        Err(RuntimeError::Internal)
    ));
    let original = qualify_hot(ProviderError::Unavailable).unwrap();
    let bytes = original.canonical.as_bytes().to_vec();
    assert_eq!(
        policy
            .request(
                QualifiedIncident::Adapter {
                    original,
                    context: Box::new(qualify_hot(EvmContext { source: 7 }).unwrap()),
                },
                &context
            )
            .unwrap(),
        (Assessment::Recoverable, RecoveryRequest::RetryState)
    );
    assert!(matches!(
        serde_json::from_slice::<ProviderError>(&bytes).unwrap(),
        ProviderError::Unavailable
    ));
    let pending = RecoveryContext::new(
        ExecutionPhase::EffectPending,
        RecoveryAllowances::new(2, 1),
        3,
        &[],
    );
    assert_eq!(
        policy
            .request(
                QualifiedIncident::Adapter {
                    original: qualify_hot(ProviderError::Unavailable).unwrap(),
                    context: Box::new(qualify_hot(EvmContext { source: 7 }).unwrap()),
                },
                &pending
            )
            .unwrap(),
        (Assessment::Recoverable, RecoveryRequest::Stop)
    );
}

#[test]
fn recovery_association_rejects_missing_and_incompatible_exact_bindings() {
    let mut classifiers = Classifiers::new();
    classifiers
        .bind::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>(
            NoParams, NoParams, NoParams,
        )
        .unwrap();
    assert!(classifiers
        .bind::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>(
            NoParams, NoParams, NoParams
        )
        .is_err());
    let source = mfm_program::IncidentAbi::of::<EvmIncident>().unwrap();
    let classifier = classifiers.binding(&source).unwrap();
    assert!(classifiers
        .binding(&mfm_program::IncidentAbi::of::<PortfolioIncident>().unwrap())
        .is_err());

    let mut handlers = Handlers::new();
    handlers.bind::<EvmIncident, Stop>(NoParams).unwrap();
    handlers.bind::<PortfolioIncident, Stop>(NoParams).unwrap();
    assert!(handlers.bind::<EvmIncident, RetryRead>(NoParams).is_err());
    let matching = handlers.binding(&source).unwrap();
    let mismatched = handlers
        .binding(&mfm_program::IncidentAbi::of::<PortfolioIncident>().unwrap())
        .unwrap();
    let empty = RuntimeAssemblyBuilder::new().unwrap().finish();
    assert!(matches!(
        empty.inner.associate_recovery(classifier, matching),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_classifier::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>().unwrap();
    builder.register_classifier::<ProviderError, Identity<EvmFailure>, Identity<EvmContext>, EvmClassifier>().unwrap();
    builder.register_handler::<EvmIncident, Stop>().unwrap();
    builder
        .register_handler::<PortfolioIncident, Stop>()
        .unwrap();
    assert!(matches!(
        builder.register_classifier::<
            ProviderError,
            Identity<EvmFailure>,
            Identity<EvmContext>,
            ConflictingEvmClassifier,
        >(),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    let assembly = builder.finish();
    assert!(matches!(
        assembly.inner.associate_recovery(classifier, mismatched),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    let policy = assembly
        .inner
        .associate_recovery(classifier, matching)
        .unwrap();
    let context = RecoveryContext::new(ExecutionPhase::Read, RecoveryAllowances::new(1, 0), 1, &[]);
    assert_eq!(
        policy
            .request(
                QualifiedIncident::Domain(qualify_hot(EvmFailure { source: 1 }).unwrap()),
                &context
            )
            .unwrap(),
        (Assessment::Recoverable, RecoveryRequest::Stop)
    );

    let mut override_handlers = Handlers::new();
    override_handlers
        .bind::<EvmIncident, RetryRead>(NoParams)
        .unwrap();
    assert!(matches!(
        assembly
            .inner
            .associate_recovery(classifier, override_handlers.binding(&source).unwrap()),
        Err(RuntimeError::IncompatibleAssembly)
    ));
}

#[test]
fn recovery_framework_units_have_distinct_exact_contracts() {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_value::<NoParams>().unwrap();
    builder.register_value::<NoContext>().unwrap();
    assert_ne!(
        nominal_contract_ref::<NoParams>().unwrap(),
        nominal_contract_ref::<NoContext>().unwrap()
    );
    assert_eq!(qualify_hot(NoParams).unwrap().canonical.as_bytes(), b"null");
}

#[test]
fn recovery_root_mapping_associates_an_exact_ordered_path_from_the_original() {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_map::<MapFailure>().unwrap();
    builder
        .register_map::<Identity<PortfolioFailure>>()
        .unwrap();
    let assembly = builder.finish();
    let input = nominal_contract_ref::<EvmFailure>().unwrap();
    let output = nominal_contract_ref::<PortfolioFailure>().unwrap();
    let path = [
        MapBinding::new::<MapFailure>(&Offset { value: 100 }).unwrap(),
        MapBinding::new::<Identity<PortfolioFailure>>(&NoParams).unwrap(),
    ];
    let root = assembly
        .inner
        .associate_root_map(&input, &output, &path)
        .unwrap();
    let value = root
        .apply(qualify_hot(EvmFailure { source: 7 }).unwrap())
        .unwrap();
    assert_eq!(take::<PortfolioFailure>(value).unwrap().collection, 107);
    assert!(matches!(
        root.apply(qualify_hot(EvmFailure { source: u64::MAX }).unwrap()),
        Err(RuntimeError::Internal)
    ));
    assert!(matches!(
        root.apply(qualify_hot(PortfolioFailure { collection: 7 }).unwrap()),
        Err(RuntimeError::Internal)
    ));
    assert!(matches!(
        assembly.inner.associate_root_map(&input, &output, &[]),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert!(matches!(
        assembly
            .inner
            .associate_root_map(&input, &output, &[path[1].clone(), path[0].clone()]),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert!(matches!(
        assembly.inner.associate_root_map(&input, &input, &path),
        Err(RuntimeError::IncompatibleAssembly)
    ));

    let empty = RuntimeAssemblyBuilder::new().unwrap().finish();
    assert!(matches!(
        empty.inner.associate_root_map(&input, &output, &path),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert!(matches!(
        empty.inner.associate_root_map(&input, &input, &[]),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    let identity = assembly
        .inner
        .associate_root_map(&input, &input, &[])
        .unwrap();
    assert_eq!(
        take::<EvmFailure>(
            identity
                .apply(qualify_hot(EvmFailure { source: 7 }).unwrap())
                .unwrap()
        )
        .unwrap()
        .source,
        7
    );
}

struct Observation;
impl mfm_capabilities::ReadCapabilityContract for Observation {
    type Intent = Offset;
    type Evidence = Offset;
    type OperationalError = ProviderError;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.recovery-observation@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(_: &ContentRef, _: &Offset, _: &Offset) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}
struct EvmRead;
impl mfm_program::State for EvmRead {
    type Input = Offset;
    type Output = Offset;
    type Failure = EvmFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-read@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl mfm_program::ReadState<Observation> for EvmRead {
    type AdapterContext = EvmContext;
    fn prepare(input: &Offset) -> std::result::Result<Offset, mfm_program::PreparationError> {
        Ok(Offset { value: input.value })
    }
    fn interpret(
        _: Offset,
        evidence: &Offset,
    ) -> std::result::Result<
        mfm_program::ProposedStateOutcome<Offset, EvmFailure>,
        StateExecutionError,
    > {
        Ok(mfm_program::ProposedStateOutcome::Failure {
            failure: EvmFailure {
                source: evidence.value,
            },
        })
    }
    fn adapter_context(
        input: &Offset,
        _: &Offset,
        _: &ProviderError,
    ) -> std::result::Result<EvmContext, StateExecutionError> {
        Ok(EvmContext {
            source: input.value,
        })
    }
}
impl mfm_program::CapabilityInjection<EvmRead> for Observation {
    type Setup = Offset;
    type ExpandedInput = Offset;
    type ExpandedOutput = Offset;
    type ExpandedFailure = EvmFailure;
    type FailureMap = Identity<EvmFailure>;
    fn failure_map_params(_: &Offset) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &Offset) -> mfm_program::Result<ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, r)| r)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct MappedRead {
    mapped_offset: u64,
}
impl mfm_program::Operation for MappedRead {
    type Input = Offset;
    type Output = Offset;
    type Failure = PortfolioFailure;
    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut mfm_program::OperationExpansion<Offset, Offset, PortfolioFailure>,
    ) -> mfm_program::Result<()> {
        let mut classifiers = Classifiers::new();
        classifiers.bind::<ProviderError, MapFailure, MapContext, PortfolioClassifier>(
            Offset {
                value: self.mapped_offset,
            },
            Offset {
                value: self.mapped_offset,
            },
            Offset { value: 17 },
        )?;
        scope.classifiers(classifiers)?;
        scope.read::<EvmRead, Observation, MapFailure>(
            &Offset { value: 1 },
            Offset { value: 100 },
            mfm_program::Occurrence::new(),
            mfm_program::ConclusionBound::new(4096)?,
        )
    }
}

#[test]
fn mapped_parameters_change_program_identity_and_survive_cold_association() {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_read::<EvmRead, Observation>().unwrap();
    builder
        .register_adapter::<Observation, _, _>(Offset { value: 1 }, |_, intent| {
            Box::pin(async move {
                Ok(Offset {
                    value: intent.value,
                })
            })
        })
        .unwrap();
    builder
        .register_classifier::<ProviderError, MapFailure, MapContext, PortfolioClassifier>()
        .unwrap();
    builder
        .register_handler::<PortfolioIncident, Stop>()
        .unwrap();
    let assembly = builder.finish();
    let context = RecoveryContext::new(ExecutionPhase::Read, RecoveryAllowances::new(2, 0), 2, &[]);
    let mut previous = None;
    for (mapped_offset, expected) in [
        (10, Assessment::Recoverable),
        (11, Assessment::Nonrecoverable),
    ] {
        let program = mfm_program::expand_program(
            mfm_ids::EntryPointId::new("mfm.test/mapped-read@1").unwrap(),
            &MappedRead { mapped_offset },
            &Offset { value: 7 },
            mfm_program::ProgramLimits::new(2),
        )
        .unwrap();
        assert_ne!(previous.as_ref(), Some(program.content_ref()));
        previous = Some(program.content_ref().clone());
        let executable = assembly
            .associate(mfm_program::Program::decode_canonical(program.canonical_bytes()).unwrap())
            .unwrap();
        assert_eq!(
            executable.declarations[0]
                .recovery
                .request(
                    QualifiedIncident::Domain(qualify_hot(EvmFailure { source: 7 }).unwrap()),
                    &context,
                )
                .unwrap(),
            (expected, RecoveryRequest::Stop)
        );
        let root = executable.declarations[0]
            .root_map
            .apply(qualify_hot(EvmFailure { source: 7 }).unwrap())
            .unwrap();
        assert_eq!(take::<PortfolioFailure>(root).unwrap().collection, 107);
    }
}

// The integration suite exercises the remaining scripted append outcomes.
#[allow(dead_code)]
#[path = "../../../tests/support/scripted_store.rs"]
mod scripted_store;
use scripted_store::{AppendAction, ScriptedStore};
mod execution;
