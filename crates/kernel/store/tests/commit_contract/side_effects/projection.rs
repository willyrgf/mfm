use super::*;

#[test]
fn side_effect_ledger_state_exposes_valid_prepared_view() {
    let run_id = run_id(120);
    let mut store = StoreContractRunStore::new();
    append_side_effect_prepare(&mut store, &run_id);

    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &side_effect_pair_id())
        .expect("side-effect projection");
    let pair_id = side_effect_pair_id();
    let pair_projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &pair_id)
        .expect("pair projection");
    assert_eq!(pair_projection.ledger_key, projection.ledger_key);
    let state = projection.ledger_state().expect("typed ledger state");
    let pair_state = store
        .projection_snapshot()
        .side_effect_state_for_pair(&run_id, &pair_id)
        .expect("pair state lookup")
        .expect("pair state");
    assert_eq!(pair_state.ledger_purpose(), state.ledger_purpose());
    assert!(matches!(
        pair_state.phase(),
        SideEffectLedgerPhase::Prepared { .. }
    ));
    assert!(state.is_forward_completion_candidate());
    let SideEffectLedgerPhase::Prepared {
        claim,
        prepared_invocation,
        resource_key,
    } = state.phase()
    else {
        panic!("expected prepared state");
    };
    assert_eq!(claim.claim_generation, 1);
    assert!(prepared_invocation.is_none());
    assert!(resource_key.is_none());
}

#[test]
fn side_effect_ledger_state_rejects_malformed_required_phase_evidence() {
    #[derive(Clone, Copy)]
    enum Case {
        MissingActiveClaim,
        MissingConfirmation,
    }

    for (case, expected_error) in [
        (Case::MissingActiveClaim, "phase requires active claim"),
        (
            Case::MissingConfirmation,
            "confirmation evidence is missing",
        ),
    ] {
        let run_id = run_id(120);
        let mut store = StoreContractRunStore::new();
        match case {
            Case::MissingActiveClaim => append_side_effect_prepare(&mut store, &run_id),
            Case::MissingConfirmation => append_forward_confirmation(&mut store, &run_id),
        }
        let mut projection = store
            .projection_snapshot()
            .side_effect_for_pair(&run_id, &side_effect_pair_id())
            .expect("side-effect projection")
            .clone();
        match case {
            Case::MissingActiveClaim => projection.claim = None,
            Case::MissingConfirmation => projection.confirmation = None,
        }

        let error = projection
            .ledger_state()
            .expect_err("malformed side-effect projection rejects");
        assert_projection_conflict_contains(error, expected_error);
    }
}

#[test]
fn side_effect_ledger_state_classifies_ambiguity_as_terminal_not_frontier() {
    let run_id = run_id(120);
    let mut store = StoreContractRunStore::new();
    append_side_effect_observation_setup(&mut store, &run_id);
    let ambiguity_artifact = artifact_id(121);
    let ambiguity_digest = content_digest(122);
    append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-ambiguity-typestate",
        vec![
            side_effect_ambiguous(ambiguity_artifact.clone(), ambiguity_digest.clone()),
            side_effect_attempt_failed(false),
        ],
        vec![side_effect_evidence(
            ambiguity_artifact,
            ambiguity_digest,
            schema_id("mfm.test.ambiguity", 76),
            ArtifactRole::AmbiguityEvidence,
        )],
    )
    .expect("append ambiguity terminal pair");

    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &side_effect_pair_id())
        .expect("side-effect projection");
    let state = projection.ledger_state().expect("typed ledger state");
    assert!(!state.is_forward_completion_candidate());
    assert!(matches!(
        state.phase(),
        SideEffectLedgerPhase::Ambiguous { .. }
    ));
}
