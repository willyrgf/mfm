use crate::config::Config;
use crate::portfolio::{Portfolio, PortfolioOperation, PortfolioStatus};
use mfm_machine::state::Label;
use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    YamlFile(String),
}
pub const CONFIG_SOURCE_CTX: Label = Label("config_source_ctx");

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ConfigCtx {
    pub config_source: ConfigSource,
    pub config: Config,
}
pub const CONFIG_CTX: Label = Label("config_ctx");

// Portfolio Context for managing portfolio operations
pub mod portfolio {
    use super::*;
    use crate::portfolio::{PortfolioError, TokenBalance};
    use std::sync::{Arc, RwLock};

    pub const PORTFOLIO_CTX: Label = Label("portfolio_ctx");

    // Holds portfolio data in a thread-safe way
    #[derive(Clone, Debug)]
    pub struct PortfolioContext {
        inner: Arc<RwLock<PortfolioContextInner>>,
    }

    // Inner data of the portfolio context
    #[derive(Debug)]
    struct PortfolioContextInner {
        pub portfolio: Portfolio,
    }

    impl PortfolioContext {
        // Get the current portfolio status
        pub fn get_status(&self) -> PortfolioStatus {
            let inner = self.inner.read().unwrap();
            inner.portfolio.status.clone()
        }

        // Get the token balances
        pub fn get_balances(&self) -> Vec<TokenBalance> {
            let inner = self.inner.read().unwrap();
            inner.portfolio.state.balances.clone()
        }

        // Get the current operation
        pub fn get_operation(&self) -> Option<PortfolioOperation> {
            let inner = self.inner.read().unwrap();
            inner.portfolio.operation.clone()
        }

        // Get the wallet address from config
        pub fn get_wallet_address(&self) -> String {
            let inner = self.inner.read().unwrap();
            inner.portfolio.config.wallet.address.to_string()
        }
    }
}
