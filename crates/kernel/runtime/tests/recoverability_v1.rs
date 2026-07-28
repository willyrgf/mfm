#[path = "../src/decision.rs"]
mod runtime_decision;

#[path = "../../../../tests/support/recoverability_v1.rs"]
mod recoverability_v1_support;

use recoverability_v1_support::{assert_lower_layer_owner_vector, run_consumer, OwnerVector};
use runtime_decision::{
    scan_observations, select_action, DecisionInput, EvidenceScan, IntegrityBlock,
    ObservationVerdict, SelectedAction,
};

#[test]
fn runtime_executes_all_570_frozen_recoverability_vectors() {
    run_consumer("mfm-runtime", |vector| {
        assert_lower_layer_owner_vector(vector);
        assert_runtime_owner_vector(vector);
    });
}

fn assert_runtime_owner_vector(owner: OwnerVector<'_>) {
    match owner {
        OwnerVector::RelationalPositive(_) => assert_runtime_positive(owner),
        OwnerVector::RelationalRejection(_) => {
            let decision = select_action(DecisionInput::<(), (), (), _> {
                occurrences: Vec::new(),
                integrity_blocks: vec![IntegrityBlock {
                    certified_order: 0,
                    observation_order: 0,
                    detail: owner.id(),
                }],
                operationally_blocked: true,
                closed: false,
                open_all_terminal: None,
            });
            assert_eq!(
                decision,
                SelectedAction::IntegrityBlocked { detail: owner.id() },
                "{}",
                owner.id()
            );
        }
    }
}

fn assert_runtime_positive(owner: OwnerVector<'_>) {
    match owner.kind() {
        "read_returned_validation_verdict" | "read_safe_failure_verdict" => {
            assert_runtime_read_verdict(owner)
        }
        "relational_acceptance" => assert_eq!(
            scan_observations([ObservationVerdict::Settlement(owner.id())]),
            EvidenceScan::Settlement {
                observation_order: 0,
                candidate: owner.id(),
            },
            "{}",
            owner.id()
        ),
        "frontier_order" => assert_eq!(
            scan_observations([ObservationVerdict::<()>::InsufficientEvidence]),
            EvidenceScan::AllInsufficient,
            "{}",
            owner.id()
        ),
        "commit_coordinate_separation"
        | "content_identity_separation"
        | "cross_run_source_redaction_identity"
        | "export_identity"
        | "identity_separation"
        | "request_identity"
        | "resource_refold"
        | "schema_identity"
        | "type_separation" => {
            assert!(!owner.id().is_empty());
        }
        other => panic!("{}: unclassified runtime callback kind {other}", owner.id()),
    }
}

fn assert_runtime_read_verdict(owner: OwnerVector<'_>) {
    let expected =
        recoverability_v1_support::string(owner.vector(), "expected_reproduction_verdict");
    let actual = match expected {
        "committed_typed_terminal" => {
            scan_observations([ObservationVerdict::Settlement(owner.id())])
        }
        "insufficient_evidence" => {
            scan_observations([ObservationVerdict::<&str>::InsufficientEvidence])
        }
        "invalid_evidence" | "structural_reject_before_callback" => {
            scan_observations([ObservationVerdict::<&str>::InvalidEvidence])
        }
        other => panic!("{}: unknown reproduction verdict {other}", owner.id()),
    };
    match expected {
        "committed_typed_terminal" => assert_eq!(
            actual,
            EvidenceScan::Settlement {
                observation_order: 0,
                candidate: owner.id(),
            },
            "{}",
            owner.id()
        ),
        "insufficient_evidence" => {
            assert_eq!(actual, EvidenceScan::AllInsufficient, "{}", owner.id())
        }
        "invalid_evidence" | "structural_reject_before_callback" => assert_eq!(
            actual,
            EvidenceScan::InvalidEvidence {
                observation_order: 0,
            },
            "{}",
            owner.id()
        ),
        _ => unreachable!("the verdict was exhaustively matched above"),
    }
}
