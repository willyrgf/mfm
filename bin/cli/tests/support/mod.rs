use assert_cmd::Command;

pub(crate) fn parse_success_json(stdout: &[u8]) -> serde_json::Value {
    let parsed: serde_json::Value =
        serde_json::from_slice(stdout).expect("stdout must be valid json");
    assert_eq!(parsed["status"], "success");
    parsed["data"].clone()
}

pub(crate) fn sanitize_machine_readable_cli_env(cmd: &mut Command) -> &mut Command {
    // Keep JSON response channels deterministic for parity tests even when the parent
    // environment enables tracing (e.g. LOG_LEVEL/RUST_LOG in CI debug runs).
    cmd.env_remove("LOG_LEVEL")
        .env_remove("RUST_LOG")
        .env_remove("LOG_FORMAT")
        .env_remove("LOG_SPAN_EVENTS")
}
