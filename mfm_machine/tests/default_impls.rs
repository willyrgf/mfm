use anyhow::anyhow;
use anyhow::Error;
use mfm_machine::state::context::Context;
use mfm_machine::state::context::ContextWrapper;
use mfm_machine::state::DependencyStrategy;
use mfm_machine::state::Label;
use mfm_machine::state::StateError;
use mfm_machine::state::StateErrorRecoverability;
use mfm_machine::state::StateHandler;
use mfm_machine::state::StateMetadata;
use mfm_machine::state::StateResult;
use mfm_machine::state::Tag;
use mfm_machine_derive::StateMetadataReqs;
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
    fn read(&self) -> Result<Value, anyhow::Error> {
        serde_json::to_value(self).map_err(|e| anyhow!(e))
    }

    fn write(&mut self, value: &Value) -> Result<(), Error> {
        let ctx: ContextA = serde_json::from_value(value.clone()).map_err(|e| anyhow!(e))?;
        self.a = ctx.a;
        self.b = ctx.b;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
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
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let value = context.lock().unwrap().read().unwrap();
        let _data: ContextA = serde_json::from_value(value).unwrap();

        let mut rng = rand::thread_rng();
        let data = json!({ "a": "setting up", "b": rng.gen_range(0..9) });

        match context.lock().as_mut().unwrap().write(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Recoverable,
                e,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
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
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let value = context.lock().unwrap().read().unwrap();
        let _data: ContextA = serde_json::from_value(value).unwrap();
        if _data.b % 2 == 0 {
            return Err(StateError::ParsingInput(
                StateErrorRecoverability::Recoverable,
                anyhow!("the input is even, should be odd"),
            ));
        }

        let data = json!({ "a": "the input number is odd", "b": _data.b });
        match context.lock().as_mut().unwrap().write(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Unrecoverable,
                e,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
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
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let value = context.lock().unwrap().read().unwrap();
        let _data: ContextA = serde_json::from_value(value).unwrap();
        let data =
            json!({ "a": format!("{}: {}", "some new data reported", _data.a), "b": _data.b });
        match context.lock().as_mut().unwrap().write(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Recoverable,
                e,
            )),
        }
    }
}

// ---
#[derive(Serialize, Deserialize)]
pub struct Config {
    pub a: String,
    pub b: String,
}

#[derive(Serialize, Deserialize)]
pub struct ConfigStateCtx {
    pub config: Config,
    pub c: String,
}

impl ConfigStateCtx {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            c: String::new(),
        }
    }
}

impl Context for ConfigStateCtx {
    fn read(&self) -> Result<Value, anyhow::Error> {
        serde_json::to_value(self).map_err(|e| anyhow!(e))
    }

    fn write(&mut self, value: &Value) -> Result<(), Error> {
        let ctx: ConfigStateCtx = serde_json::from_value(value.clone()).map_err(|e| anyhow!(e))?;
        self.config = ctx.config;
        self.c = ctx.c;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct OnChainValuesCtx {
    pub config: Config,
    pub c: String,
    pub values: Vec<String>,
}

impl OnChainValuesCtx {
    // TODO: may be a from?
    pub fn new(config_ctx: ConfigStateCtx) -> Self {
        Self {
            config: config_ctx.config,
            c: config_ctx.c,
            values: vec![],
        }
    }
}

impl Context for OnChainValuesCtx {
    fn read(&self) -> Result<Value, anyhow::Error> {
        serde_json::to_value(self).map_err(|e| anyhow!(e))
    }

    fn write(&mut self, value: &Value) -> Result<(), Error> {
        let ctx: OnChainValuesCtx =
            serde_json::from_value(value.clone()).map_err(|e| anyhow!(e))?;
        self.config = ctx.config;
        self.c = ctx.c;
        self.values = ctx.values;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct ConfigState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for ConfigState {
    fn default() -> Self {
        Self::new()
    }
}
impl ConfigState {
    pub fn new() -> Self {
        Self {
            label: Label::new("config_state").unwrap(),
            tags: vec![Tag::new("setup").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for ConfigState {
    fn handler(&self, context: ContextWrapper) -> StateResult {
        //let value = context.lock().unwrap().read().unwrap();
        //let _data: ConfigStateCtx = serde_json::from_value(value).unwrap();

        let config = Config {
            a: "config_a".to_string(),
            b: "config_b".to_string(),
        };
        let config_state_ctx = ConfigStateCtx::new(config);

        let data = serde_json::to_value(config_state_ctx).unwrap();

        match context.lock().as_mut().unwrap().write(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Recoverable,
                e,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct OnChainValuesState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for OnChainValuesState {
    fn default() -> Self {
        Self::new()
    }
}

impl OnChainValuesState {
    pub fn new() -> Self {
        Self {
            label: Label::new("onchain_values").unwrap(),
            tags: vec![Tag::new("computation").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for OnChainValuesState {
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let value = context.lock().unwrap().read().unwrap();
        let _data: ConfigStateCtx = serde_json::from_value(value).unwrap();

        let mut onchain_value_ctx = OnChainValuesCtx::new(_data);
        onchain_value_ctx.values = vec!["txn1".to_string(), "txn2".to_string()];

        let data = serde_json::to_value(onchain_value_ctx).unwrap();

        match context.lock().as_mut().unwrap().write(&data) {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Unrecoverable,
                e,
            )),
        }
    }
}
