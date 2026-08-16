use mfm_runtime::{RunView, Runtime};

fn extract_store(runtime: Runtime) {
    let Runtime { store, .. } = runtime;
    let _ = store;
}

fn extract_state(view: RunView) {
    let RunView { state, .. } = view;
    let _ = state;
}

fn main() {}
