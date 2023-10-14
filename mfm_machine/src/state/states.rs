use anyhow::Error;
use serde_derive::{Deserialize, Serialize};

use crate::state::{Label, StateConfig, StateHandler, Tag};

use super::{
    context::{Context, RawContext},
    DependencyStrategy,
};

struct SetupState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl SetupState {
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

impl StateHandler for SetupState {
    fn handler<C: Context>(&self, context: &mut C) -> Result<(), Error> {
        let _data: SetupStateData = context.read().unwrap();
        let data = "some new data".to_string();
        context.write(&data)
    }
}

impl StateConfig for SetupState {
    fn label(&self) -> &Label {
        &self.label
    }

    fn tags(&self) -> &[Tag] {
        &self.tags
    }

    fn depends_on(&self) -> &[Tag] {
        &self.depends_on
    }

    fn depends_on_strategy(&self) -> &DependencyStrategy {
        &self.depends_on_strategy
    }
}

struct ReportState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl ReportState {
    pub fn new() -> Self {
        Self {
            label: Label::new("report_state").unwrap(),
            tags: vec![Tag::new("report").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for ReportState {
    fn handler<C: Context>(&self, context: &mut C) -> Result<(), Error> {
        let _data: String = context.read().unwrap();
        let data = "some new data".to_string();
        context.write(&data)
    }
}

impl StateConfig for ReportState {
    fn label(&self) -> &Label {
        &self.label
    }

    fn tags(&self) -> &[Tag] {
        &self.tags
    }

    fn depends_on(&self) -> &[Tag] {
        &self.depends_on
    }

    fn depends_on_strategy(&self) -> &DependencyStrategy {
        &self.depends_on_strategy
    }
}

#[cfg(test)]
mod test {
    use crate::state::State;

    use super::*;

    #[test]
    fn test_setup_state_initialization() {
        let label = Label::new("setup_state").unwrap();
        let tags = vec![Tag::new("setup").unwrap()];
        let state: State<SetupState> = State::Setup(SetupState::new());
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
