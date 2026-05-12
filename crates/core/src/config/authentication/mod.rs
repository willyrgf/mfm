//! Authentication-method configuration.
//!
//! The configuration file can advertise non-secret signer references. Runtime callers are expected
//! to hand these references to the keystore state/transport layer instead of loading secret material
//! from core configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Non-secret reference to a key stored in the local keystore.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeystoreReference {
    /// Optional keystore path override.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Optional exact keystore entry id.
    #[serde(default)]
    pub key_id: Option<String>,
    /// Optional keystore entry label selector.
    #[serde(default)]
    pub label: Option<String>,
}

/// Supported authentication methods for runtime wallet access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "config")]
pub enum Method {
    /// Use a key already imported into the local keystore.
    #[serde(rename = "keystore")]
    Keystore(KeystoreReference),
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

#[cfg(test)]
mod tests {
    use super::{KeystoreReference, Method, Methods};

    #[test]
    fn wallet_method_is_not_supported() {
        let raw = r#"
        - method: wallet
          wallet:
            private_key_path: /tmp/plaintext-key
            not_encrypted: true
        "#;

        let err = serde_yaml::from_str::<Methods>(raw).unwrap_err();
        assert!(err.to_string().contains("unknown variant"));
    }

    #[test]
    fn keystore_reference_is_non_secret_config() {
        let raw = r#"
        - method: keystore
          config:
            path: /tmp/mfm.keystore
            key_id: 00000000-0000-0000-0000-000000000001
        "#;

        let methods = serde_yaml::from_str::<Methods>(raw).unwrap();
        assert_eq!(
            methods.get_methods(),
            &vec![Method::Keystore(KeystoreReference {
                path: Some("/tmp/mfm.keystore".into()),
                key_id: Some("00000000-0000-0000-0000-000000000001".to_string()),
                label: None,
            })]
        );
    }
}
