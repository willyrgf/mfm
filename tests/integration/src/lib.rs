#![warn(missing_docs)]
//! Shared workspace integration and parity test helpers.
//!
//! This crate keeps cross-workspace test plumbing in one place so parity suites can coordinate
//! run ids and shared setup without duplicating helper code in every test binary.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_integration_tests::parity_run_ids::read_parity_evm_run_id;
//!
//! let _run_id = read_parity_evm_run_id();
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mfm_collectors_evm_jsonrpc_http::EvmSourceKind;
use mfm_collectors_rpc_control::{EvmIoClient, JsonRpcCall, DEFAULT_CONTROL_SCOPE};
use mfm_machine::engine::Stores;
use mfm_machine::errors::IoError;
use mfm_machine::ids::{RunId, StateId};
use mfm_machine::live_io::{
    FactIndex, LiveIo, LiveIoEnv, LiveIoTransportFactory, NoopFactRecorder,
};
use mfm_machine::stores::{ArtifactStore, StreamStore};
use mfm_transports_rpc_control::{
    resolve_rpc_control_bootstrap_sources_from_env, RpcControlBootstrapSource,
    RpcControlPlaneStorageMode, RpcControlTransportFactory,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

const RPC_CONTROL_HELPER_MAX_ATTEMPTS: usize = 5;
const RPC_CONTROL_HELPER_RETRY_DELAY_MS: u64 = 200;

fn io_error_retryable(err: &IoError) -> bool {
    match err {
        IoError::MissingFactKey(info)
        | IoError::Transport(info)
        | IoError::RateLimited(info)
        | IoError::Other(info) => info.retryable,
        IoError::MissingFact { info, .. } => info.retryable,
    }
}

/// Helpers for persisting parity run ids between coordinated integration-test phases.
pub mod parity_run_ids {
    use super::*;

    /// Environment variable that points to the handoff file for EVM parity run ids.
    pub const PARITY_EVM_RUN_IDS_PATH_ENV: &str = "MFM_PARITY_EVM_RETH_RUN_IDS_PATH";
    /// Environment variable that points to the handoff file for Aave parity run ids.
    pub const PARITY_AAVE_RUN_IDS_PATH_ENV: &str = "MFM_PARITY_AAVE_V3_RUN_IDS_PATH";

    const PARITY_EVM_RUN_IDS_KIND: &str = "parity_evm_reth_run_ids_v1";
    const PARITY_AAVE_RUN_IDS_KIND: &str = "parity_aave_v3_run_ids_v1";

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ParityEvmRunIds {
        kind: String,
        evm_run_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ParityAaveRunIds {
        kind: String,
        phase_a_run_id: String,
        phase_b_run_id: String,
    }

    /// Writes a single EVM parity run id to the configured handoff file.
    pub fn write_parity_evm_run_id(run_id: &RunId) {
        let path = required_env_path(PARITY_EVM_RUN_IDS_PATH_ENV);
        let payload = ParityEvmRunIds {
            kind: PARITY_EVM_RUN_IDS_KIND.to_string(),
            evm_run_id: run_id.0.to_string(),
        };
        write_json_atomic(&path, &payload, "evm parity run ids");
    }

    /// Writes the phase A and phase B Aave parity run ids to the configured handoff file.
    pub fn write_parity_aave_run_ids(phase_a_run_id: &RunId, phase_b_run_id: &RunId) {
        let path = required_env_path(PARITY_AAVE_RUN_IDS_PATH_ENV);
        let payload = ParityAaveRunIds {
            kind: PARITY_AAVE_RUN_IDS_KIND.to_string(),
            phase_a_run_id: phase_a_run_id.0.to_string(),
            phase_b_run_id: phase_b_run_id.0.to_string(),
        };
        write_json_atomic(&path, &payload, "aave parity run ids");
    }

    /// Reads the EVM parity run id from the configured handoff file.
    pub fn read_parity_evm_run_id() -> RunId {
        let path = required_env_path(PARITY_EVM_RUN_IDS_PATH_ENV);
        let payload: ParityEvmRunIds = read_json(&path, "evm parity run ids");
        assert_eq!(
            payload.kind,
            PARITY_EVM_RUN_IDS_KIND,
            "unexpected kind for evm parity run ids file `{}`",
            path.display()
        );
        parse_run_id("evm_run_id", &payload.evm_run_id)
    }

    /// Reads the phase A and phase B Aave parity run ids from the configured handoff file.
    pub fn read_parity_aave_run_ids() -> (RunId, RunId) {
        let path = required_env_path(PARITY_AAVE_RUN_IDS_PATH_ENV);
        let payload: ParityAaveRunIds = read_json(&path, "aave parity run ids");
        assert_eq!(
            payload.kind,
            PARITY_AAVE_RUN_IDS_KIND,
            "unexpected kind for aave parity run ids file `{}`",
            path.display()
        );
        let phase_a_run_id = parse_run_id("phase_a_run_id", &payload.phase_a_run_id);
        let phase_b_run_id = parse_run_id("phase_b_run_id", &payload.phase_b_run_id);
        (phase_a_run_id, phase_b_run_id)
    }

    #[allow(clippy::disallowed_methods)]
    fn required_env_path(env_var: &str) -> PathBuf {
        let path = std::env::var(env_var)
            .unwrap_or_else(|_| panic!("`{env_var}` must be set for parity run-id handoff"));
        PathBuf::from(path)
    }

    fn write_json_atomic<T: Serialize>(path: &Path, value: &T, label: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|err| {
                panic!(
                    "failed creating parent directory `{}` for {label}: {err}",
                    parent.display()
                )
            });
        }

        let bytes = serde_json::to_vec_pretty(value)
            .unwrap_or_else(|err| panic!("failed serializing {label}: {err}"));
        let tmp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));

        std::fs::write(&tmp_path, bytes).unwrap_or_else(|err| {
            panic!(
                "failed writing temp file `{}` for {label}: {err}",
                tmp_path.display()
            )
        });

        std::fs::rename(&tmp_path, path).unwrap_or_else(|err| {
            panic!(
                "failed moving temp file `{}` to `{}` for {label}: {err}",
                tmp_path.display(),
                path.display()
            )
        });
    }

    fn read_json<T: DeserializeOwned>(path: &Path, label: &str) -> T {
        let bytes = std::fs::read(path).unwrap_or_else(|err| {
            panic!("failed reading {label} file `{}`: {err}", path.display())
        });
        serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            panic!("failed decoding {label} file `{}`: {err}", path.display())
        })
    }

    fn parse_run_id(field: &str, value: &str) -> RunId {
        let parsed = uuid::Uuid::parse_str(value)
            .unwrap_or_else(|err| panic!("invalid uuid in `{field}` (`{value}`): {err}"));
        RunId(parsed)
    }
}

