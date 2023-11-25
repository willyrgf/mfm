use std::{sync::Arc, usize};

use anyhow::anyhow;

use crate::state::{context::ContextWrapper, StateHandler, StateResult, States};

use self::tracker::{HashMapTracker, Index, Tracker};

pub mod tracker;

pub struct StateMachineBuilder {
    pub states: States,
    pub tracker: Option<Box<dyn Tracker>>,
}

impl StateMachineBuilder {
    pub fn new(states: States) -> Self {
        Self {
            states,
            tracker: None,
        }
    }

    pub fn tracker(mut self, tracker: Box<dyn Tracker>) -> Self {
        self.tracker = Some(tracker);
        self
    }

    pub fn build(self) -> StateMachine {
        StateMachine {
            states: self.states,
            tracker: self
                .tracker
                .unwrap_or_else(|| Box::new(HashMapTracker::new())),
        }
    }
}

pub struct StateMachine {
    pub states: States,
    pub tracker: Box<dyn Tracker>,
}

#[derive(Debug)]
pub enum StateMachineError {
    EmptyState((), anyhow::Error),
    InternalError(StateResult, anyhow::Error),
    StateError(StateResult, anyhow::Error),
}

impl StateMachine {
    pub fn new(states: States) -> Self {
        Self {
            states,
            tracker: Box::new(HashMapTracker::new()),
        }
    }

    fn has_state(&self, state_index: usize) -> bool {
        self.states.len() > state_index
    }

    // TODO: add logging, instrumentation
    fn transition(
        &mut self,
        mut context: ContextWrapper,
        state_index: usize,
        last_state_result: Option<StateResult>,
    ) -> Result<(usize, ContextWrapper), StateMachineError> {
        if !self.has_state(0) {
            return Err(StateMachineError::EmptyState(
                (),
                anyhow!("there no state to execute"),
            ));
        }

        let state = &self.states[state_index];
        let tracker = self.tracker.as_mut();
        // TODO: should state.label() return an &Label or Label?
        // TODO: remove this unwrap for proper handling
        tracker.track(
            Index::new(state_index, state.label(), state.tags()),
            context.clone(),
        );

        // if thats true, means that no state was executed before and this is the first one
        if last_state_result.is_none() {
            return Ok((state_index, context));
        }

        let state_result = last_state_result.unwrap();

        // TODO: it may be the transition
        match state_result {
            Ok(()) => Ok((state_index + 1, context)),
            Err(e) => {
                if e.is_recoverable() {
                    // FIXME: we're looking just for the first depends_on of a state
                    // we should implement an well defined rule for the whole dependency
                    // system between states, and follow this definition here as well.
                    let state_depends_on = state.depends_on();
                    let indexes_state_deps =
                        tracker.search_by_tag(state_depends_on.first().unwrap());

                    let last_index_of_first_dep = indexes_state_deps.last().unwrap().clone();

                    println!(
                        "trying to recover it from {:?}::{} to {:?}::{}",
                        state.label(),
                        state_index,
                        last_index_of_first_dep.state_label,
                        last_index_of_first_dep.state_index
                    );

                    let last_index_state_ctx =
                        tracker.recover(last_index_of_first_dep.clone()).unwrap();

                    println!("are we waiting for some lock here??");
                    context = last_index_state_ctx.clone();

                    println!("are we waiting for some lock here???");

                    // TODO: design the possible state recoverability and default cases
                    Ok((last_index_of_first_dep.state_index, context))
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
        &mut self,
        context: ContextWrapper,
        state_index: usize,
        last_state_result: Option<StateResult>,
    ) -> Result<(), StateMachineError> {
        let (next_state_index, context) =
            self.transition(context.clone(), state_index, last_state_result)?;

        if !self.has_state(next_state_index) {
            return Ok(());
        }

        let current_state = &self.states[next_state_index];

        let result = current_state.handler(context.clone());

        self.execute_rec(context, next_state_index, Option::Some(result))
    }

    pub fn execute(&mut self, context: ContextWrapper) -> Result<(), StateMachineError> {
        self.execute_rec(context, 0, Option::None)
    }
}

#[cfg(test)]
mod test {
    use crate::state::context::{wrap_context, ContextWrapper};
    use crate::state::{
        context::Context, DependencyStrategy, Label, StateHandler, StateMetadata, Tag,
    };
    use crate::state::{StateError, StateErrorRecoverability};
    use anyhow::anyhow;
    use anyhow::Error;
    use mfm_machine_macros::StateMetadataReqs;
    use serde_derive::{Deserialize, Serialize};
    use serde_json::{json, Value};

    use super::StateResult;

    #[derive(Serialize, Deserialize)]
    struct ContextA {
        a: String,
        b: u64,
    }

    impl ContextA {
        fn new(a: String, b: u64) -> Self {
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

    #[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
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
        fn handler(&self, context: ContextWrapper) -> StateResult {
            let value = context.lock().unwrap().read_input().unwrap();
            let _data: ContextA = serde_json::from_value(value).unwrap();
            let data = json!({ "a": "setting up", "b": 1 });
            match context.lock().as_mut().unwrap().write_output(&data) {
                Ok(()) => Ok(()),
                Err(e) => Err(StateError::StorageAccess(
                    StateErrorRecoverability::Recoverable,
                    e,
                )),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
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
        fn handler(&self, context: ContextWrapper) -> StateResult {
            let value = context.lock().unwrap().read_input().unwrap();
            let _data: ContextA = serde_json::from_value(value).unwrap();
            let data = json!({ "a": "some new data reported", "b": 7 });
            match context.lock().as_mut().unwrap().write_output(&data) {
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
        let ctx_input = wrap_context(ContextA::new(String::from("hello"), 7));

        let result = state.handler(ctx_input);

        assert!(result.is_ok());
        assert_eq!(state.label(), label);
        assert_eq!(state.tags(), tags);
    }

    #[test]
    fn test_state_machine_execute() {
        use super::*;

        let setup_state = Box::new(Setup::new());
        let report_state = Box::new(Report::new());

        let initial_states: States = Arc::new([setup_state.clone(), report_state.clone()]);
        let initial_states_cloned = initial_states.clone();

        let iss: Vec<(Label, Vec<Tag>, Vec<Tag>, DependencyStrategy)> = initial_states_cloned
            .iter()
            .map(|is| {
                (
                    is.label(),
                    is.tags(),
                    is.depends_on(),
                    is.depends_on_strategy(),
                )
            })
            .collect();

        let mut state_machine = StateMachine::new(initial_states);

        let context = wrap_context(ContextA::new(String::from("hello"), 7));
        let result = state_machine.execute(context.clone());
        let last_ctx_message = context.lock().unwrap().read_input().unwrap();

        assert_eq!(state_machine.states.len(), iss.len());

        state_machine.states.iter().zip(iss.iter()).for_each(
            |(s, (label, tags, depends_on, depends_on_strategy))| {
                assert_eq!(s.label(), *label);
                assert_eq!(s.tags(), *tags);
                assert_eq!(s.depends_on(), *depends_on);
                assert_eq!(s.depends_on_strategy(), *depends_on_strategy);
            },
        );

        assert!(result.is_ok());
        assert_eq!(
            last_ctx_message,
            json!({"a": "some new data reported", "b": 7})
        );
    }
}
