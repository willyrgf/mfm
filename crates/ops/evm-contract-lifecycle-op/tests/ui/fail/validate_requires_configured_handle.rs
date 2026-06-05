use mfm_evm_contract_model::DeployedContract;
use mfm_op_evm_contract_lifecycle::ValidateContractOperation;
use mfm_program::{Handle, Operation};

fn main() {}

fn wrong_typestate<'program, 'scope>(
    deployed: Handle<'program, 'scope, DeployedContract>,
) -> <ValidateContractOperation as Operation>::Input<'program, 'scope> {
    deployed
}
