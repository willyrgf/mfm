use super::*;

pub(super) fn contract_lifecycle_runner_output_summary(
    stream: &[store::KernelEventEnvelope],
) -> Vec<String> {
    attempt_output_commit_summaries(stream, "mfm.evm.contract/")
}

pub(super) fn attempt_output_commit_summaries(
    stream: &[store::KernelEventEnvelope],
    state_kind_prefix: &str,
) -> Vec<String> {
    let state_kinds_by_node = state_kinds_by_node(stream);
    let mut summaries = Vec::new();
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == commit_key
        {
            end += 1;
        }
        if commit_key.as_str().starts_with("attempt-output:") {
            let state_kind = runner_output_node_id(&stream[index..end])
                .and_then(|node_id| state_kinds_by_node.get(&node_id))
                .map(String::as_str)
                .unwrap_or("unknown");
            if state_kind.starts_with(state_kind_prefix) {
                let payloads = stream[index..end]
                    .iter()
                    .map(|event| runner_payload_summary(event.payload()))
                    .collect::<Vec<_>>()
                    .join("+");
                summaries.push(format!(
                    "{}:{state_kind}:{payloads}",
                    commit_key_class(commit_key.as_str())
                ));
            }
        }
        index = end;
    }
    summaries
}

pub(super) fn state_kinds_by_node(
    stream: &[store::KernelEventEnvelope],
) -> BTreeMap<String, String> {
    stream
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => Some((
                payload.node_id.as_str().to_owned(),
                payload
                    .state_kind
                    .canonical_name()
                    .unwrap_or_else(|| payload.state_kind.as_str())
                    .to_owned(),
            )),
            _ => None,
        })
        .collect()
}

macro_rules! node_id_from_payload {
    ($payload:expr, $($variant:ident),+ $(,)?) => {
        match $payload {
            $(
                events::KernelEventPayload::$variant(payload) => {
                    Some(payload.node_id.as_str().to_owned())
                }
            )+
            _ => None,
        }
    };
}

pub(super) fn runner_output_node_id(events: &[store::KernelEventEnvelope]) -> Option<String> {
    events.iter().find_map(|event| {
        node_id_from_payload!(
            event.payload(),
            FactRecorded,
            CellProduced,
            CellSkipped,
            SideEffectIntentPersisted,
            SideEffectClaimed,
            SideEffectClaimTakenOver,
            SideEffectInvocationPrepared,
            SideEffectInvocationStarted,
            SideEffectNotSubmittedProven,
            SideEffectSubmissionObserved,
            SideEffectSubmissionUnknown,
            SideEffectReceiptObserved,
            SideEffectConfirmationObserved,
            SideEffectAmbiguous,
            SideEffectFailed,
            StateAttemptCompleted,
        )
    })
}

pub(super) fn runner_payload_summary(payload: &events::KernelEventPayload) -> String {
    match payload {
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            format!(
                "artifact_referenced[role={}]",
                payload.artifact_ref.role.as_str()
            )
        }
        events::KernelEventPayload::RetentionRefsAppended(payload) => {
            let roles = payload
                .refs
                .iter()
                .map(|retention| retention.role.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("retention_refs_appended[roles={roles}]")
        }
        _ => payload_schema_name(payload).to_owned(),
    }
}

pub(super) fn payload_schema_name(payload: &events::KernelEventPayload) -> &str {
    payload
        .schema_descriptor()
        .schema_name
        .strip_prefix("mfm.events.v1.")
        .unwrap_or_else(|| payload.schema_descriptor().schema_name)
}

pub(super) fn commit_key_class(commit_key: &str) -> &str {
    if commit_key.starts_with("attempt-output:") {
        "attempt-output"
    } else {
        commit_key
    }
}
