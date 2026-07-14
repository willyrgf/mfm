use super::*;

#[test]
fn proof_program_lowers_to_typed_state_contracts() {
    let draft = proof_program_draft(ProofWorkflowConfig::default()).expect("draft");
    assert_eq!(draft.state_nodes().len(), 3);
    let state_keys = draft
        .state_nodes()
        .iter()
        .map(|node| node.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        state_keys,
        ["read_fact", "apply_side_effect", "assemble_output"]
    );

    let certified = mfm_certify::certify_program_draft(&draft).expect("certified proof spec");
    certified.envelope().verify_hash().expect("hash verifies");
    let side_effect = certified
        .envelope()
        .spec
        .nodes
        .iter()
        .find(|node| node.stable_key.as_str() == "apply_side_effect")
        .expect("side-effect node");
    assert!(
        side_effect.side_effect.is_some(),
        "proof side effect declares a typed side-effect contract"
    );
    assert_eq!(side_effect.adapter_bindings.len(), 1);
}

#[test]
fn proof_manual_resolution_program_certifies() {
    let certified = certified_manual_resolution_proof_spec(ProofWorkflowConfig::default())
        .expect("certified manual proof spec");
    assert!(matches!(
        certified.envelope().spec.saga,
        mfm_spec::v1::SagaPolicySpec::ManualResolution { .. }
    ));
    assert!(
        certified.envelope().spec.nodes.iter().any(|node| matches!(
            node.framework,
            Some(mfm_spec::v1::FrameworkNodeSpec::ResolveSagaTerminal(_))
        )),
        "manual-resolution proof spec must include a saga terminal resolver"
    );
}
