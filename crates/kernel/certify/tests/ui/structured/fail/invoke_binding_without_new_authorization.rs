use mfm_certify::structured::{
    QualifiedPhysicalBinding, ReadPhysicalBindingKind, RuntimeProcessRegistry,
};

fn bypass(
    processes: &RuntimeProcessRegistry,
    binding: QualifiedPhysicalBinding<ReadPhysicalBindingKind>,
) {
    let _completion = processes.invoke_qualified_physical_binding(binding);
}

fn main() {}
