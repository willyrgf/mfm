//! Proof transport scenario descriptors.

use crate::{ExpectedJsonValue, GoldenLabel, PublicJsonFragment, ScenarioDescriptor};

/// Proof transport conformance scenario descriptor.
pub const SCENARIO: ScenarioDescriptor = ScenarioDescriptor {
    label: "proof_transport.conformance.deterministic",
    purpose: "deterministic proof transport execution and replay conformance",
    public_summary_media_type: "application/json",
    public_json_fragments: &PUBLIC_JSON_FRAGMENTS,
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

/// Golden labels for proof transport conformance data.
pub const GOLDEN_LABELS: [GoldenLabel; 0] = [];
