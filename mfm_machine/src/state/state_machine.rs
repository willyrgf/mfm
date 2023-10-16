use std::usize;

use anyhow::{anyhow, Error};

use super::{context::Context, State, StateError};

struct StateMachine {
    states: Vec<State>,
}

type StateResult = Result<(), Error>;

impl StateMachine {
    pub fn new(initial_states: Vec<State>) -> Self {
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
    ) -> Result<usize, Error> {
        if !self.has_state(0) {
            return Err(anyhow!("no states defined to execute"));
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
                match e.downcast::<StateError>() {
                    Ok(se) if se.is_recoverable() => {
                        // TODO: replay based on depends_on logic
                        // TODO: extract it to an generic func
                        Err(se.into())
                    }
                    Ok(se) => Err(se.into()),
                    // TODO: what we want to return in this case?
                    Err(ed) => Err(ed),
                }
            }
        }
    }

    fn execute_rec(
        &self,
        context: &mut impl Context,
        state_index: usize,
        last_state_result: Option<StateResult>,
    ) -> Result<(), Error> {
        let next_state_index = self.transition(context, state_index, last_state_result)?;

        if !self.has_state(next_state_index) {
            return Ok(());
        }

        let current_state = &self.states[next_state_index];

        let result = match current_state {
            State::Setup(h) => h.handler(context),
            State::Report(h) => h.handler(context),
        };

        self.execute_rec(context, next_state_index, Option::Some(result))
    }

    pub fn execute(&self, context: &mut impl Context) -> Result<(), Error> {
        self.execute_rec(context, 0, Option::None)
    }
}

mod test {
    #[test]
    fn test_state_machine_execute() {
        use super::*;
        use crate::state::{context::RawContext, states, StateWrapper};

        let initial_states = vec![
            State::Setup(StateWrapper::new(states::Setup::new())),
            State::Report(StateWrapper::new(states::Report::new())),
        ];

        let state_machine = StateMachine::new(initial_states.clone());

        let mut context = RawContext::new();
        let result = state_machine.execute(&mut context);
        let last_ctx_message: String = context.read().unwrap();

        assert_eq!(state_machine.states, initial_states);
        assert!(result.is_ok());
        assert_eq!(last_ctx_message, "some new data reported".to_string());
    }
}
