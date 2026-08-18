use mfm_program::{InjectionWriter, MatchJoin, Never, OperationExpansion};

fn require_clone<T: Clone>() {}
fn require_default<T: Default>() {}

fn inspect_expansion(expansion: &mut OperationExpansion<Never, Never, Never>) {
    expansion.label();
    expansion.place();
    expansion.select();
    expansion.branch_to();
    expansion.return_success();
    expansion.finish();
    expansion.effect();
}

fn inspect_writer(writer: &mut InjectionWriter) {
    writer.read();
    writer.operation();
    writer.match_join();
    writer.with_failure_handler();
    writer.label();
    writer.place();
    writer.select();
    writer.branch_to();
    writer.return_success();
    writer.finish();
    writer.effect();
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
    let _writer = InjectionWriter {};
    let _join = MatchJoin::<Never, Never, Never> {};
    let _ = OperationExpansion::<Never, Never, Never>::new;
    let _ = InjectionWriter::new;
    let _ = MatchJoin::<Never, Never, Never>::new;
    require_clone::<OperationExpansion<Never, Never, Never>>();
    require_default::<OperationExpansion<Never, Never, Never>>();
    require_clone::<InjectionWriter>();
    require_default::<InjectionWriter>();
    require_clone::<MatchJoin<Never, Never, Never>>();
    require_default::<MatchJoin<Never, Never, Never>>();
}
