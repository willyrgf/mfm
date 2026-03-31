use super::*;
use async_trait::async_trait;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig, RunManifest,
};
use mfm_machine::engine::Stores;
use mfm_machine::errors::StorageError;
use mfm_machine::events::{Event, EventEnvelope, KernelEvent};
use mfm_machine::ids::{ArtifactId, OpId, RunId, StateId};
use mfm_machine::stores::{
    AppendBatchResult, ArtifactKind, ArtifactStore, StreamAppend, StreamId, StreamRecord,
    StreamStore,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
struct NoopStreamStore;

#[async_trait]
impl StreamStore for NoopStreamStore {
    async fn head_seq(&self, _stream_id: &StreamId) -> Result<u64, StorageError> {
        Ok(0)
    }
    async fn append(&self, _append: StreamAppend) -> Result<u64, StorageError> {
        Ok(0)
    }
    async fn append_batch(
        &self,
        _appends: Vec<StreamAppend>,
    ) -> Result<AppendBatchResult, StorageError> {
        Ok(AppendBatchResult {
            stream_heads: Vec::new(),
        })
    }
    async fn read_range(
        &self,
        _stream_id: &StreamId,
        _from_seq: u64,
        _to_seq: Option<u64>,
    ) -> Result<Vec<StreamRecord>, StorageError> {
        Ok(Vec::new())
    }
}

#[derive(Clone)]
struct NoopArtifactStore;

#[async_trait]
impl ArtifactStore for NoopArtifactStore {
    async fn put(&self, _kind: ArtifactKind, _bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        Ok(ArtifactId("0".repeat(64)))
    }
    async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
        Ok(Vec::new())
    }
    async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
        Ok(false)
    }
}

fn env() -> LiveIoEnv {
    LiveIoEnv {
        stores: Stores {
            streams: Arc::new(NoopStreamStore),
            artifacts: Arc::new(NoopArtifactStore),
        },
        run_id: RunId(uuid::Uuid::new_v4()),
        state_id: StateId::must_new("machine.main.s1".to_string()),
        attempt: 0,
    }
}

fn run_config_with_allowlist(prefixes: Vec<String>) -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff: BackoffPolicy::Fixed {
                delay: std::time::Duration::from_millis(0),
            },
        },
        event_profile: EventProfile::Normal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
        nix_flake_allowlist: prefixes,
    }
}

#[derive(Clone)]
struct FixedStreamStore {
    stream: Arc<Vec<EventEnvelope>>,
}

#[async_trait]
impl StreamStore for FixedStreamStore {
    async fn head_seq(&self, _stream_id: &StreamId) -> Result<u64, StorageError> {
        Ok(self.stream.last().map(|e| e.seq).unwrap_or(0))
    }

    async fn append(&self, _append: StreamAppend) -> Result<u64, StorageError> {
        Ok(self.stream.last().map(|e| e.seq).unwrap_or(0))
    }

    async fn append_batch(
        &self,
        _appends: Vec<StreamAppend>,
    ) -> Result<AppendBatchResult, StorageError> {
        Ok(AppendBatchResult {
            stream_heads: Vec::new(),
        })
    }

    async fn read_range(
        &self,
        _stream_id: &StreamId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<StreamRecord>, StorageError> {
        let to = to_seq.unwrap_or(u64::MAX);
        self.stream
            .iter()
            .filter(|e| e.seq >= from_seq && e.seq <= to)
            .cloned()
            .map(|envelope| {
                Ok(StreamRecord {
                    stream_id: StreamId::run(envelope.run_id),
                    seq: envelope.seq,
                    ts_millis: envelope.ts_millis,
                    kind: mfm_machine::events::STREAM_RECORD_KIND_MACHINE_EVENT.to_string(),
                    payload: serde_json::to_value(envelope.event).map_err(|_| {
                        StorageError::Other(info(
                            "event_encode_failed",
                            ErrorCategory::Storage,
                            "failed to encode event payload",
                        ))
                    })?,
                })
            })
            .collect()
    }
}

#[derive(Clone, Default)]
struct FixedArtifactStore {
    blobs: Arc<Mutex<HashMap<ArtifactId, Vec<u8>>>>,
}

#[async_trait]
impl ArtifactStore for FixedArtifactStore {
    async fn put(&self, _kind: ArtifactKind, bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        let id = mfm_machine::hashing::artifact_id_for_bytes(&bytes);
        self.blobs.lock().await.insert(id.clone(), bytes);
        Ok(id)
    }

    async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
        let inner = self.blobs.lock().await;
        inner.get(id).cloned().ok_or_else(|| {
            StorageError::NotFound(info(
                "not_found",
                ErrorCategory::Storage,
                "artifact not found",
            ))
        })
    }

