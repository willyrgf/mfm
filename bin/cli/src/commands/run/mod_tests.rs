#[test]
fn read_only_run_commands_use_evidence_only_services() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let run_root = manifest_dir.join("src/commands/run");
    let read_only_commands = [
        "list.rs",
        "status.rs",
        "stream.rs",
        "public_output.rs",
        "replay.rs",
    ];

    for file in read_only_commands {
        let path = run_root.join(file);
        let source = std::fs::read_to_string(&path).expect("read command source");
        assert!(
            source.contains("connect_run_read_services"),
            "{} must construct evidence-only run services",
            path.display()
        );
        assert!(
            !source.contains("connect_run_services"),
            "{} must not construct live run services",
            path.display()
        );
    }
}
