use std::usize;

use anyhow::{anyhow, Error};

use super::{context::Context, State, StateError, StateHandler};

struct StateMachine<T> {
    states: Vec<State<T>>,
}

impl<T> StateMachine<T>
where
    T: StateHandler,
{
    pub fn new(initial_states: Vec<State<T>>) -> Self {
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
