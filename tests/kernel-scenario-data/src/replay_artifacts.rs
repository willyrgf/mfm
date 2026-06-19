//! Replay artifact negative-case descriptors.

use crate::{
    CorruptionCaseDescriptor, CorruptionCaseFamily, ExpectedJsonValue, GoldenLabel,
    PublicJsonFragment, ScenarioDescriptor,
};

/// Replay artifact negative-case scenario descriptor.
pub const NEGATIVE_CASE_SCENARIO: ScenarioDescriptor = ScenarioDescriptor {
    label: "replay.artifacts.negative_cases",
    purpose: "replay artifact evidence and live-capability negative cases",
    public_summary_media_type: "application/json",
    public_json_fragments: &PUBLIC_JSON_FRAGMENTS,
    corruption_cases: &CORRUPTION_CASES,
    golden_labels: &GOLDEN_LABELS,
};

/// Public JSON fragments expected from replay artifact fixture summaries.
pub const PUBLIC_JSON_FRAGMENTS: [PublicJsonFragment; 3] = [
    PublicJsonFragment {
        pointer: "/run_mode",
        expected: ExpectedJsonValue::String("completed"),
    },
    PublicJsonFragment {
        pointer: "/replay_live_cap_requests_count",
        expected: ExpectedJsonValue::Unsigned(0),
    },
    PublicJsonFragment {
        pointer: "/status",
        expected: ExpectedJsonValue::String("success"),
    },
];

/// Replay artifact evidence corruption cases.
pub const CORRUPTION_CASES: [CorruptionCaseDescriptor; 6] = [
    CorruptionCaseDescriptor {
        name: "wrong_role",
        family: CorruptionCaseFamily::ReplayArtifactEvidence,
        mutation: "replace_artifact_role",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: Some("state_output"),
    },
    CorruptionCaseDescriptor {
        name: "wrong_producer",
        family: CorruptionCaseFamily::ReplayArtifactEvidence,
        mutation: "replace_artifact_producer",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: Some("state_output"),
    },
    CorruptionCaseDescriptor {
        name: "missing_committed_artifact_reference",
        family: CorruptionCaseFamily::ReplayArtifactEvidence,
        mutation: "remove_committed_artifact_reference",
        expected_error_kind: "artifact_missing",
        artifact_role_tag: Some("state_output"),
    },
    CorruptionCaseDescriptor {
        name: "tampered_spec",
        family: CorruptionCaseFamily::ReplayArtifactEvidence,
        mutation: "tamper_typed_execution_spec",
        expected_error_kind: "artifact_missing",
        artifact_role_tag: Some("typed_execution_spec"),
    },
    CorruptionCaseDescriptor {
        name: "tampered_certificate",
        family: CorruptionCaseFamily::ReplayArtifactEvidence,
        mutation: "tamper_typed_spec_certificate",
        expected_error_kind: "artifact_missing",
        artifact_role_tag: Some("typed_spec_certificate"),
    },
    CorruptionCaseDescriptor {
        name: "replay_with_live_capability",
        family: CorruptionCaseFamily::ReplayArtifactEvidence,
        mutation: "request_live_capability_from_replay",
        expected_error_kind: "live_capability_request",
        artifact_role_tag: None,
    },
];

/// Golden labels for replay artifact negative-case data.
pub const GOLDEN_LABELS: [GoldenLabel; 2] = [
    GoldenLabel {
        label: "replay_artifact_negative_case_matrix",
        surface: "tests/integration/src/test_support.rs",
    },
    GoldenLabel {
        label: "typed_certified_slice_public_replay_summary",
        surface: "tests/integration/src/test_support.rs",
    },
];
