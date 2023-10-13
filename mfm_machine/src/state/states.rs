use anyhow::Error;

use crate::state::{Label, StateConfig, StateHandler, Tag};

use super::{
    context::{Context, ContextInput, ContextOutput},
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

impl StateHandler for SetupState {
    type InputContext = ContextInput;
    type OutputContext = ContextOutput;

    fn handler(&self, context: ContextInput) -> Result<ContextOutput, Error> {
        let _data = context.read();
        let data = "some new data".to_string();
        let ctx_output = ContextOutput {};
        ctx_output.write(&data);
        Ok(ctx_output)
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
    type InputContext = ContextInput;
    type OutputContext = ContextOutput;

    fn handler(&self, context: ContextInput) -> Result<ContextOutput, Error> {
        let _data = context.read();
        let data = "some new data".to_string();
        let ctx_output = ContextOutput {};
        ctx_output.write(&data);
        Ok(ctx_output)
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
        let ctx_input = ContextInput {};
        match state {
            State::Setup(t) => {
                let result = t.handler(ctx_input);
                assert!(result.is_ok());
                assert_eq!(t.label(), &label);
                assert_eq!(t.tags(), &tags);
            }
            _ => panic!("expected Setup state"),
        }
    }
}
