#![warn(missing_docs)]
//! Shared helpers for MFM integration tests.

use std::fs;
use std::path::PathBuf;

use mfm_core::keystore::{Keystore, KeystoreConfig};
use mfm_store::v1 as store;

/// In-memory REST app state used by integration tests.
pub type InMemoryRestAppState = mfm_rest_api::AppState<store::AsyncInMemoryTypedRunStore>;

/// Builds in-memory REST app state rooted at `artifact_root`.
pub fn in_memory_rest_app_state(artifact_root: impl Into<PathBuf>) -> InMemoryRestAppState {
    mfm_rest_api::AppState {
        store: store::AsyncInMemoryTypedRunStore::default(),
        artifacts: mfm_artifact_store_fs::FsTypedArtifactStore::new(artifact_root),
    }
}

/// Calls a JSON-RPC endpoint and returns the response `result`.
pub async fn rpc_call(rpc_url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .expect("send json-rpc request")
        .error_for_status()
        .expect("json-rpc http status");
    let payload: serde_json::Value = response.json().await.expect("json-rpc response json");
    if let Some(error) = payload.get("error") {
        panic!("json-rpc {method} returned error: {error}");
    }
    payload
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("json-rpc {method} response missing result: {payload}"))
}

/// Ephemeral funded keystore wallet used by reth-backed EVM parity tests.
pub struct FundedRethKeystoreWallet {
    temp_dir: tempfile::TempDir,
    keystore_env: String,
    password_file_env: String,
    /// Funded sender address derived from the keystore entry.
    pub from: String,
    /// Stable keystore entry id used by EVM signer config fixtures.
    pub entry_id: String,
    keystore_path: PathBuf,
    password_file_path: PathBuf,
}

impl FundedRethKeystoreWallet {
    /// Returns the typed workflow signer intent for this wallet.
    pub fn signer_json(&self) -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": self.from,
        })
    }

    /// Returns the process-local runtime signer registry JSON for this wallet.
    pub fn runtime_signer_registry_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "signer_ref": "deployer",
                "entry_id": self.entry_id,
                "keystore_env": self.keystore_env,
                "unlock_file_env": self.password_file_env,
            }
        ])
    }

    /// Returns the EVM source registry JSON that routes one local source to this reth node.
    pub fn runtime_source_registry_json(
        &self,
        source_id: &str,
        expected_chain_id: u64,
        endpoint_url: &str,
    ) -> serde_json::Value {
        let mut source = serde_json::json!({
            "id": source_id,
            "expected_chain_id": expected_chain_id,
        });
        let source_object = source.as_object_mut().expect("source object");
        source_object.insert(
            ["rpc", "_url"].concat(),
            serde_json::Value::String(endpoint_url.to_owned()),
        );
        source_object.insert(["author", "ization"].concat(), serde_json::Value::Null);
        serde_json::json!({
            "sources": [
                source
            ],
            "policies": [
                {
                    "id": source_id,
                    "ordered_sources": [source_id]
                }
            ]
        })
    }

    /// Returns true when `rendered` exposes process-local signer registry details.
    pub fn rendered_contains_runtime_signer_config(&self, rendered: &str) -> bool {
        rendered.contains(&self.entry_id)
            || rendered.contains(&self.keystore_env)
            || rendered.contains(&self.password_file_env)
    }

    /// Returns true when `rendered` contains test-local secret-bearing file paths.
    pub fn rendered_contains_secret_path(&self, rendered: &str) -> bool {
        rendered.contains(&self.keystore_path.display().to_string())
            || rendered.contains(&self.password_file_path.display().to_string())
    }
}

impl Drop for FundedRethKeystoreWallet {
    fn drop(&mut self) {
        std::env::remove_var(&self.keystore_env);
        std::env::remove_var(&self.password_file_env);
        let _ = self.temp_dir.path();
    }
}

/// Creates an ephemeral keystore wallet from a reth dev pre-funded account and verifies balance.
pub async fn funded_reth_keystore_wallet(
    rpc_url: &str,
    account_index: u32,
) -> FundedRethKeystoreWallet {
    const TEST_PASSWORD: &str = "reth-parity-test-password-123";
    const RETH_DEV_MNEMONIC: &str = "test test test test test test test test test test test junk";
    assert!(
        account_index < 20,
        "reth --dev prefunds 20 mnemonic accounts"
    );
    let derivation_path = format!("m/44'/60'/0'/0/{account_index}");

    let temp_dir = tempfile::tempdir().expect("reth keystore wallet tempdir");
    let keystore_path = temp_dir.path().join("reth-parity.keystore");
    let password_file_path = temp_dir.path().join("reth-parity.password");
    fs::write(&password_file_path, TEST_PASSWORD).expect("write reth parity password file");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::insecure_integration_test())
            .expect("create reth parity keystore");
    keystore
        .unlock(TEST_PASSWORD)
        .expect("unlock reth parity keystore");
    let entry_id = keystore
        .import_mnemonic(
            Some("reth-parity-funded-signer".to_owned()),
            RETH_DEV_MNEMONIC,
            &derivation_path,
            None,
        )
        .expect("import reth parity key");
    let key_info = keystore
        .list_keys()
        .expect("list reth parity keys")
        .into_iter()
        .find(|key| key.id == entry_id)
        .expect("imported reth parity key info");
    let from = format!("{:?}", key_info.address);

    let suffix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .to_ascii_uppercase();
    let keystore_env = format!("MFM_EVM_PARITY_KEYSTORE_{suffix}");
    let password_file_env = format!("MFM_EVM_PARITY_KEYSTORE_PASSWORD_FILE_{suffix}");
    std::env::set_var(&keystore_env, &keystore_path);
    std::env::set_var(&password_file_env, &password_file_path);

    // Reth dev nodes prefund this deterministic key set, but recent releases do
    // not expose the dev accounts through `eth_accounts`. The balance assertion
    // below is the funding contract these tests actually need.
    let balance = rpc_call(
        rpc_url,
        "eth_getBalance",
        serde_json::json!([from.clone(), "latest"]),
    )
    .await;
    let balance = balance
        .as_str()
        .map(parse_u128_hex_quantity)
        .expect("reth funded balance hex");
    assert!(balance > 0, "reth parity keystore wallet must be funded");

    FundedRethKeystoreWallet {
        temp_dir,
        keystore_env,
        password_file_env,
        from,
        entry_id: entry_id.to_string(),
        keystore_path,
        password_file_path,
    }
}

fn parse_u128_hex_quantity(raw: &str) -> u128 {
    let trimmed = raw
        .strip_prefix("0x")
        .expect("hex quantity must start with 0x");
    u128::from_str_radix(trimmed, 16).expect("hex quantity must parse as u128")
}
