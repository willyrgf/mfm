use std::usize;

use anyhow::anyhow;

use crate::state::{context::ContextWrapper, StateResult, States};

use self::tracker::{HashMapTracker, Index, Tracker, TrackerHistory};

pub mod tracker;

pub struct StateMachineBuilder {
    pub states: States,
    pub tracker: Option<Box<dyn Tracker>>,
    pub max_recoveries: usize,
}

pub const MAX_RECOVERIES_MULT: usize = 3;

// default_max_recoveries is number of states * MAX_RECOVERIES_MULT + 1
pub fn default_max_recoveries(states: States) -> usize {
    states.len() * MAX_RECOVERIES_MULT + 1
}

impl StateMachineBuilder {
    pub fn new(states: States) -> Self {
        Self {
            states: states.clone(),
            tracker: None,
            max_recoveries: default_max_recoveries(states),
        }
    }

    pub fn tracker(mut self, tracker: Box<dyn Tracker>) -> Self {
        self.tracker = Some(tracker);
        self
    }

    pub fn max_recoveries(mut self, max: usize) -> Self {
        self.max_recoveries = max;
        self
    }

    pub fn build(self) -> StateMachine {
        StateMachine {
            states: self.states,
            tracker: self
                .tracker
                .unwrap_or_else(|| Box::new(HashMapTracker::new())),
            max_recoveries: self.max_recoveries,
        }
    }
}

pub struct StateMachine {
    pub states: States,
    pub tracker: Box<dyn Tracker>,
    max_recoveries: usize,
}

#[derive(Debug)]
pub enum StateMachineError {
    ReachedMaxRecoveries((), anyhow::Error),
    EmptyState((), anyhow::Error),
    InternalError(StateResult, anyhow::Error),
    StateError(StateResult, anyhow::Error),
}

impl StateMachine {
    pub fn new(states: States) -> Self {
        Self {
            states: states.clone(),
            tracker: Box::new(HashMapTracker::new()),
            max_recoveries: default_max_recoveries(states),
        }
    }

    pub fn track_history(&self) -> TrackerHistory {
        self.tracker.history()
    }

    fn has_state(&self, state_index: usize) -> bool {
        self.states.len() > state_index
    }

    fn reached_max_recoveries(&self) -> (bool, usize) {
        let steps = self.track_history().len();
        (steps >= self.max_recoveries, steps)
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

        if let (true, steps) = self.reached_max_recoveries() {
            return Err(StateMachineError::ReachedMaxRecoveries(
                (),
                anyhow!("reached max recoveries ({})", steps),
            ));
        }

        let state = &self.states[state_index];

        // if thats true, means that no state was executed before and this is the first one
        if last_state_result.is_none() {
            return Ok((state_index, context));
        }

        let state_result = last_state_result.unwrap();

        // //FIXME: state_machine.track_history() show be enough
        // let value = context.lock().unwrap().dump().unwrap();
        // println!(
        //     "state: {:?}; state_index: {}; state_result: {:?}; value: {:?}",
        //     state.label(),
        //     state_index,
        //     state_result,
        //     value,
        // );

        // TODO: it may be the transition
        match state_result {
            Ok(()) => Ok((state_index + 1, context)),
            Err(e) => {
                if e.is_recoverable() {
                    // FIXME: we're looking just for the first depends_on of a state
                    // we should implement an well defined rule for the whole dependency
                    // system between states, and follow this definition here as well.
                    let state_depends_on = state.depends_on();
                    let indexes_state_deps = self
                        .tracker
                        .search_by_tag(state_depends_on.first().unwrap());

                    let last_index_of_first_dep = indexes_state_deps.last().unwrap().clone();

                    let last_index_state_ctx = self
                        .tracker
                        .recover(last_index_of_first_dep.clone())
                        .unwrap();

                    context = last_index_state_ctx.clone();

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

        let state = &self.states[next_state_index];

        let result = state.handler(context.clone());
        let _ = self.tracker.as_mut().track(
            Index::new(next_state_index, state.label(), state.tags()),
            context.clone(),
        );

        self.execute_rec(context, next_state_index, Option::Some(result))
    }

    pub fn execute(&mut self, context: ContextWrapper) -> Result<(), StateMachineError> {
        self.execute_rec(context, 0, Option::None)
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::state::context::{wrap_context, Context, ContextWrapper, Local};
    use crate::state::{DependencyStrategy, Label, StateHandler, StateMetadata, Tag};
    use crate::state::{StateError, StateErrorRecoverability};
    use mfm_machine_derive::StateMetadataReqs;
    use serde_derive::{Deserialize, Serialize};
    use serde_json::json;

    use super::StateResult;

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

    #[derive(Serialize, Deserialize)]
    struct SetupCtx {
        a: String,
        b: u32,
    }

    impl StateHandler for Setup {
        fn handler(&self, context: ContextWrapper) -> StateResult {
            let data = SetupCtx {
                a: "setup_b".to_string(),
                b: 1,
            };
            match context
                .lock()
                .as_mut()
                .unwrap()
                .write("setup".to_string(), &json!(data))
            {
                Ok(()) => Ok(()),
                Err(e) => Err(StateError::StorageAccess(
                    StateErrorRecoverability::Recoverable,
                    e,
                )),
            }
        }
    }

    #[derive(Serialize, Deserialize)]
    pub struct ReportCtx {
        pub report_msg: String,
        pub report_value: u32,
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
            let setup_ctx: SetupCtx =
                serde_json::from_value(context.lock().unwrap().read("setup".to_string()).unwrap())
                    .unwrap();
            let data = json!(ReportCtx {
                report_msg: format!("{}: {}", "some new data reported", setup_ctx.a),
                report_value: setup_ctx.b
            });
            match context
                .lock()
                .as_mut()
                .unwrap()
                .write("report".to_string(), &data)
            {
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
        let label = Label::new("setup_state").unwrap();
        let tags = vec![Tag::new("setup").unwrap()];
        let state = Setup::new();
        let ctx_input = wrap_context(Local::default());

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

        let context = wrap_context(Local::default());
        let result = state_machine.execute(context.clone());
        let last_ctx_message = context.lock().unwrap().dump().unwrap();

        assert_eq!(state_machine.states.len(), iss.len());

        state_machine.states.iter().zip(iss.iter()).for_each(
            |(s, (label, tags, depends_on, depends_on_strategy))| {
                assert_eq!(s.label(), *label);
                assert_eq!(s.tags(), *tags);
                assert_eq!(s.depends_on(), *depends_on);
                assert_eq!(s.depends_on_strategy(), *depends_on_strategy);
            },
        );

        let last_ctx_data: Local = serde_json::from_value(last_ctx_message).unwrap();
        let report_ctx: ReportCtx =
            serde_json::from_value(last_ctx_data.read("report".to_string()).unwrap()).unwrap();

        println!("report_msg: {}", report_ctx.report_msg);

        assert!(result.is_ok());
        assert_eq!(
            report_ctx.report_msg,
            String::from("some new data reported: setup_b")
        );

        assert_eq!(report_ctx.report_value, 1);
    }
}