    async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
        Ok(self.blobs.lock().await.contains_key(id))
    }
}

async fn env_with_manifest_allowlist(prefixes: Vec<String>) -> LiveIoEnv {
    let run_id = RunId(uuid::Uuid::new_v4());
    let manifest_id = ArtifactId("1".repeat(64));
    let initial_snapshot_id = ArtifactId("2".repeat(64));

    let manifest = RunManifest {
        op_id: OpId::must_new("nix_app".to_string()),
        op_version: "v1".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config_with_allowlist(prefixes),
        build: BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };

    let mut blobs = HashMap::new();
    blobs.insert(
        manifest_id.clone(),
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    );
    let artifacts = Arc::new(FixedArtifactStore {
        blobs: Arc::new(Mutex::new(blobs)),
    });

    let stream = vec![EventEnvelope {
        run_id,
        seq: 1,
        ts_millis: Some(0),
        event: Event::Kernel(KernelEvent::RunStarted {
            op_id: OpId::must_new("nix_app".to_string()),
            manifest_id,
            initial_snapshot_id,
        }),
    }];

    LiveIoEnv {
        stores: Stores {
            streams: Arc::new(FixedStreamStore {
                stream: Arc::new(stream),
            }),
            artifacts,
        },
        run_id,
        state_id: StateId::must_new("machine.main.s1".to_string()),
        attempt: 0,
    }
}

#[test]
fn split_flake_app_ref_requires_fragment() {
    let err = split_flake_app_ref("github:willyrgf/mfm").expect_err("expected error");
    match err {
        IoError::Other(info) => assert_eq!(info.code.0, CODE_NIX_REQUEST_INVALID),
        other => panic!("expected Other, got: {other:?}"),
    }
}

#[test]
fn attr_path_defaults_to_apps_system_program() {
    let system = "aarch64-darwin";
    let got = attr_path_for_program(system, "jq_fmt_example");
    assert_eq!(got, "apps.aarch64-darwin.jq_fmt_example.program");
}

#[test]
fn flake_installable_target_uses_attr_path() {
    let got = flake_installable_target("path:/repo", "apps.x86_64-linux.app.program");
    assert_eq!(got, "path:/repo#apps.x86_64-linux.app.program");
}

#[test]
fn repo_local_flake_ref_rewrites_against_workspace_root() {
    let rewritten = rewrite_repo_local_flake_ref_with_root(
        "path:.#aave-v3-origin-fetch",
        Path::new("/tmp/mfm-workspace"),
    )
    .expect("rewrite");
    assert_eq!(rewritten, "path:/tmp/mfm-workspace#aave-v3-origin-fetch");

    let nested = rewrite_repo_local_flake_ref_with_root(
        "path:./nixfied#tool",
        Path::new("/tmp/mfm-workspace"),
    )
    .expect("rewrite");
    assert_eq!(nested, "path:/tmp/mfm-workspace/nixfied#tool");
}

#[test]
fn flake_ref_prefix_matching_is_boundary_aware() {
    assert!(flake_ref_matches_prefix(
        "github:willyrgf/mfm",
        "github:willyrgf/mfm#jq_fmt_example"
    ));
    assert!(!flake_ref_matches_prefix(
        "github:willyrgf/mfm",
        "github:willyrgf/mfm-malicious#jq_fmt_example"
    ));
    assert!(flake_ref_matches_prefix(
        "path:.",
        "path:.#aave-v3-origin-fetch"
    ));
    assert!(flake_ref_matches_prefix("path:.", "path:./nixfied#tool"));
    assert!(!flake_ref_matches_prefix("path:.", "path:../outside#tool"));
}

#[test]
fn store_root_from_program_path_extracts_store_root() {
    let got = store_root_from_program_path("/nix/store/hash-app/bin/app");
    assert_eq!(got.as_deref(), Some("/nix/store/hash-app"));
}

#[test]
fn store_root_from_program_path_rejects_non_store_paths() {
    assert!(store_root_from_program_path("/tmp/app").is_none());
    assert!(store_root_from_program_path("/nix/store/").is_none());
}

#[test]
fn inject_host_env_bindings_uses_host_value_without_recording_it() {
    let source_env = format!("MFM_TEST_HOST_ENV_{}", uuid::Uuid::new_v4().simple());
    std::env::set_var(&source_env, "secret-value");
    let mut cmd = tokio::process::Command::new("env");
    let bindings = HashMap::from([("MFM_TARGET_ENV".to_string(), source_env.clone())]);

    inject_host_env_bindings(&mut cmd, &bindings).expect("inject bindings");

    let envs = cmd.as_std().get_envs().collect::<Vec<_>>();
    let target = envs
        .iter()
        .find(|(key, _)| *key == std::ffi::OsStr::new("MFM_TARGET_ENV"))
        .and_then(|(_, value)| value.as_ref())
        .and_then(|value| value.to_str());
    assert_eq!(target, Some("secret-value"));
    std::env::remove_var(source_env);
}

