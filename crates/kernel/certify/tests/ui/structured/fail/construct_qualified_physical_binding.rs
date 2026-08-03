use std::marker::PhantomData;

use mfm_certify::structured::{QualifiedPhysicalBinding, ReadPhysicalBindingKind};

fn main() {
    let _binding: QualifiedPhysicalBinding<ReadPhysicalBindingKind> = QualifiedPhysicalBinding {
        core: todo!(),
        _kind: PhantomData,
    };
}
