use mfm_store::{OpenedStructuredStore, QualifiedHistoryPort};

fn clone_opened(store: OpenedStructuredStore) {
    let _ = store.clone();
}

fn clone_history(port: QualifiedHistoryPort) {
    let _ = port.clone();
}

fn main() {}
