use anyhow::anyhow;
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
use serde_json::json;

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct Setup {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

#[derive(Serialize, Deserialize)]
pub struct SetupCtx {
    a: String,
    b: u32,
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
        let mut rng = rand::thread_rng();
        let data = SetupCtx {
            a: "setup_b".to_string(),
            b: rng.gen_range(0..9),
        };

        match context
            .lock()
            .as_mut()
            .unwrap()
            .write("setup".to_string(), &json!(data))
        {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Recoverable,
                e,
            )),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ComputePriceCtx {
    msg: String,
    b: u32,
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
        let value = context.lock().unwrap().read("setup".to_string()).unwrap();
        let _data: SetupCtx = serde_json::from_value(value).unwrap();
        if _data.b % 2 == 0 {
            return Err(StateError::ParsingInput(
                StateErrorRecoverability::Recoverable,
                anyhow!("the input is even, should be odd"),
            ));
        }

        let data = ComputePriceCtx {
            msg: "the input number is odd".to_string(),
            b: _data.b,
        };
        match context
            .lock()
            .as_mut()
            .unwrap()
            .write("compute".to_string(), &json!(data))
        {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Unrecoverable,
                e,
            )),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ReportCtx {
    pub report_msg: String,
    pub report_value: u32,
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
        let value = {
            let compute_ctx = context.lock().unwrap().read("compute".to_string());
            if let Ok(value) = compute_ctx {
                value
            } else {
                context.lock().unwrap().read("setup".to_string()).unwrap()
            }
        };

        let data = match serde_json::from_value::<ComputePriceCtx>(value.clone()) {
            Ok(computer_ctx) => json!(ReportCtx {
                report_msg: format!("{}: {}", "some new data reported", computer_ctx.msg),
                report_value: computer_ctx.b
            }),
            Err(_) => {
                let setup_ctx: SetupCtx = serde_json::from_value(value).unwrap();
                json!(ReportCtx {
                    report_msg: format!("{}: {}", "some new data reported", setup_ctx.a),
                    report_value: setup_ctx.b
                })
            }
        };

        match context
            .lock()
            .as_mut()
            .unwrap()
            .write("report".to_string(), &data)
        {
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

#[derive(Serialize, Deserialize)]
pub struct OnChainValuesCtx {
    pub config: Config,
    pub c: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct ConfigState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

pub const CONFIG: &str = "config";

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
        let config = Config {
            a: "config_a".to_string(),
            b: "config_b".to_string(),
        };
        let c = "".to_string();
        let config_state_ctx = ConfigStateCtx { config, c };

        let data = serde_json::to_value(config_state_ctx).unwrap();

        match context
            .lock()
            .as_mut()
            .unwrap()
            .write(CONFIG.to_string(), &data)
        {
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
pub const ONCHAINVALUES: &str = "onchain_values";

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
        let value = context.lock().unwrap().read(CONFIG.to_string()).unwrap();
        let _data: ConfigStateCtx = serde_json::from_value(value).unwrap();

        let onchain_value_ctx = OnChainValuesCtx {
            config: _data.config,
            c: _data.c,
            values: vec!["txn1".to_string(), "txn2".to_string()],
        };

        let data = serde_json::to_value(onchain_value_ctx).unwrap();

        match context
            .lock()
            .as_mut()
            .unwrap()
            .write(ONCHAINVALUES.to_string(), &data)
        {
            Ok(()) => Ok(()),
            Err(e) => Err(StateError::StorageAccess(
                StateErrorRecoverability::Unrecoverable,
                e,
            )),
        }
    }
}
