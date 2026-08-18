use mfm_ids::{ContentRef, StableId};
use mfm_program::{
    CapabilityInjection, InjectionWriter, MatchJoin, Never, Operation, OperationExpansion,
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

    fn expand(
        &self,
        expansion: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        require_static_expansion(expansion);
        Ok(())
    }
}

fn require_static_join(_join: &'static mut MatchJoin<Never, Never, Never>) {}

struct EscapingJoin;

impl Operation for EscapingJoin {
    type Input = Never;
    type Output = Never;
    type Failure = Never;

    fn expand(
        &self,
        expansion: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        expansion.match_join::<Never, Never>(|join| {
            require_static_join(join);
            Ok(())
        })
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

fn require_static_writer(_writer: &'static mut InjectionWriter) {}

struct EscapingCapability;

impl CapabilityInjection<HookState> for EscapingCapability {
    type Setup = ();
    type ExpandedInput = Never;
    type ExpandedOutput = Never;

    fn original_binding_ref(_setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        Err(ProgramError::InvalidContract)
    }

    fn write_before(
        _setup: &Self::Setup,
        writer: &mut InjectionWriter,
    ) -> mfm_program::Result<()> {
        require_static_writer(writer);
        Ok(())
    }
}

fn main() {}
