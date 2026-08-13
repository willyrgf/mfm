use mfm_store::SelectedRun;

fn consume(selected: SelectedRun) {
    let _evidence = selected.into_qualified_run();
    let _stale = selected.action();
}

fn clone_selected(selected: SelectedRun) {
    let _ = selected.clone();
}

fn split_evidence(selected: SelectedRun) {
    let _evidence = selected.qualified_run().clone();
    let _authority = selected.action();
}

fn main() {}
