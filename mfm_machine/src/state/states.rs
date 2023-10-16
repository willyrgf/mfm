use anyhow::Error;
use mfm_machine_macros::StateReqs;
use serde_derive::{Deserialize, Serialize};

use crate::state::{context::Context, DependencyStrategy, Label, StateConfig, StateHandler, Tag};

#[derive(Debug, Clone, PartialEq, StateReqs)]
pub struct Setup {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Setup {
    pub fn new() -> Self {
        Self {
            label: Label::new("setup_state").unwrap(),
            tags: vec![Tag::new("setup").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct SetupStateData {}

impl StateHandler for Setup {
    fn handler<C: Context>(&self, context: &mut C) -> Result<(), Error> {
        let _data: SetupStateData = context.read().unwrap();
        let data = "some new data".to_string();
        context.write(&data)
    }
}

#[derive(Debug, Clone, PartialEq, StateReqs)]
pub struct Report {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

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
    fn handler<C: Context>(&self, context: &mut C) -> Result<(), Error> {
        let _data: String = context.read().unwrap();
        let data = "some new data reported".to_string();
        context.write(&data)
    }
}

#[cfg(test)]
mod test {
    use crate::state::{context::RawContext, State, StateWrapper};

    use super::*;

    #[test]
    fn test_setup_state_initialization() {
        let label = Label::new("setup_state").unwrap();
        let tags = vec![Tag::new("setup").unwrap()];
        let state = State::Setup(StateWrapper::new(Setup::new()));
        let mut ctx_input = RawContext::new();
        match state {
            State::Setup(t) => {
                let result = t.handler(&mut ctx_input);
                assert!(result.is_ok());
                assert_eq!(t.label(), &label);
                assert_eq!(t.tags(), &tags);
            }
            _ => panic!("expected Setup state"),
        }
    }
}
