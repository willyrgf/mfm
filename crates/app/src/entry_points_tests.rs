use super::*;
use crate::{OpVersion, PublicOpName};
use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};
use mfm_store::v1::{
    self as store, AdmissionToken, ExecutionClaimStatus, ExecutionClaimStore,
    NowaitSkipAdmissionResult, RunEventStore, StoreScopeStore,
};
use std::sync::Arc;

fn portfolio_public_op_name() -> PublicOpName {
    PublicOpName::new(mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT.public_name)
        .expect("name")
}

fn portfolio_op(registry: &EntryPointOpRegistry) -> Arc<dyn crate::LaunchableOp> {
    registry
        .resolve_latest(&portfolio_public_op_name())
        .expect("portfolio op")
}

#[path = "entry_points_tests/launch.rs"]
mod launch;
#[path = "entry_points_tests/registry.rs"]
mod registry;

struct EntryPointPrepFixture {
    entry_point_registry: EntryPointOpRegistry,
    certification_registry: mfm_certify::CertificationRegistry,
    store_scope_id: mfm_ids::StoreScopeId,
}

impl EntryPointPrepFixture {
    fn with_store_scope_hex(hex: &str) -> Self {
        Self::with_store_scope_id(
            mfm_ids::StoreScopeId::new(format!("mfm.store_scope.v1:{hex}")).expect("store scope"),
        )
    }

    fn with_store_scope_id(store_scope_id: mfm_ids::StoreScopeId) -> Self {
        Self {
            entry_point_registry: production_entry_point_op_registry().expect("registry"),
            certification_registry: crate::production_certification_registry().expect("cert"),
            store_scope_id,
        }
    }

    fn prepare_portfolio_launch(
        &self,
        config: String,
        invocation_key: Option<crate::InvocationKey>,
    ) -> crate::PreparedEntryPointRunLaunch {
        crate::prepare_entry_point_run_launch(crate::EntryPointRunLaunchInput {
            entry_point_registry: &self.entry_point_registry,
            public_op_name: portfolio_public_op_name(),
            op_version: None,
            authored_config: AuthoredConfig::new(AuthoredConfigFormat::Json, config)
                .expect("authored config"),
            certification_registry: &self.certification_registry,
            store_scope_id: self.store_scope_id.clone(),
            invocation_key,
        })
        .expect("prepared entry-point launch")
    }

    fn prepare_sample_portfolio(
        &self,
        invocation_key: Option<crate::InvocationKey>,
    ) -> crate::PreparedEntryPointRunLaunch {
        self.prepare_portfolio_launch(sample_portfolio_config_json(), invocation_key)
    }

    fn prepare_bitcoin_portfolio(
        &self,
        invocation_key: Option<crate::InvocationKey>,
    ) -> crate::PreparedEntryPointRunLaunch {
        self.prepare_portfolio_launch(sample_bitcoin_portfolio_config_json(), invocation_key)
    }
}

struct EntryPointRunFixture {
    prep: EntryPointPrepFixture,
    store: mfm_store::v1::AsyncInMemoryRunStore,
    _runtime_config_dir: tempfile::TempDir,
    runtime_config_path: std::path::PathBuf,
}

impl EntryPointRunFixture {
    async fn in_memory() -> Self {
        let store = mfm_store::v1::AsyncInMemoryRunStore::default();
        let store_scope_id = store.load_store_scope_id().await.expect("store scope");
        let (runtime_config_dir, runtime_config_path) = test_runtime_config();
        Self {
            prep: EntryPointPrepFixture::with_store_scope_id(store_scope_id),
            store,
            _runtime_config_dir: runtime_config_dir,
            runtime_config_path,
        }
    }

    fn prepare_sample_portfolio(
        &self,
        invocation_key: Option<crate::InvocationKey>,
    ) -> crate::PreparedEntryPointRunLaunch {
        self.prep.prepare_sample_portfolio(invocation_key)
    }

    fn prepare_bitcoin_portfolio(
        &self,
        invocation_key: Option<crate::InvocationKey>,
    ) -> crate::PreparedEntryPointRunLaunch {
        self.prep.prepare_bitcoin_portfolio(invocation_key)
    }

    fn services(
        &self,
    ) -> crate::RunServices<
        mfm_store::v1::AsyncInMemoryRunStore,
        mfm_store::v1::AsyncInMemoryRunStore,
    > {
        let fact_index = crate::ProjectionFactIndexProvider::new(self.store.clone());
        let runners = crate::production_runner_registry(
            Arc::new(self.store.clone()),
            Arc::new(fact_index),
            Some(&self.runtime_config_path),
        )
        .expect("runners");
        crate::make_run_services(
            runners,
            self.store.clone(),
            self.store.clone(),
            self.prep.certification_registry.clone(),
        )
    }

    async fn run_admitted_count(&self, run_id: &mfm_ids::RunId) -> usize {
        self.store
            .load_run_stream(run_id)
            .await
            .expect("run stream")
            .into_iter()
            .filter(|event| {
                matches!(
                    event.payload(),
                    mfm_events::v1::KernelEventPayload::RunAdmitted(_)
                )
            })
            .count()
    }
}

