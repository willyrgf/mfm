use mfm_events::v1 as events;
use mfm_runtime::RunnerEventPayload;

fn artifact_reference() -> events::ArtifactReferenced {
    unimplemented!()
}

fn main() {
    let _ = RunnerEventPayload::ArtifactReferenced(artifact_reference());
}
