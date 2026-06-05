use mfm_evm_contract_model::ValidationReport;
use mfm_op_evm_contract_lifecycle::ConfigureContractOperation;
use mfm_program::{Handle, Operation};

fn main() {}

fn wrong_typestate<'program, 'scope>(
    report: Handle<'program, 'scope, ValidationReport>,
) -> <ConfigureContractOperation as Operation>::Input<'program, 'scope> {
    report
}
