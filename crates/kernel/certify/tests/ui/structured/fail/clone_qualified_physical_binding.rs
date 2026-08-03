use mfm_certify::structured::{QualifiedPhysicalBinding, ReadPhysicalBindingKind};

fn clone_binding(binding: QualifiedPhysicalBinding<ReadPhysicalBindingKind>) {
    let _duplicate = binding.clone();
}

fn main() {}
