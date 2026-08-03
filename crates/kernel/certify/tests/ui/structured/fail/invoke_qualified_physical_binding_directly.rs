use mfm_certify::structured::{QualifiedPhysicalBinding, ReadPhysicalBindingKind};

fn invoke(binding: QualifiedPhysicalBinding<ReadPhysicalBindingKind>) {
    let _completion = binding.invoke();
}

fn main() {}
