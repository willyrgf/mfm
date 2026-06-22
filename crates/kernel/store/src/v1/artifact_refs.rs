use super::*;

/// Returns artifact evidence requirements referenced by one kernel event payload.
pub fn event_artifact_requirements(payload: &KernelEventPayload) -> Vec<EventArtifactRequirement> {
    payload.artifact_requirements()
}

pub(super) fn referenced_artifact_ids(request: &CommitRequest) -> BTreeSet<ArtifactId> {
    let mut artifact_ids = BTreeSet::new();
    for payload in &request.payloads {
        for requirement in event_artifact_requirements(payload) {
            artifact_ids.insert(requirement.artifact_id);
        }
    }
    artifact_ids
}
