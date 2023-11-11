extern crate mfm_machine;

use anyhow::anyhow;
use anyhow::Error;
use mfm_machine::state::context::Context;
use mfm_machine::state::DependencyStrategy;
use mfm_machine::state::Label;
use mfm_machine::state::StateConfig;
use mfm_machine::state::StateError;
use mfm_machine::state::StateErrorRecoverability;
use mfm_machine::state::StateHandler;
use mfm_machine::state::StateResult;
use mfm_machine::state::Tag;
use mfm_machine::StateConfigReqs;
use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize)]
struct ContextA {
    a: String,
    b: u64,
}

impl ContextA {
    fn _new(a: String, b: u64) -> Self {
        Self { a, b }
    }
}

impl mfm_machine::state::context::Context for ContextA {
    fn read_input(&self) -> Result<Value, anyhow::Error> {
        serde_json::to_value(self).map_err(|e| anyhow!(e))
    }

    fn write_output(&mut self, value: &Value) -> Result<(), Error> {
        let ctx: ContextA = serde_json::from_value(value.clone()).map_err(|e| anyhow!(e))?;
        self.a = ctx.a;
        self.b = ctx.b;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, StateConfigReqs)]
pub struct Setup {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for Setup {
    fn default() -> Self {
        Self::new()
    }
}

impl Setup {
    fn new() -> Self {
        Self {
            label: Label::new("setup_state").unwrap(),
            tags: vec![Tag::new("setup").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for Setup {
    fn handler(&self, context: &mut dyn Context) -> StateResult {
        let _data: ContextA = serde_json::from_value(context.read_input().unwrap()).unwrap();
        let data = json!({ "a": "setting up", "b": 1 });
        match context.write_output(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Recoverable,
                e,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, StateConfigReqs)]
pub struct Report {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl Report {
    pub fn new() -> Self {
        Self {
            label: Label::new("report_state").unwrap(),
            tags: vec![Tag::new("report").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for Report {
    fn handler(&self, context: &mut dyn Context) -> StateResult {
        let _data: ContextA = serde_json::from_value(context.read_input().unwrap()).unwrap();
        let data = json!({ "a": "some new data reported", "b": 7 });
        match context.write_output(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Recoverable,
                e,
            )),
        }
    }
}

#[test]
fn test_state_machine_execute() {
    use mfm_machine::state::state_machine::StateMachine;
    use std::sync::Arc;

    let setup_state = Box::new(Setup::new());
    let report_state = Box::new(Report::new());

    let initial_states: Arc<[Box<dyn StateHandler>]> =
        Arc::new([setup_state.clone(), report_state.clone()]);
    let initial_states_cloned = initial_states.clone();

    let iss: Vec<(Label, &[Tag], &[Tag], DependencyStrategy)> = initial_states_cloned
        .iter()
        .map(|is| {
            (
                is.label().clone(),
                is.tags().clone(),
                is.depends_on().clone(),
                is.depends_on_strategy().clone(),
            )
        })
        .collect();

    let state_machine = StateMachine::new(initial_states);

    let mut context = ContextA::_new(String::from("hello"), 7);
    let result = state_machine.execute(&mut context);
    let last_ctx_message = context.read_input().unwrap();

    assert_eq!(state_machine.states.len(), iss.len());

    state_machine.states.iter().zip(iss.iter()).for_each(
        |(s, (label, tags, depends_on, depends_on_strategy))| {
            assert_eq!(s.label(), label);
            assert_eq!(s.tags(), *tags);
            assert_eq!(s.depends_on(), *depends_on);
            assert_eq!(s.depends_on_strategy(), depends_on_strategy);
        },
    );

    assert!(result.is_ok());
    assert_eq!(
        last_ctx_message,
        json!({"a": "some new data reported", "b": 7})
    );
}
