use mfm_capabilities::ReadCapabilityContract;
use mfm_ids::{ContentRef, StableId};
use mfm_program::*;
use mfm_runtime::RuntimeAssemblyBuilder;

struct Observation;
impl ReadCapabilityContract for Observation {
    type Intent = NoParams;
    type Evidence = NoParams;
    type OperationalError = NoParams;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.unclassified@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(_: &ContentRef, _: &NoParams, _: &NoParams) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}
struct Read;
impl State for Read {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.unclassified-read@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl ReadState<Observation> for Read {
    type AdapterContext = NoContext;
    fn adapter_context(
        _: &NoParams,
        _: &NoParams,
        _: &NoParams,
    ) -> std::result::Result<NoContext, StateExecutionError> {
        Ok(NoContext)
    }
    fn prepare(_: &NoParams) -> std::result::Result<NoParams, PreparationError> {
        Ok(NoParams)
    }
    fn interpret(
        input: NoParams,
        _: &NoParams,
    ) -> std::result::Result<ProposedStateOutcome<NoParams, Never>, StateExecutionError> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl CapabilityInjection<Read> for Observation {
    type Setup = NoParams;
    type ExpandedInput = NoParams;
    type ExpandedOutput = NoParams;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &NoParams) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(_: &NoParams) -> mfm_program::Result<ContentRef> {
        nominal_contract_ref::<NoParams>()
    }
}
fn capability_remains_reusable<C: ReadCapabilityContract>() {}
fn author(scope: &mut OperationExpansion<NoParams, NoParams, Never>) {
    scope
        .read::<Read, Observation, Identity<Never>>(
            &NoParams,
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(1024).unwrap(),
        )
        .unwrap();
}
fn main() {
    capability_remains_reusable::<Observation>();
    RuntimeAssemblyBuilder::new()
        .unwrap()
        .register_read::<Read, Observation>()
        .unwrap();
}