/// Shared helpers for tests that should exercise the managed RPC control plane.
pub mod rpc_control {
    use super::*;

    const DEFAULT_PARITY_RETH_HTTP_PORT: &str = "8565";
    const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";

    fn single_source(
        id: &str,
        network_id: &str,
        rpc_url: &str,
        kind: EvmSourceKind,
    ) -> RpcControlBootstrapSource {
        RpcControlBootstrapSource {
            id: id.to_string(),
            network_id: Some(network_id.to_string()),
            rpc_url: rpc_url.to_string(),
            authorization: None,
            kind,
            require_get_proof_probe: false,
        }
    }

    fn source_kind_name(kind: EvmSourceKind) -> &'static str {
        match kind {
            EvmSourceKind::Local => "local",
            EvmSourceKind::RemoteUser => "remote_user",
            EvmSourceKind::RemotePublic => "remote_public",
        }
    }

    /// Builds a control-plane source using a single remote-user RPC endpoint.
    pub fn single_remote_user_source(
        id: &str,
        network_id: &str,
        rpc_url: &str,
    ) -> RpcControlBootstrapSource {
        single_source(id, network_id, rpc_url, EvmSourceKind::RemoteUser)
    }

    fn default_local_reth_sources_from_env() -> Vec<RpcControlBootstrapSource> {
        let port = std::env::var("RETH_HTTP_PORT")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_PARITY_RETH_HTTP_PORT.to_string());
        if port.is_empty() {
            return Vec::new();
        }

        let rpc_url = format!("http://127.0.0.1:{port}");
        vec![
            single_source(
                "reth_ethereum_mainnet",
                "ethereum-mainnet",
                &rpc_url,
                EvmSourceKind::Local,
            ),
            single_source("reth_local", "reth-local", &rpc_url, EvmSourceKind::Local),
        ]
    }

    /// Loads bootstrap sources from the canonical `rpc.control` env surface.
    ///
    /// Parity CI runs a local Reth service on a fixed port; when the JSON bootstrap catalog is
    /// unexpectedly missing in the test process, synthesize the canonical two-network local
    /// source catalog so parity suites still exercise managed routing through explicit source
    /// declarations.
    pub fn bootstrap_sources_from_env() -> Vec<RpcControlBootstrapSource> {
        let sources = resolve_rpc_control_bootstrap_sources_from_env();
        if !sources.is_empty() {
            return sources;
        }
        let fallback = default_local_reth_sources_from_env();
        if !fallback.is_empty() {
            let raw_json = serde_json::Value::Array(
                fallback
                    .iter()
                    .map(|source| {
                        serde_json::json!({
                            "id": source.id,
                            "network_id": source.network_id,
                            "rpc_url": source.rpc_url,
                            "authorization": source.authorization,
                            "kind": source_kind_name(source.kind),
                            "require_get_proof_probe": source.require_get_proof_probe,
                        })
                    })
                    .collect(),
            )
            .to_string();
            std::env::set_var(ENV_EVM_RPC_SOURCES_JSON, raw_json);
        }
        fallback
    }

    /// Loads bootstrap sources from env and asserts at least one source exists for `network_id`.
    pub fn required_bootstrap_sources_from_env_for_network(
        network_id: &str,
    ) -> Vec<RpcControlBootstrapSource> {
        let sources = bootstrap_sources_from_env();
        assert!(
            !sources.is_empty(),
            "MFM_EVM_RPC_SOURCES_JSON must include at least one bootstrap source"
        );
        assert!(
            sources
                .iter()
                .any(|source| source.network_id.as_deref() == Some(network_id)),
            "MFM_EVM_RPC_SOURCES_JSON must include at least one source for network `{network_id}`"
        );
        sources
    }

    /// Returns a single-source control transport for a specific state and run.
    pub fn transport_for_state(
        streams: Arc<dyn StreamStore>,
        artifacts: Arc<dyn ArtifactStore>,
        run_id: RunId,
        state_id: StateId,
        sources: Vec<RpcControlBootstrapSource>,
    ) -> Box<dyn mfm_machine::live_io::LiveIoTransport> {
        let factory = RpcControlTransportFactory::new(sources)
            .with_control_plane_storage_mode(RpcControlPlaneStorageMode::StreamStore);
        let env = LiveIoEnv {
            stores: Stores { streams, artifacts },
            run_id,
            state_id,
            attempt: 0,
        };
        factory.make(env)
    }

    /// Executes a managed RPC call through `rpc.control` and returns the response payload.
    pub async fn call(
        sources: &[RpcControlBootstrapSource],
        network_id: &str,
        streams: Arc<dyn StreamStore>,
        artifacts: Arc<dyn ArtifactStore>,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        call_in_scope(
            sources,
            network_id,
            DEFAULT_CONTROL_SCOPE,
            streams,
            artifacts,
            method,
            params,
        )
        .await
    }

    /// Executes a managed RPC call through `rpc.control` within an explicit control scope.
    pub async fn call_in_scope(
        sources: &[RpcControlBootstrapSource],
        network_id: &str,
        control_scope: &str,
        streams: Arc<dyn StreamStore>,
        artifacts: Arc<dyn ArtifactStore>,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        for attempt in 0..RPC_CONTROL_HELPER_MAX_ATTEMPTS {
            let state_id = StateId::must_new("rpc_control.integration.helper_call".to_string());
            let run_id = RunId(uuid::Uuid::new_v4());
            let stream_store = Arc::clone(&streams);
            let artifacts_store = Arc::clone(&artifacts);
            let transport = transport_for_state(
                stream_store,
                artifacts_store,
                run_id,
                state_id.clone(),
                sources.to_vec(),
            );
            let mut live = LiveIo::new(
                run_id,
                state_id.clone(),
                0,
                Arc::clone(&artifacts),
                FactIndex::default(),
                Arc::new(NoopFactRecorder),
                transport,
            );
            let mut client = EvmIoClient::new(state_id, &mut live);
            match client
                .call(JsonRpcCall::for_scope_and_network(
                    control_scope,
                    network_id,
                    method,
                    params.clone(),
                ))
                .await
            {
                Ok(response) => return response.response,
                Err(err)
                    if attempt + 1 < RPC_CONTROL_HELPER_MAX_ATTEMPTS
                        && io_error_retryable(&err) =>
                {
                    std::thread::sleep(Duration::from_millis(RPC_CONTROL_HELPER_RETRY_DELAY_MS));
                }
                Err(err) => {
                    panic!("rpc.control call failed: {err:?}");
                }
            }
        }

        unreachable!("rpc.control retry loop must return or panic")
    }
}
