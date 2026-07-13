use super::*;

#[path = "artifact_contract.rs"]
mod contract;
#[path = "artifact_refs.rs"]
mod refs;

pub use self::contract::*;
pub use self::refs::*;

pub(super) fn event_artifact_requirements(
    payload: &KernelEventPayload,
) -> Vec<refs::EventArtifactRequirement> {
    refs::event_artifact_requirements(payload)
}
