fn main() {
    let output = mfm_runtime::ErasedRunnerOutput::new(Vec::new());
    let _ = output.staged_artifacts;
    let _ = output.staged_retention_refs;
    let _ = output.payloads;
}
