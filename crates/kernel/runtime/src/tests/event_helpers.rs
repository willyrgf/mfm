use super::*;

struct SyntheticSideEffectAppend<'a> {
    fixture: &'a Fixture,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    attempt_id: &'a AttemptId,
}

impl<'a> SyntheticSideEffectAppend<'a> {
    fn new(
        fixture: &'a Fixture,
        run_id: &'a RunId,
        node: &'a spec::NodeSpec,
        attempt_id: &'a AttemptId,
    ) -> Self {
        Self {
            fixture,
            run_id,
            node,
            attempt_id,
        }
    }

    fn ledger_purpose(&self) -> events::SideEffectLedgerPurpose {
        events::SideEffectLedgerPurpose::Forward
    }

    fn pair_fields(
        &self,
        node: &spec::NodeSpec,
        role: events::SideEffectPairRole,
    ) -> (SideEffectPairId, events::SideEffectPairRole) {
        side_effect_pair_fields_for_purpose(
            &self.fixture.runtime_spec,
            &node.node_id,
            &self.ledger_purpose(),
            role,
        )
    }

    fn append(
        &self,
        store: &mut TestTypedRunStore,
        commit_key: &str,
        payloads: Vec<events::KernelEventPayload>,
        required_artifacts: Vec<store::ArtifactEvidenceRef>,
        required_side_effect_state: store::RequiredSideEffectState,
        require_attempt_started: bool,
    ) {
        let required_present_logical_keys = require_attempt_started
            .then(|| {
                store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    self.node.node_id, self.attempt_id
                ))
                .expect("attempt logical key")
            })
            .into_iter()
            .collect::<Vec<_>>();
        store
            .append_prepared_commit(store_typed_commit_request! {
                run_id: self.run_id.clone(),
                expected_next_seq: store.expected_next_seq(self.run_id),
                commit_key: store::CommitKey::new(commit_key).expect("commit key"),
                payloads: payloads,
                required_artifacts: required_artifacts,
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys,
                    required_side_effect_states: vec![store::SideEffectStatePrecondition {
                        pair_id: fixture_side_effect_pair_id(self.fixture, self.node),
                        required: required_side_effect_state,
                    }],
                    certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                        self.run_id.clone(),
                        self.fixture.runtime_spec.spec(),
                    )
                    .expect("certified run authority")),
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append synthetic side-effect commit");
    }
}

#[path = "event_helpers/side_effect_append.rs"]
mod side_effect_append;
pub(super) use self::side_effect_append::*;
#[path = "event_helpers/side_effect_phases.rs"]
mod side_effect_phases;
pub(super) use self::side_effect_phases::*;
#[path = "event_helpers/manual_resolution.rs"]
mod manual_resolution;
pub(super) use self::manual_resolution::*;
#[path = "event_helpers/facts.rs"]
mod facts;
pub(super) use self::facts::*;
#[path = "event_helpers/terminal.rs"]
mod terminal;
pub(super) use self::terminal::*;