async fn admit_entry_point_run(
    services: &crate::RunServices<
        mfm_store::v1::AsyncInMemoryRunStore,
        mfm_store::v1::AsyncInMemoryRunStore,
    >,
    prepared: crate::PreparedEntryPointRunLaunch,
) -> mfm_ids::RunId {
    let request = prepared.request;
    let run_id = request.run_id.clone();
    let runtime_spec = mfm_runtime::CertifiedRuntimeSpec::new(request.certified_spec.clone())
        .expect("runtime spec");
    let expected_next_seq = services
        .store()
        .expected_next_seq(&run_id)
        .await
        .expect("expected next seq");
    let launch = services
        .scheduler
        .prepare_run_launch(
            &runtime_spec,
            request.identity_material,
            request.evidence,
            expected_next_seq,
        )
        .expect("prepare admitted-only launch");
    services
        .scheduler
        .start_run(services.store(), launch)
        .await
        .expect("admit run");
    run_id
}

fn test_runtime_config() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("runtime config tempdir");
    let path = dir.path().join("runtime.toml");
    std::fs::write(
        &path,
        r#"
[evm.sources."ethereum-mainnet"]
rpc_url = "http://127.0.0.1:1"

[evm.routes."ethereum-mainnet"]
source_ref = "ethereum-mainnet"
"#,
    )
    .expect("write runtime config");
    (dir, path)
}

fn sample_portfolio_config_json() -> String {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "chain_id": 1,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_main",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
                    "implementation": { "kind": "address_only" },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "unit_price_dec": "1800.00"
                            }
                        ]
                    },
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        }
    })
    .to_string()
}

fn sample_bitcoin_portfolio_config_json() -> String {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "portfolio_btc",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "bitcoin-mainnet",
                    "family": "bitcoin",
                    "bitcoin_network": "main",
                    "source_identity": "public-bitcoin-core",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_btc_mainnet",
                    "subject": {
                        "kind": "bitcoin_address",
                        "address": "bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw"
                    },
                    "implementation": { "kind": "address_only" },
                    "network_id": "bitcoin-mainnet",
                    "symbol_ids": ["btc.native.bitcoin-mainnet"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "btc.native.bitcoin-mainnet",
                    "display_symbol": "BTC",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "bitcoin-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "btc.native.bitcoin-mainnet",
                                "unit_price_dec": "0.00"
                            }
                        ]
                    },
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        }
    })
    .to_string()
}

fn content_digest_str(byte: u8) -> String {
    format!("content:sha256-jcs-v1:{}", format!("{byte:02x}").repeat(32))
}

fn context_json() -> serde_json::Value {
    serde_json::json!({
        "lifecycle_key": "app-entry-test-lifecycle",
        "network": {
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "chain_fingerprint": null,
            "finality_or_observation_policy": null,
        },
        "contract_profile": {
            "profile_id": "app-entry-test-contract",
            "artifact_digest": content_digest_str(0x20),
            "interface_digest": content_digest_str(0x21),
            "creation_bytecode_digest": null,
            "deployed_code_hash": null,
            "selector_event_policy_digest": null,
        },
    })
}

fn signer_json() -> serde_json::Value {
    serde_json::json!({
        "signer_ref": "deployer",
        "expected_signer_address": "0x000000000000000000000000000000000000dead",
    })
}

fn deploy_action_json() -> serde_json::Value {
    serde_json::json!({
        "signer": signer_json(),
    })
}

fn configure_action_json() -> serde_json::Value {
    serde_json::json!({
        "signer": signer_json(),
        "calls": [],
    })
}

fn validate_action_json() -> serde_json::Value {
    serde_json::json!({})
}

fn deploy_config_json() -> serde_json::Value {
    serde_json::json!({
        "context": context_json(),
        "deploy": deploy_action_json(),
    })
}

fn lifecycle_config_json() -> serde_json::Value {
    serde_json::json!({
        "context": context_json(),
        "deploy": deploy_action_json(),
        "configure": configure_action_json(),
        "validate": validate_action_json(),
    })
}

fn import_deployed_json() -> serde_json::Value {
    serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000dead",
            "provenance_label": "app-entry-test-external",
            "evidence_policy": {
                "require_code": false
            }
        },
    })
}

fn import_configured_json() -> serde_json::Value {
    serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000dead",
            "provenance_label": "app-entry-test-external",
            "evidence_policy": {
                "require_code": false,
                "allow_external_claimed_configured": true
            }
        },
    })
}

fn configure_entry_config_json() -> serde_json::Value {
    serde_json::json!({
        "context": context_json(),
        "import_deployed": import_deployed_json(),
        "configure": configure_action_json(),
    })
}

fn validate_entry_config_json() -> serde_json::Value {
    serde_json::json!({
        "context": context_json(),
        "import_configured": import_configured_json(),
        "validate": validate_action_json(),
    })
}
