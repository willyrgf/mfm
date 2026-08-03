use mfm_program::structured::PolicyProceed;

fn duplicate<Input, Output, Failure>(proceed: PolicyProceed<Input, Output, Failure>) {
    let _duplicate = proceed.clone();
}

fn main() {}
