//! Proof transport scenario descriptors.

use crate::{
    CorruptionCaseDescriptor, CorruptionCaseFamily, ExpectedJsonValue, GoldenLabel,
    PublicJsonFragment, ScenarioDescriptor,
};

/// Proof transport conformance scenario descriptor.
pub const SCENARIO: ScenarioDescriptor = ScenarioDescriptor {
    label: "proof_transport.conformance.deterministic",
    purpose: "deterministic proof transport execution and replay conformance",
    public_summary_media_type: "application/json",
    public_json_fragments: &PUBLIC_JSON_FRAGMENTS,
    corruption_cases: &CORRUPTION_CASES,
    golden_labels: &GOLDEN_LABELS,
};

/// Public JSON fragments expected from the proof transport conformance summary.
pub const PUBLIC_JSON_FRAGMENTS: [PublicJsonFragment; 5] = [
    PublicJsonFragment {
        pointer: "/kind",
        expected: ExpectedJsonValue::String("proof-implementation-conformance"),
    },
    PublicJsonFragment {
        pointer: "/version",
        expected: ExpectedJsonValue::Unsigned(1),
    },
    PublicJsonFragment {
        pointer: "/payload/facts_valid",
        expected: ExpectedJsonValue::Bool(true),
    },
    PublicJsonFragment {
        pointer: "/payload/replay_valid",
        expected: ExpectedJsonValue::Bool(true),
    },
    PublicJsonFragment {
        pointer: "/payload/confirmation_valid",
        expected: ExpectedJsonValue::Bool(true),
    },
];

/// Replay stream corruption cases for proof transport conformance.
pub const CORRUPTION_CASES: [CorruptionCaseDescriptor; 7] = [
    CorruptionCaseDescriptor {
        name: "completed_history_without_retention_projection",
        family: CorruptionCaseFamily::ProofReplayStream,
        mutation: "remove_retention_projection_after_completion",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: Some("retention_manifest"),
    },
    CorruptionCaseDescriptor {
        name: "standalone_retention_projection_history",
        family: CorruptionCaseFamily::ProofReplayStream,
        mutation: "retain_only_projection_payload",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: Some("retention_manifest"),
    },
    CorruptionCaseDescriptor {
        name: "failed_completion_without_framework_authority",
        family: CorruptionCaseFamily::ProofReplayStream,
        mutation: "replace_completion_authority_with_failed_without_claim",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: None,
    },
    CorruptionCaseDescriptor {
        name: "post_completion_retention_refs",
        family: CorruptionCaseFamily::ProofReplayStream,
        mutation: "append_retention_refs_after_completion_projection",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: Some("state_output"),
    },
    CorruptionCaseDescriptor {
        name: "post_projection_retention_refs_before_completion",
        family: CorruptionCaseFamily::ProofReplayStream,
        mutation: "insert_retention_refs_after_projection_before_completion",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: Some("state_output"),
    },
    CorruptionCaseDescriptor {
        name: "extra_payload_in_retention_projection_commit",
        family: CorruptionCaseFamily::ProofReplayStream,
        mutation: "add_sidecar_payload_to_retention_projection_commit",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: Some("retention_manifest"),
    },
    CorruptionCaseDescriptor {
        name: "same_sequence_sidecar_retention_projection_payload",
        family: CorruptionCaseFamily::ProofReplayStream,
        mutation: "add_same_sequence_sidecar_retention_projection",
        expected_error_kind: "invalid_run_stream",
        artifact_role_tag: Some("retention_manifest"),
    },
];

/// Golden labels for proof transport conformance data.
pub const GOLDEN_LABELS: [GoldenLabel; 2] = [
    GoldenLabel {
        label: "proof_transport_conformance_summary",
        surface: "tests/integration/tests/proof_transport_conformance.rs",
    },
    GoldenLabel {
        label: "proof_transport_replay_corruption_matrix",
        surface: "tests/integration/tests/proof_transport_conformance.rs",
    },
];
