fn main() {
    let value = serde_json::json!({ "amount": 1 });
    let _ = <serde_json::Value as mfm_program::IntoStateInput<
        'static,
        'static,
        serde_json::Value,
    >>::into_binding(value);
}
