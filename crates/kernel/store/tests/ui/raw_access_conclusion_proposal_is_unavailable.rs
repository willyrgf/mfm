use mfm_journal::{ImmutableObject, StateOutcome};
use mfm_store::{QualifiedHistoryPort, SelectedRun};

fn submit_raw(
    port: &QualifiedHistoryPort,
    selected: SelectedRun,
    outcome: StateOutcome,
    object: ImmutableObject,
) {
    let _ = port.prepare_selected_pure_conclusion(selected, outcome, object, None);
}

fn main() {}
