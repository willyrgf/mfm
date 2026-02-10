use anyhow::anyhow;
use mfm_machine::state::safe_context::SafeContext;
use mfm_machine::state::{
    DependencyStrategy, Label, StateError, StateErrorRecoverability, StateHandler, StateMetadata,
    StateResult, Tag,
};
use mfm_machine_derive::StateMetadataReqs;
use rand::Rng;
use serde_derive::{Deserialize, Serialize};

#[mfm_machine_derive::state_handler(
    label = "setup_state",
    tags = ["setup", "config"],
    depends_on = [],
    strategy = Latest
)]
#[derive(Debug, Clone, PartialEq)]
pub struct Setup;

#[async_trait::async_trait]
impl StateHandler for Setup {
    async fn handler(&self, context: SafeContext) -> StateResult {
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

#[async_trait::async_trait]
impl StateHandler for ComputePrice {
    async fn handler(&self, context: SafeContext) -> StateResult {
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

#[mfm_machine_derive::state_handler(
    label = "report_state",
    tags = ["report"],
    depends_on = ["compute_price"],
    strategy = Latest
)]
#[derive(Debug, Clone, PartialEq)]
pub struct Report;

#[async_trait::async_trait]
impl StateHandler for Report {
    async fn handler(&self, context: SafeContext) -> StateResult {
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

#[async_trait::async_trait]
impl StateHandler for ConfigState {
    async fn handler(&self, _context: SafeContext) -> StateResult {
        // Always succeed
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

#[async_trait::async_trait]
impl StateHandler for OnChainValuesState {
    async fn handler(&self, _context: SafeContext) -> StateResult {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
#[allow(dead_code)]
pub struct ValidationState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for ValidationState {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationState {
    pub fn new() -> Self {
        Self {
            label: Label::new("validation_state").unwrap(),
            tags: vec![Tag::new("validation").unwrap()],
            depends_on: vec![Tag::new("compute_price").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

#[async_trait::async_trait]
impl StateHandler for ValidationState {
    async fn handler(&self, context: SafeContext) -> StateResult {
        let price_data: Config = context
            .read_typed("price")
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

        // Validate the price calculations
        if price_data.b < 2 {
            return Err(StateError::Unknown(
                StateErrorRecoverability::Recoverable,
                anyhow!("Price value too low"),
            ));
        }

        let validation_data = Config {
            a: format!("{}_validated", price_data.a),
            b: price_data.b,
        };

        context
            .write_typed("validation", &validation_data)
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
#[allow(dead_code)]
pub struct NotificationState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for NotificationState {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationState {
    pub fn new() -> Self {
        Self {
            label: Label::new("notification_state").unwrap(),
            tags: vec![Tag::new("notification").unwrap()],
            depends_on: vec![Tag::new("report").unwrap(), Tag::new("validation").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

#[async_trait::async_trait]
impl StateHandler for NotificationState {
    async fn handler(&self, context: SafeContext) -> StateResult {
        let report_data: Config = context
            .read_typed("report")
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

        let validation_data: Config = context
            .read_typed("validation")
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

        let notification_data = Config {
            a: format!("{}_notified_{}", report_data.a, validation_data.a),
            b: report_data.b + 10,
        };

        context
            .write_typed("notification", &notification_data)
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
#[allow(dead_code)]
pub struct AnalyticsState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for AnalyticsState {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalyticsState {
    pub fn new() -> Self {
        Self {
            label: Label::new("analytics_state").unwrap(),
            tags: vec![Tag::new("analytics").unwrap()],
            depends_on: vec![
                Tag::new("report").unwrap(),
                Tag::new("notification").unwrap(),
            ],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

#[async_trait::async_trait]
impl StateHandler for AnalyticsState {
    async fn handler(&self, context: SafeContext) -> StateResult {
        // Generate random analytics data
        let mut rng = rand::rng();
        let analytics_value = rng.random_range(100..1000);

        let analytics_data = Config {
            a: "analytics_processed".to_string(),
            b: analytics_value,
        };

        context
            .write_typed("analytics", &analytics_data)
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
#[allow(dead_code)]
pub struct FinalizeState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl Default for FinalizeState {
    fn default() -> Self {
        Self::new()
    }
}

impl FinalizeState {
    pub fn new() -> Self {
        Self {
            label: Label::new("finalize_state").unwrap(),
            tags: vec![Tag::new("finalize").unwrap()],
            depends_on: vec![
                Tag::new("report").unwrap(),
                Tag::new("notification").unwrap(),
                Tag::new("analytics").unwrap(),
            ],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

#[async_trait::async_trait]
impl StateHandler for FinalizeState {
    async fn handler(&self, context: SafeContext) -> StateResult {
        let report_data: Config = context
            .read_typed("report")
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

        let analytics_data: Config = context
            .read_typed("analytics")
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

        let notification_data: Config = context
            .read_typed("notification")
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

        let finalize_data = Config {
            a: "finalized_workflow".to_string(),
            b: report_data.b + analytics_data.b + notification_data.b,
        };

        context
            .write_typed("finalized", &finalize_data)
            .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
    }
}
