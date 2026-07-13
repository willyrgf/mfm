pub(crate) fn parse_success_json(stdout: &[u8]) -> serde_json::Value {
    let parsed: serde_json::Value =
        serde_json::from_slice(stdout).expect("stdout must be valid json");
    assert_eq!(parsed["status"], "success");
    parsed["data"].clone()
}
