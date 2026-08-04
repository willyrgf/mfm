use mfm_certify::structured::{
    QualifiedPhysicalBinding, ReadPhysicalBindingKind, CertifiedProcessRegistry,
};

fn bypass(
    processes: &CertifiedProcessRegistry,
    binding: QualifiedPhysicalBinding<ReadPhysicalBindingKind>,
) {
    let _completion = processes.invoke_authorized_physical_binding(binding);
}

fn main() {}
