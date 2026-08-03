use mfm_certify::structured::{QualifiedPhysicalBinding, ReadPhysicalBindingKind};

fn split(binding: QualifiedPhysicalBinding<ReadPhysicalBindingKind>) {
    let QualifiedPhysicalBinding { core, .. } = binding;
    drop(core);
}

fn main() {}
