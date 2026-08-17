use mfm_program::{InjectionWriter, MatchJoin, Never};

fn require_clone<T: Clone>() {}
fn require_default<T: Default>() {}

fn inspect_writer(writer: &mut InjectionWriter) {
    writer.read();
    writer.operation();
    writer.match_join();
    writer.with_failure_handler();
    writer.finish();
}

fn inspect_join(join: &mut MatchJoin<Never, Never, Never>) {
    join.select();
    join.place();
    join.branch_to();
    join.return_success();
    join.finish();
}

fn main() {
    let _writer = InjectionWriter {};
    let _join = MatchJoin::<Never, Never, Never> {};
    require_clone::<InjectionWriter>();
    require_default::<InjectionWriter>();
    require_clone::<MatchJoin<Never, Never, Never>>();
    require_default::<MatchJoin<Never, Never, Never>>();
}
