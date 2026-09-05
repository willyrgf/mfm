use mfm_program::{MatchJoin, Never, OperationExpansion};

fn require_clone<T: Clone>() {}
fn require_default<T: Default>() {}

fn inspect_expansion(expansion: &mut OperationExpansion<Never, Never, Never>) {
    expansion.label();
    expansion.place();
    expansion.select();
    expansion.branch_to();
    expansion.return_success();
    expansion.finish();
}

fn inspect_join(join: &mut MatchJoin<Never, Never, Never>) {
    join.select();
    join.place();
    join.label();
    join.branch_to();
    join.return_success();
    join.finish();
    join.effect();
}

fn main() {
    let _expansion = OperationExpansion::<Never, Never, Never> {};
    let _join = MatchJoin::<Never, Never, Never> {};
    let _ = OperationExpansion::<Never, Never, Never>::new;
    let _ = MatchJoin::<Never, Never, Never>::new;
    require_clone::<OperationExpansion<Never, Never, Never>>();
    require_default::<OperationExpansion<Never, Never, Never>>();
    require_clone::<MatchJoin<Never, Never, Never>>();
    require_default::<MatchJoin<Never, Never, Never>>();
}
