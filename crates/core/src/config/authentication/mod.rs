//! Authentication-method configuration.
//!
//! The configuration file can advertise more than one wallet-loading strategy. Runtime callers are
//! expected to walk the configured methods in declaration order and choose the first supported
//! option.

/// Wallet-based authentication configuration.
pub mod wallet;

use self::wallet::Wallet;

use serde::{Deserialize, Serialize};

/// Supported authentication methods for runtime wallet access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "wallet")]
pub enum Method {
    /// Load a wallet from a local file configuration.
    #[serde(rename = "wallet")]
    Wallet(Wallet),
    /// Placeholder for MetaMask-driven authentication.
    #[serde(rename = "meta_mask")]
    MetaMask,
}

/// Ordered list of supported authentication methods.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Methods(Vec<Method>);

impl Methods {
    /// Returns the configured authentication methods in declaration order.
    ///
    /// Callers typically iterate this list and use the first method they support at runtime.
    pub fn get_methods(&self) -> &Vec<Method> {
        &self.0
    }
}
