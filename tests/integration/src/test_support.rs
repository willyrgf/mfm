#![warn(missing_docs)]
//! Shared helpers for MFM integration tests.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use mfm_core::keystore::{Keystore, KeystoreConfig};
use mfm_events::v1::{ArtifactRole, KernelEventPayload};
use mfm_fact_capabilities::{FactIndexReadCapability, FactIndexReadProvider, FactRecordCapability};
use mfm_store::v1 as store;
use mfm_store::v1::RetainedArtifactReadProvider;

#[path = "run_control_support.rs"]
mod run_control_support;

pub use run_control_support::{
    admit_portfolio_run_without_driving, assert_framework_started_before_terminal_evidence,
    prepare_entry_point_launch_for_store, prepare_portfolio_launch_for_store,
    set_evm_runtime_config_env_for_test, set_evm_runtime_config_env_with_signer_for_test,
    start_portfolio_rpc_mock, write_evm_runtime_config_for_test, EnvVarRestore,
    RuntimeConfigSignerBinding, ENV_RUNTIME_CONFIG_FILE,
};

/// Re-export: merge-safe Platform holding seed for store-backed portfolio report tests.
pub use store::test_support::{
    seed_platform_holding_facts_for_test, FactProjectionFixtureInputForTest,
    PlatformHoldingFactSeedForTest,
};

/// In-memory REST app state used by integration tests.
pub type InMemoryRestAppState = mfm_rest_api::AppState<store::AsyncInMemoryRunStore>;

/// Store-backed projection fact-index (shared with app process assembly tests).
pub use mfm_app::ProjectionFactIndexProvider;

/// Binds the shared fact capabilities owned by an integration-test process.
pub fn register_process_fact_capabilities(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    fact_index: &dyn FactIndexReadProvider,
) -> mfm_runtime::Result<()> {
    registry.register_capability_spec::<FactIndexReadCapability>(
        mfm_runtime::CapabilityImplementationId::new(fact_index.implementation_id())?,
    )?;
    registry.register_capability_spec::<FactRecordCapability>(
        mfm_runtime::CapabilityImplementationId::new("mfm.integration.managed-fact-record.v1")?,
    )
}

/// Builds in-memory REST app state.
pub fn in_memory_rest_app_state() -> InMemoryRestAppState {
    let store = store::AsyncInMemoryRunStore::default();
    let fact_index = Arc::new(ProjectionFactIndexProvider::new(store.clone()));
    let fact_query_receipt_trust_root = Some(fact_index.receipt_trust_root());
    mfm_rest_api::AppState {
        store,
        runtime_config_path: None,
        fact_query_receipt_trust_root,
        fact_query_authority_ready: true,
        fact_index,
    }
}

/// Loads all retained fact-query evidence artifacts referenced by `stream`.
pub async fn fact_query_evidences(
    store: &store::AsyncInMemoryRunStore,
    stream: &[store::KernelEventEnvelope],
) -> Vec<mfm_facts::FactQueryEvidence> {
    let mut evidences = Vec::new();
    for event in stream {
        let KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != ArtifactRole::FactQueryEvidence {
            continue;
        }
        let requirement = store::event_artifact_requirements(event.payload())
            .into_iter()
            .next()
            .expect("query evidence artifact requirement");
        let artifact = store
            .read_retained_artifact(&requirement)
            .await
            .expect("query evidence artifact");
        evidences.push(
            mfm_facts::parse_canonical_fact_query_evidence_bytes(artifact.bytes())
                .expect("query evidence bytes"),
        );
    }
    evidences
}

/// Builds a JSON POST request for REST integration tests.
pub fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let payload = serde_json::to_string(&body).expect("request body serializes");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .expect("request")
}

/// Builds an empty-body POST request for REST integration tests.
pub fn empty_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// Parses an Axum response body as JSON for REST integration tests.
pub async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("response json")
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

    /// Writes and selects runtime config for this wallet and one EVM route.
    pub fn set_runtime_config_env_for_test(
        &self,
        network_id: &str,
        endpoint_url: &str,
    ) -> EnvVarRestore {
        set_evm_runtime_config_env_with_signer_for_test(
            network_id,
            endpoint_url,
            Some(RuntimeConfigSignerBinding {
                signer_ref: "deployer",
                entry_id: &self.entry_id,
                keystore_path: &self.keystore_path,
                unlock_file: &self.password_file_path,
            }),
        )
    }

    /// Returns true when `rendered` exposes process-local signer registry details.
    pub fn rendered_contains_runtime_signer_config(&self, rendered: &str) -> bool {
        rendered.contains(&self.entry_id)
    }

    /// Returns true when `rendered` contains test-local secret-bearing file paths.
    pub fn rendered_contains_secret_path(&self, rendered: &str) -> bool {
        rendered.contains(&self.keystore_path.display().to_string())
            || rendered.contains(&self.password_file_path.display().to_string())
    }
}

impl Drop for FundedRethKeystoreWallet {
    fn drop(&mut self) {
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
