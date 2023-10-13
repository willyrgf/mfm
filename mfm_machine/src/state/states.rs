use anyhow::Error;

use crate::state::{Label, StateHandler, StateMetadata, Tag};

use super::context::{Context, ContextInput, ContextOutput};

struct SetupState {
    label: Label,
    tags: Vec<Tag>,
}

impl SetupState {
    pub fn new(label: Label, tags: Vec<Tag>) -> Self {
        Self { label, tags }
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

impl StateMetadata for SetupState {
    fn label(&self) -> &Label {
        &self.label
    }

    fn tags(&self) -> &[Tag] {
        &self.tags
    }
}

struct ReportState {
    label: Label,
    tags: Vec<Tag>,
}

impl ReportState {
    pub fn new(label: Label, tags: Vec<Tag>) -> Self {
        Self { label, tags }
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

impl StateMetadata for ReportState {
    fn label(&self) -> &Label {
        &self.label
    }

    fn tags(&self) -> &[Tag] {
        &self.tags
    }
}

#[cfg(test)]
mod test {
    use crate::state::State;

    use super::*;

    #[test]
    fn test_setup_state_initialization() {
        let label = Label::new("label_first").unwrap();
        let tags = vec![Tag::new("setup").unwrap(), Tag::new("tagxzy").unwrap()];
        let state: State<SetupState> = State::Setup(SetupState::new(label.clone(), tags.clone()));
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