#[test]
fn inject_host_env_bindings_rejects_missing_source_env() {
    let mut cmd = tokio::process::Command::new("env");
    let bindings = HashMap::from([(
        "MFM_TARGET_ENV".to_string(),
        "MFM_TEST_MISSING_SOURCE_ENV".to_string(),
    )]);

    let err = inject_host_env_bindings(&mut cmd, &bindings).expect_err("missing env must fail");
    match err {
        IoError::Transport(info) => assert_eq!(info.code.0, CODE_NIX_HOST_ENV_MISSING),
        other => panic!("expected transport error, got: {other:?}"),
    }
}

#[test]
fn command_failure_details_include_exit_code_and_stderr() {
    let details = command_failure_details(
        "nix build",
        "path:/repo#apps.x86_64-linux.app.program",
        Some(100),
        b"",
        b"error: failed to fetch\n",
    );
    assert_eq!(
        details.get("command").and_then(|v| v.as_str()),
        Some("nix build")
    );
    assert_eq!(
        details.get("target").and_then(|v| v.as_str()),
        Some("path:/repo#apps.x86_64-linux.app.program")
    );
    assert_eq!(details.get("exit_code").and_then(|v| v.as_i64()), Some(100));
    assert!(details
        .get("stderr")
        .and_then(|v| v.as_str())
        .expect("stderr")
        .contains("failed to fetch"));
}

#[test]
fn command_failure_details_truncate_large_stderr() {
    let huge = "x".repeat(MAX_STDERR_DETAIL_BYTES + 32);
    let details = command_failure_details(
        "nix eval",
        "path:/repo#apps.x86_64-linux.app.program",
        Some(1),
        b"",
        huge.as_bytes(),
    );
    let stderr = details
        .get("stderr")
        .and_then(|v| v.as_str())
        .expect("stderr");
    assert!(stderr.starts_with("...[truncated]"));
}

#[tokio::test]
async fn rejects_disallowed_app_ref_by_default() {
    let factory = NixFlakeTransportFactory::default();
    let mut t = factory.make(env());

    let err = t
        .call(IoCall {
            namespace: NAMESPACE_NIX_EXEC.to_string(),
            request: serde_json::json!({
                "kind": "resolve_flake_app_v1",
                "app": "github:someone/else#app",
                "timeout_ms": 1
            }),
            fact_key: None,
        })
        .await
        .expect_err("expected error");

    match err {
        IoError::Other(info) => assert_eq!(info.code.0, CODE_NIX_APP_NOT_ALLOWED),
        other => panic!("expected Other, got: {other:?}"),
    }
}

#[test]
fn default_policy_allows_repo_local_path_refs() {
    let policy = NixFlakePolicy::default();
    assert!(flake_ref_allowed(&policy, "path:.#aave-v3-origin-fetch"));
    assert!(!flake_ref_allowed(&policy, "path:../elsewhere#tool"));
}

#[tokio::test]
async fn rejects_invalid_request_shape() {
    let factory = NixFlakeTransportFactory::default();
    let mut t = factory.make(env());

    let err = t
        .call(IoCall {
            namespace: NAMESPACE_NIX_EXEC.to_string(),
            request: serde_json::json!("not an object"),
            fact_key: None,
        })
        .await
        .expect_err("expected error");

    match err {
        IoError::Other(info) => assert_eq!(info.code.0, CODE_NIX_REQUEST_INVALID),
        other => panic!("expected Other, got: {other:?}"),
    }
}

#[tokio::test]
async fn manifest_allowlist_overrides_factory_policy() {
    let env = env_with_manifest_allowlist(vec!["path:/definitely-missing".to_string()]).await;
    let factory = NixFlakeTransportFactory::new(NixFlakePolicy {
        allow_prefixes: vec!["github:willyrgf/mfm".to_string()],
    });
    let mut t = factory.make(env);

    let err = t
        .call(IoCall {
            namespace: NAMESPACE_NIX_EXEC.to_string(),
            request: serde_json::json!({
                "kind": "resolve_flake_app_v1",
                "app": "path:/definitely-missing#jq_fmt_example",
                "timeout_ms": 1000
            }),
            fact_key: None,
        })
        .await
        .expect_err("expected error");

    match err {
        IoError::Transport(info) => {
            assert!(info.code.0 == CODE_NIX_EVAL_FAILED || info.code.0 == CODE_NIX_TIMEOUT)
        }
        IoError::Other(info) => {
            assert_ne!(info.code.0, CODE_NIX_APP_NOT_ALLOWED)
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
