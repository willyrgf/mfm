use mfm_store::SelectedRun;

fn main() {
    let _ = SelectedRun {};
    let _: SelectedRun = serde_json::from_str("{}").unwrap();
}
