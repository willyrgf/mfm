use anyhow::anyhow;
use mfm_machine::state::context::{wrap_context, ContextWrapper, Local};
use mfm_machine::state::safe_context::SafeContext;
use mfm_machine::state::{
    standard_tags, DependencyStrategy, Label, StateError, StateErrorRecoverability, StateHandler,
    StateMetadata, StateResult, Tag,
};
use mfm_machine::state_machine::StateMachine;
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

#[derive(Debug, Serialize, Deserialize)]
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
            tags: vec![Tag::new("setup").unwrap(), standard_tags::CONFIG],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for Setup {
    #[allow(deprecated)]
    fn handler(&self, context: ContextWrapper) -> StateResult {
        self.handler_safe(SafeContext::from_wrapper(context))
    }

    fn handler_safe(&self, context: SafeContext) -> StateResult {
        let data = Config {
            a: "setup_b".to_string(),
            b: 1,
        };

        context
            .write_typed(CONFIG, &data)
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct ComputePrice {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComputePriceCtx {
    msg: String,
    b: u32,
}

impl Default for ComputePrice {
    fn default() -> Self {
        Self::new()
    }
}
impl ComputePrice {
    pub fn new() -> Self {
        Self {
            label: Label::new("compute_price_state").unwrap(),
            tags: vec![Tag::new("compute_price").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for ComputePrice {
    #[allow(deprecated)]
    fn handler(&self, context: ContextWrapper) -> StateResult {
        self.handler_safe(SafeContext::from_wrapper(context))
    }

    fn handler_safe(&self, context: SafeContext) -> StateResult {
        let config_data: Config = context
            .read_typed(CONFIG)
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

        let price_data = Config {
            a: format!("{}_compute_price", config_data.a),
            b: config_data.b * 2,
        };

        context
            .write_typed("price", &price_data)
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct Report {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

#[derive(Serialize, Deserialize)]
pub struct ReportCtx {
    pub report_msg: String,
    pub report_value: u32,
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
            tags: vec![Tag::new("report").unwrap(), standard_tags::REPORT],
            depends_on: vec![Tag::new("compute_price").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for Report {
    #[allow(deprecated)]
    fn handler(&self, context: ContextWrapper) -> StateResult {
        self.handler_safe(SafeContext::from_wrapper(context))
    }

    fn handler_safe(&self, context: SafeContext) -> StateResult {
        let price_data: Config = context
            .read_typed("price")
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

        let report_data = Config {
            a: format!("{}_report", price_data.a),
            b: price_data.b * 2,
        };

        context
            .write_typed("report", &report_data)
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
    }
}

// ---
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub a: String,
    pub b: u32,
}

pub const CONFIG: &str = "config";

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
            tags: vec![Tag::new("config").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for ConfigState {
    #[allow(deprecated)]
    fn handler(&self, context: ContextWrapper) -> StateResult {
        self.handler_safe(SafeContext::from_wrapper(context))
    }

    fn handler_safe(&self, _context: SafeContext) -> StateResult {
        Ok(())
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
            label: Label::new("on_chain_values_state").unwrap(),
            tags: vec![Tag::new("on_chain").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for OnChainValuesState {
    #[allow(deprecated)]
    fn handler(&self, context: ContextWrapper) -> StateResult {
        self.handler_safe(SafeContext::from_wrapper(context))
    }

    fn handler_safe(&self, _context: SafeContext) -> StateResult {
        Ok(())
    }
}
