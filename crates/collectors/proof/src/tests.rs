use super::*;

#[test]
fn proof_apply_config_accepts_checked_actions_at_construction() {
    assert!(ProofApplyConfig::new("accept").is_ok());
}

#[test]
fn proof_workflow_config_has_distinct_canonical_shape_from_runtime_intent() {
    let config = ProofWorkflowConfig::default();
    let intent = ProofIntent {
        fact_n: config.read.fact_n,
        action: config.apply.action().to_owned(),
    };

    let config_json = serde_json::to_string(&config).expect("config json");
    let intent_json = serde_json::to_string(&intent).expect("intent json");
    let config_bytes =
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&config_json).expect("config");
    let intent_bytes =
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&intent_json).expect("intent");

    assert_ne!(config_bytes.as_bytes(), intent_bytes.as_bytes());
    assert_ne!(config_bytes.content_digest(), intent_bytes.content_digest());
}

#[test]
fn proof_side_effect_outputs_reflect_terminal_evidence_level() {
    let state = ProofApplySideEffectState::new(
        mfm_program::ValidatedConfig::new(ProofApplyConfig::new("accept").expect("config"))
            .expect("validated config"),
    )
    .expect("state");
    let input = ProofFact { n: 7 };
    let context = mfm_program::CertifiedContext::no_context();
    let intent = state.prepare_intent(&input, &context).expect("intent");

    let receipt = state
        .output_from_receipt(
            &input,
            &intent,
            &ProofReceipt {
                tx_hash: "0xreceipt".to_owned(),
                submission_id: "proof-submission-accept-7".to_owned(),
            },
            &context,
        )
        .expect("receipt output");
    let confirmation = state
        .output_from_confirmation(
            &input,
            &intent,
            &ProofConfirmation {
                tx_hash: "0xconfirmed".to_owned(),
                confirmations: 3,
            },
            &context,
        )
        .expect("confirmation output");

    assert_eq!(receipt.status, "receipt_observed");
    assert_eq!(receipt.confirmations, 0);
    assert_eq!(confirmation.status, "confirmed");
    assert_eq!(confirmation.confirmations, 3);
}
