use mfm_ids::{ContentRef, StableId};
use mfm_program::{
    CapabilityInjection, Identity, Never, NoParams, Operation, OperationExpansion,
    ProgramError, State,
};

fn require_static_expansion(
    _expansion: &'static mut OperationExpansion<Never, Never, Never>,
) {
}

struct EscapingExpansion;

impl Operation for EscapingExpansion {
    type Input = Never;
    type Output = Never;
    type Failure = Never;

    fn validate_input(&self, _input: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        expansion: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        require_static_expansion(expansion);
        Ok(())
    }
}

struct HookState;

impl State for HookState {
    type Input = Never;
    type Output = Never;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.scoped-escape/state@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

fn require_static_hook(_writer: &'static mut OperationExpansion<Never, Never, Never>) {}

struct EscapingCapability;

impl CapabilityInjection<HookState> for EscapingCapability {
    type Setup = ();
    type ExpandedInput = Never;
    type ExpandedOutput = Never;
    type ExpandedFailure = <HookState as mfm_program::State>::Failure;

    type FailureMap = Identity<Never>;

    fn failure_map_params(_setup: &Self::Setup) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }

    fn original_binding_ref(_setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        Err(ProgramError::InvalidContract)
    }

    fn write_before(
        _setup: &Self::Setup,
        writer: &mut OperationExpansion<Never, Never, Never>,
    ) -> mfm_program::Result<()> {
        require_static_hook(writer);
        Ok(())
    }
}

fn main() {}
