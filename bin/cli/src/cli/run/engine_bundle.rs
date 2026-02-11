use std::sync::Arc;

use async_trait::async_trait;

use crate::cli::command_result::CommandError;
use mfm_collectors_evm_jsonrpc_http::{EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory};
use mfm_machine::engine::ExecutionEngine;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError, RunError, StorageError};
use mfm_machine::exec_transport::ExecProgramTransportFactory;
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
use mfm_machine::nix_exec_transport::NixFlakeTransportFactory;
use mfm_machine::runtime::{ChildRunLiveIoTransportFactory, DefaultExecutionEngine, PlanResolver};
use mfm_op_evm_read::EvmReadOp;
use mfm_op_nix_app::NixAppOp;
use mfm_op_proof::ProofOp;
use mfm_sdk::op::OperationRegistry;
use mfm_sdk::pipeline::PipelinePlanner;
use mfm_sdk::unstable::{DefaultPipelinePlanner, HashMapOperationRegistry, SdkPlanResolver};

const ENV_EVM_RPC_URL: &str = "MFM_EVM_RPC_URL";
const ENV_EVM_RPC_AUTHORIZATION: &str = "MFM_EVM_RPC_AUTHORIZATION";

fn info(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.into(),
        details: None,
    }
}

struct CliLiveIoTransportFactory;

impl LiveIoTransportFactory for CliLiveIoTransportFactory {
    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(CliLiveIoTransport)
    }
}

struct CliLiveIoTransport;

#[async_trait]
impl LiveIoTransport for CliLiveIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        match call.namespace.as_str() {
            "proof.read" => Ok(serde_json::json!({ "n": 1 })),
            "proof.side_effect" => {
                let k = call
                    .request
                    .get("idempotency_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("missing");
                let prefix: String = k.chars().take(8).collect();
                Ok(serde_json::json!({ "tx_hash": format!("0x{prefix}") }))
            }
            "proof.output" => Ok(call.request),
            other => Err(IoError::Other(info(
                "unknown_namespace",
                ErrorCategory::Unknown,
                format!("unknown namespace: {other}"),
            ))),
        }
    }
}

pub(super) struct EngineBundle {
    pub engine: Arc<dyn ExecutionEngine>,
    pub registry: Arc<dyn OperationRegistry>,
    pub planner: Arc<dyn PipelinePlanner>,
}

pub(super) fn make_engine_bundle() -> EngineBundle {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(ProofOp::default()));
    reg.register(Arc::new(EvmReadOp));
    reg.register(Arc::new(NixAppOp));
    let registry: Arc<dyn OperationRegistry> = Arc::new(reg);

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));

    let rpc_url = std::env::var(ENV_EVM_RPC_URL).ok();
    let authorization = std::env::var(ENV_EVM_RPC_AUTHORIZATION).ok();
    let evm_factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
            rpc_url,
            authorization,
            ..EvmJsonRpcHttpConfig::default()
        }));

    let mut routes: std::collections::HashMap<String, Arc<dyn LiveIoTransportFactory>> =
        std::collections::HashMap::new();
    routes.insert("proof".to_string(), Arc::new(CliLiveIoTransportFactory));
    routes.insert(
        "exec".to_string(),
        Arc::new(ExecProgramTransportFactory::default()),
    );
    routes.insert(
        "nix".to_string(),
        Arc::new(NixFlakeTransportFactory::default()),
    );
    routes.insert("evm".to_string(), evm_factory);

    let base_factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(RouterLiveIoTransportFactory::new(routes));
    let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(ChildRunLiveIoTransportFactory::new(
        Arc::clone(&resolver),
        Arc::clone(&base_factory),
    ));
    let engine: Arc<dyn ExecutionEngine> =
        Arc::new(DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory));

    EngineBundle {
        engine,
        registry,
        planner,
    }
}

pub(super) fn command_error_from_storage_error(err: StorageError) -> CommandError {
    match err {
        StorageError::Concurrency(info)
        | StorageError::NotFound(info)
        | StorageError::Corruption(info)
        | StorageError::Other(info) => CommandError::new(info.code.0, info.message),
    }
}

pub(super) fn command_error_from_run_error(err: RunError) -> CommandError {
    match err {
        RunError::InvalidPlan(info) => CommandError::new(info.code.0, info.message),
        RunError::Storage(se) => match se {
            StorageError::Concurrency(info)
            | StorageError::NotFound(info)
            | StorageError::Corruption(info)
            | StorageError::Other(info) => CommandError::new(info.code.0, info.message),
        },
        RunError::Context(ce) => match ce {
            mfm_machine::errors::ContextError::MissingKey { info, .. }
            | mfm_machine::errors::ContextError::Serialization(info)
            | mfm_machine::errors::ContextError::Other(info) => {
                CommandError::new(info.code.0, info.message)
            }
        },
        RunError::Io(ie) => match ie {
            IoError::MissingFactKey(info)
            | IoError::MissingFact { info, .. }
            | IoError::Transport(info)
            | IoError::RateLimited(info)
            | IoError::Other(info) => CommandError::new(info.code.0, info.message),
        },
        RunError::State(se) => CommandError::new(se.info.code.0, se.info.message),
        RunError::Other(info) => CommandError::new(info.code.0, info.message),
    }
}
