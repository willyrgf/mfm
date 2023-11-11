use std::{sync::Arc, usize};

use anyhow::anyhow;

use super::{context::Context, StateHandler, StateResult};

pub struct StateMachine {
    pub states: Arc<[Box<dyn StateHandler>]>,
}

#[derive(Debug)]
pub enum StateMachineError {
    EmptyState((), anyhow::Error),
    InternalError(StateResult, anyhow::Error),
    StateError(StateResult, anyhow::Error),
}

impl StateMachine {
    pub fn new(initial_states: Arc<[Box<dyn StateHandler>]>) -> Self {
        Self {
            states: initial_states,
        }
    }

    fn has_state(&self, state_index: usize) -> bool {
        self.states.len() > state_index
    }

    // TODO: add logging, instrumentation
    fn transition(
        &self,
        _context: &mut impl Context,
        state_index: usize,
        last_state_result: Option<StateResult>,
    ) -> Result<usize, StateMachineError> {
        if !self.has_state(0) {
            return Err(StateMachineError::EmptyState(
                (),
                anyhow!("there no state to execute"),
            ));
        }

        // if thats true, means that no state was executed before and this is the first one
        if last_state_result.is_none() {
            return Ok(state_index);
        }

        let state_result = last_state_result.unwrap();

        // TODO: it may be the transition
        match state_result {
            Ok(()) => Ok(state_index + 1),
            Err(e) => {
                if e.is_recoverable() {
                    // TODO: design the possible state recoverability and default cases
                    Ok(state_index)
                } else {
                    Err(StateMachineError::StateError(
                        Err(e),
                        anyhow!("an unrecoverable error happened inside a state handler"),
                    ))
                }
            }
        }
    }

    fn execute_rec(
        &self,
        context: &mut impl Context,
        state_index: usize,
        last_state_result: Option<StateResult>,
    ) -> Result<(), StateMachineError> {
        let next_state_index = self.transition(context, state_index, last_state_result)?;

        if !self.has_state(next_state_index) {
            return Ok(());
        }

        let current_state = &self.states[next_state_index];

        let result = current_state.handler(context);

        self.execute_rec(context, next_state_index, Option::Some(result))
    }

    pub fn execute(&self, context: &mut impl Context) -> Result<(), StateMachineError> {
        self.execute_rec(context, 0, Option::None)
    }
}

#[cfg(test)]
mod test {
    use crate::state::{
        context::Context, DependencyStrategy, Label, StateConfig, StateHandler, Tag,
    };
    use crate::state::{StateError, StateErrorRecoverability};
    use anyhow::anyhow;
    use anyhow::Error;
    use mfm_machine_macros::StateConfigReqs;
    use serde_derive::{Deserialize, Serialize};
    use serde_json::{json, Value};

    use super::StateResult;

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

    impl Context for ContextA {
        fn read_input(&self) -> Result<Value, Error> {
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
    fn test_setup_state_initialization() {
        use super::*;

        let label = Label::new("setup_state").unwrap();
        let tags = vec![Tag::new("setup").unwrap()];
        let state = Setup::new();
        let mut ctx_input = ContextA::_new(String::from("hello"), 7);

        let result = state.handler(&mut ctx_input);

        assert!(result.is_ok());
        assert_eq!(state.label(), &label);
        assert_eq!(state.tags(), &tags);
    }

    #[test]
    fn test_state_machine_execute() {
        use super::*;

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
}
