use mfm_program::{Checkpoint, Never, OperationExpansion};

fn require_clone<T: Clone>() {}
fn require_default<T: Default>() {}

fn main() {
    let _expansion = OperationExpansion::<Never, Never, Never> {};
    let _checkpoint = Checkpoint::<Never> {};
    let _ = OperationExpansion::<Never, Never, Never>::new;
    require_clone::<OperationExpansion<Never, Never, Never>>();
    require_default::<OperationExpansion<Never, Never, Never>>();
    require_default::<Checkpoint<Never>>();
}
