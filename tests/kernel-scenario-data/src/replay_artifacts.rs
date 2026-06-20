//! Replay artifact scenario descriptors.

use crate::{ExpectedJsonValue, GoldenLabel, PublicJsonFragment, ScenarioDescriptor};

/// Replay artifact scenario descriptor.
pub const NEGATIVE_CASE_SCENARIO: ScenarioDescriptor = ScenarioDescriptor {
    label: "replay.artifacts",
    purpose: "replay artifact evidence public replay summary",
    public_summary_media_type: "application/json",
    public_json_fragments: &PUBLIC_JSON_FRAGMENTS,
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

/// Golden labels for replay artifact data.
pub const GOLDEN_LABELS: [GoldenLabel; 1] = [GoldenLabel {
    label: "typed_certified_slice_public_replay_summary",
    surface: "tests/integration/src/test_support.rs",
}];
