use super::*;

pub(super) fn referenced_artifact_ids(request: &CommitRequest) -> BTreeSet<ArtifactId> {
    let mut artifact_ids = BTreeSet::new();
    for payload in &request.payloads {
        for requirement in payload.artifact_requirements() {
            artifact_ids.insert(requirement.artifact_id);
        }
    }
    artifact_ids
}
