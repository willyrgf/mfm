use mfm_evm_contract_model::ConfiguredContractInstance;
use mfm_program::Handle;
use mfm_state_evm_contracts::ContextConfigureContractInputHandles;

fn main() {}

fn wrong_typestate<'program, 'scope>(
    configured: Handle<'program, 'scope, ConfiguredContractInstance>,
) -> ContextConfigureContractInputHandles<'program, 'scope> {
    ContextConfigureContractInputHandles {
        deployed: configured,
    }
}
