use anyhow::anyhow;
use anyhow::Error;
use mfm_machine::state::context::Context;
use mfm_machine::state::DependencyStrategy;
use mfm_machine::state::Label;
use mfm_machine::state::StateConfig;
use mfm_machine::state::StateError;
use mfm_machine::state::StateErrorRecoverability;
use mfm_machine::state::StateHandler;
use mfm_machine::state::StateResult;
use mfm_machine::state::Tag;
use mfm_machine::StateConfigReqs;
use rand::Rng;
use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize)]
pub struct ContextA {
    pub a: String,
    pub b: u64,
}

impl ContextA {
    pub fn new(a: String, b: u64) -> Self {
        Self { a, b }
    }
}

impl Context for ContextA {
    fn read_input(&self) -> Result<Value, anyhow::Error> {
        serde_json::to_value(self).map_err(|e| anyhow!(e))
    }

    fn write_output(&mut self, value: &Value) -> Result<(), Error> {
        let ctx: ContextA = serde_json::from_value(value.clone()).map_err(|e| anyhow!(e))?;
        self.a = ctx.a;
        self.b = ctx.b;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, StateConfigReqs)]
pub struct Setup {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for Setup {
    fn default() -> Self {
        Self::new()
    }
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

impl StateHandler for Setup {
    fn handler(&self, context: &mut dyn Context) -> StateResult {
        let _data: ContextA = serde_json::from_value(context.read_input().unwrap()).unwrap();
        let mut rng = rand::thread_rng();
        let data = json!({ "a": "setting up", "b": rng.gen_range(0..9) });
        match context.write_output(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Recoverable,
                e,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, StateConfigReqs)]
pub struct ComputePrice {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for ComputePrice {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputePrice {
    pub fn new() -> Self {
        Self {
            label: Label::new("compute_price").unwrap(),
            tags: vec![Tag::new("computation").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for ComputePrice {
    fn handler(&self, context: &mut dyn Context) -> StateResult {
        let _data: ContextA = serde_json::from_value(context.read_input().unwrap()).unwrap();
        if _data.b % 2 == 0 {
            return Err(StateError::ParsingInput(
                StateErrorRecoverability::Recoverable,
                anyhow!("the input is even, should be odd"),
            ));
        }

        let data = json!({ "a": "the input number is odd", "b": _data.b });
        match context.write_output(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Unrecoverable,
                e,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, StateConfigReqs)]
pub struct Report {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
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
    fn handler(&self, context: &mut dyn Context) -> StateResult {
        let _data: ContextA = serde_json::from_value(context.read_input().unwrap()).unwrap();
        let data =
            json!({ "a": format!("{}: {}", "some new data reported", _data.a), "b": _data.b });
        match context.write_output(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Recoverable,
                e,
            )),
        }
    }
}
