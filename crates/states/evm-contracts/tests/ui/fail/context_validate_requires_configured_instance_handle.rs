use mfm_evm_contract_model::DeployedContractInstance;
use mfm_program::Handle;
use mfm_state_evm_contracts::ContextValidateContractInputHandles;

fn main() {}

fn wrong_typestate<'program, 'scope>(
    deployed: Handle<'program, 'scope, DeployedContractInstance>,
) -> ContextValidateContractInputHandles<'program, 'scope> {
    ContextValidateContractInputHandles {
        configured: deployed,
    }
}
