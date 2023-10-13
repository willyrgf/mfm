use std::usize;

use super::{State, StateHandler};

struct StateMachine<T> {
    states: Vec<State<T>>,
    current_state_index: usize,
}

impl<T> StateMachine<T>
where
    T: StateHandler,
{
    pub fn new(initial_states: Vec<State<T>>) -> Self {
        Self {
            states: initial_states,
            current_state_index: 0,
        }
    }

    fn has_next_state(&self) -> bool {
        self.states.len() > self.current_state_index + 1
    }
}
