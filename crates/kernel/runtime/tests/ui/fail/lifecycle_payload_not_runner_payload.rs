use mfm_events::v1 as events;
use mfm_runtime::RunnerEventPayload;

fn run_admitted() -> events::RunAdmitted {
    unimplemented!()
}

fn attempt_started() -> events::StateAttemptStarted {
    unimplemented!()
}

fn completed() -> events::StateAttemptCompleted {
    unimplemented!()
}

fn failed() -> events::StateAttemptFailed {
    unimplemented!()
}

fn run_completed() -> events::RunCompleted {
    unimplemented!()
}

fn retention_refs() -> events::RetentionRefsAppended {
    unimplemented!()
}

fn retention_manifest() -> events::RetentionManifestProjected {
    unimplemented!()
}

fn main() {
    let _ = RunnerEventPayload::RunAdmitted(run_admitted());
    let _ = RunnerEventPayload::StateAttemptStarted(attempt_started());
    let _ = RunnerEventPayload::StateAttemptCompleted(completed());
    let _ = RunnerEventPayload::StateAttemptFailed(failed());
    let _ = RunnerEventPayload::RunCompleted(run_completed());
    let _ = RunnerEventPayload::RetentionRefsAppended(retention_refs());
    let _ = RunnerEventPayload::RetentionManifestProjected(retention_manifest());
}
