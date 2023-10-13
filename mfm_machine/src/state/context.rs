use serde::{Deserialize, Serialize};

pub trait Context {
    type Output: Deserialize<'static>;
    type Input: Serialize;

    fn read(&self) -> Self::Output;
    fn write(&self, ctx_input: &Self::Input);
}

#[derive(Debug)]
pub struct ContextInput {}

#[derive(Debug)]
pub struct ContextOutput {}

impl Context for ContextInput {
    type Output = String;
    type Input = String;

    fn read(&self) -> Self::Output {
        "hello".to_string()
    }

    fn write(&self, ctx_input: &Self::Input) {
        let _x = ctx_input;
    }
}

impl Context for ContextOutput {
    type Input = String;
    type Output = String;

    fn read(&self) -> Self::Output {
        "hello".to_string()
    }

    fn write(&self, ctx_input: &Self::Input) {
        let _x = ctx_input;
    }
}
