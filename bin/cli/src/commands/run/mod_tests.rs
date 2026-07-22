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
            source.contains("connect_application"),
            "{} must use the opaque application facade",
            path.display()
        );
        for forbidden in [
            "start_entry_point_run",
            "resume_run",
            "record_manual_resolution",
            "runtime_config",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must remain evidence-only: {forbidden}",
                path.display()
            );
        }
    }
}
