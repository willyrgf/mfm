use mfm_store::{HistoryReader, OpenedStructuredStore, QualifiedHistoryPort};

fn main() {
    let _ = HistoryReader::select;
    let _ = HistoryReader::prepare_selected_access;
    let _ = QualifiedHistoryPort::select_qualified;
    let _ = OpenedStructuredStore::append_admission;
    let _ = QualifiedHistoryPort::append_admission;
    let _ = QualifiedHistoryPort::prepare_access;
    let _ = QualifiedHistoryPort::prepare_conclusion;
}
