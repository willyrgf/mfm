use std::usize;

use anyhow::{anyhow, Error};

use super::{context::Context, State, StateError, StateHandler, StateWrapper};

struct StateMachine {
    states: Vec<State>,
}

impl StateMachine {
    pub fn new(initial_states: Vec<State>) -> Self {
        Self {
            states: initial_states,
        }
    }

    fn has_state(&self, state_index: usize) -> bool {
        self.states.len() > state_index
    }

    pub fn execute(&self, context: &mut impl Context, state_index: usize) -> Result<(), Error> {
        if !self.has_state(0) {
            return Err(anyhow!("no states defined to execute"));
        }

        if !self.has_state(state_index) {
            return Ok(());
        }

        let current_state = &self.states[state_index];

        let result = match current_state {
            State::Setup(h) => h.handler(context),
            State::Report(h) => h.handler(context),
        };

        // TODO: it may be the transition
        match result {
            Ok(()) => self.execute(context, state_index + 1),
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
}

mod test {
    #[test]
    fn test_state_machine_execute() {
        use super::*;
        use crate::state::{context::RawContext, states};

        let initial_states = vec![
            State::Setup(StateWrapper::new(states::Setup::new())),
            State::Report(StateWrapper::new(states::Report::new())),
        ];

        let state_machine = StateMachine::new(initial_states.clone());

        let mut context = RawContext::new();
        let result = state_machine.execute(&mut context, 0);
        let last_ctx_message: String = context.read().unwrap();

        assert_eq!(state_machine.states, initial_states);
        assert!(result.is_ok());
        assert_eq!(last_ctx_message, "some new data".to_string());
    }
}
