use super::*;
use mfm_ids::StableId;
use mfm_program::{
    Classification, ExecutionPhase, HandlerBinding, Identity, NoContext, NoParams, ProgramError,
    RecoveryAllowances, StateExecutionError,
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

impl ClassifyError for EvmFailure {
    fn classify(&self) -> Classification {
        Classification::Retryable
    }
}
impl ClassifyError for ProviderError {
    fn classify(&self) -> Classification {
        match self {
            Self::Unavailable => Classification::Retryable,
        }
    }
}

struct RetryRead;
impl Handler for RetryRead {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("test.recovery.retry-read@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        incident: &IncidentSummary,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(
            if incident.classification == Classification::Retryable
                && context.phase() == ExecutionPhase::Read
            {
                RecoveryRequest::RetryState
            } else {
                RecoveryRequest::Stop
            },
        )
    }
}

struct ConfiguredHandler;
impl Handler for ConfiguredHandler {
    type Params = Offset;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("test.recovery.configured@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        params: &Offset,
        _: &IncidentSummary,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(if params.value == 10 {
            RecoveryRequest::RetryState
        } else {
            RecoveryRequest::Stop
        })
    }
}

#[test]
fn handlers_associate_exact_parameters_without_incident_dispatch() {
    let binding = HandlerBinding::new::<ConfiguredHandler>(Offset { value: 10 }).unwrap();
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    assert!(matches!(
        builder
            .finish()
            .inner
            .associate_recovery(classify::<EvmFailure, ProviderError>, &binding),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_handler::<ConfiguredHandler>().unwrap();
    builder.register_handler::<ConfiguredHandler>().unwrap();
    let assembly = builder.finish();
    let policy = assembly
        .inner
        .associate_recovery(classify::<EvmFailure, ProviderError>, &binding)
        .unwrap();
    let context = RecoveryContext::new(
        ExecutionPhase::Read,
        RecoveryAllowances::new(2, 1),
        3,
        &[],
        &[],
    );
    for incident in [
        QualifiedIncident::Domain(&qualify_hot(EvmFailure { source: 7 }).unwrap()),
        QualifiedIncident::Adapter {
            original: &qualify_hot(ProviderError::Unavailable).unwrap(),
        },
    ] {
        assert_eq!(
            policy.request(incident, &context).unwrap(),
            RecoveryRequest::RetryState
        );
    }
    let mut wire = serde_json::to_value(&binding).unwrap();
    wire["params"] =
        serde_json::to_value(mfm_program::PolicyParams::new(&NoParams).unwrap()).unwrap();
    let mismatched: HandlerBinding = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        assembly
            .inner
            .associate_recovery(classify::<EvmFailure, ProviderError>, &mismatched),
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
        scope.handler(HandlerBinding::new::<ConfiguredHandler>(Offset {
            value: self.mapped_offset,
        })?)?;
        scope.read::<EvmRead, Observation, MapFailure>(
            &Offset { value: 1 },
            Offset { value: 100 },
            mfm_program::Occurrence::new(),
            mfm_program::ConclusionBound::new(4096)?,
        )
    }
}

#[test]
fn handler_parameters_change_program_identity_and_survive_cold_association() {
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
    builder.register_map::<MapFailure>().unwrap();
    builder.register_handler::<ConfiguredHandler>().unwrap();
    let assembly = builder.finish();
    let context = RecoveryContext::new(
        ExecutionPhase::Read,
        RecoveryAllowances::new(2, 0),
        2,
        &[],
        &[],
    );
    let mut previous = None;
    for (mapped_offset, expected) in [
        (10, RecoveryRequest::RetryState),
        (11, RecoveryRequest::Stop),
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
                    QualifiedIncident::Domain(&qualify_hot(EvmFailure { source: 7 }).unwrap()),
                    &context,
                )
                .unwrap(),
            expected
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
