use super::*;

use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::events::{Event, KernelEvent};
use mfm_machine::ids::ArtifactId;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
use mfm_machine::runtime::DefaultExecutionEngine;
use mfm_sdk::unstable::SdkPlanResolver;
use mfm_state_common::test_support as op_test_support;

fn info(code: &'static str, category: ErrorCategory, retryable: bool, message: &str) -> ErrorInfo {
    op_errors::info(code, category, retryable, message)
}

#[derive(Clone)]
struct MockEvmTransportFactory {
    chain_id_hex: String,
}

impl Default for MockEvmTransportFactory {
    fn default() -> Self {
        Self {
            chain_id_hex: "0x1".to_string(),
        }
    }
}

impl LiveIoTransportFactory for MockEvmTransportFactory {
    fn namespace_group(&self) -> &str {
        "evm"
    }

    fn make(&self, _env: mfm_machine::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(MockEvmTransport {
            chain_id_hex: self.chain_id_hex.clone(),
        })
    }
}

struct MockEvmTransport {
    chain_id_hex: String,
}

fn ok_u256_as_32byte_hex(n: u64) -> serde_json::Value {
    let hex_val = format!("{:x}", n);
    let padded = format!("{}{}", "0".repeat(64 - hex_val.len()), hex_val);
    serde_json::json!(format!("0x{padded}"))
}

#[async_trait]
impl LiveIoTransport for MockEvmTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let method = call
            .request
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        match method {
            "eth_chainId" => Ok(serde_json::Value::String(self.chain_id_hex.clone())),
            "eth_blockNumber" => Ok(serde_json::json!("0x64")),
            "eth_getBalance" => Ok(serde_json::json!("0xde0b6b3a7640000")), // 1 ETH
            "eth_call" => {
                let data = call
                    .request
                    .get("params")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_object())
                    .and_then(|o| o.get("data"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if data == encode_erc20_decimals() {
                    Ok(ok_u256_as_32byte_hex(6))
                } else if data.starts_with("0x70a08231") {
                    Ok(ok_u256_as_32byte_hex(1_000_000)) // 1 token @ 6 decimals
                } else {
                    Err(IoError::Other(info(
                        "unknown_eth_call",
                        ErrorCategory::Unknown,
                        false,
                        "unknown eth_call payload",
                    )))
                }
            }
            _ => Err(IoError::Other(info(
                "unknown_method",
                ErrorCategory::Unknown,
                false,
                "unknown jsonrpc method",
            ))),
        }
    }
}

async fn run_portfolio_snapshot_with_transports(
    op_config: serde_json::Value,
    evm_factory: Arc<dyn LiveIoTransportFactory>,
) -> (mfm_machine::engine::RunResult, Stores) {
    let op: mfm_sdk::op::DynOperation = Arc::new(PortfolioTrackerOp);
    let (registry, planner, pipeline) =
        op_test_support::single_op_plan(op, op_config).expect("pipeline");

    let resolver = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));

    let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(
        RouterLiveIoTransportFactory::from_factories(vec![evm_factory]).expect("router factories"),
    );

    let engine =
        DefaultExecutionEngine::new(resolver).with_live_transport_factory(Arc::clone(&factory));
    let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

    let stores = op_test_support::in_memory_stores();

    let res = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline,
        op_test_support::run_config_live(),
    )
    .await
    .expect("run");

    (res, stores)
}

#[test]
fn expand_sorts_tokens_and_uses_full_address_in_state_id() {
    let op = PortfolioTrackerOp;
    let g = op
        .expand(
            OpPath("portfolio_tracker.main".to_string()),
            &serde_json::json!({
                "wallet_address": "0x000000000000000000000000000000000000dead",
                "chain_id": 1,
                "tokens": [
                    {
                        "address": "0x000000000000000000000000000000000000beef",
                        "symbol": "BEEF",
                        "decimals": 6
                    },
                    {
                        "address": "0x0000000000000000000000000000000000000001",
                        "symbol": "ONE",
                        "decimals": 6
                    }
                ]
            }),
            &op_test_support::run_config_live(),
        )
        .expect("expand");

    let ids: Vec<String> = g.states.iter().map(|n| n.id.as_str().to_string()).collect();

    let a1 = "0000000000000000000000000000000000000001";
    let abeef = "000000000000000000000000000000000000beef";
    assert_eq!(a1.len(), 40);
    assert_eq!(abeef.len(), 40);

    let i1 = ids
        .iter()
        .position(|id| id == &format!("portfolio_tracker.main.token_balance_{a1}"))
        .expect("token_balance state for 0x...0001");
    let ib = ids
        .iter()
        .position(|id| id == &format!("portfolio_tracker.main.token_balance_{abeef}"))
        .expect("token_balance state for 0x...beef");

    assert!(i1 < ib, "token balance states must be sorted by address");
    let iw = ids
        .iter()
        .position(|id| id == "portfolio_tracker.main.write_snapshot")
        .expect("write_snapshot state");
    let ir = ids
        .iter()
        .position(|id| id == "portfolio_tracker.main.report")
        .expect("report state");
    assert!(iw < ir, "report state must run after write_snapshot");
}

#[tokio::test]
async fn chain_id_mismatch_fails_with_stable_code() {
    let (res, stores) = run_portfolio_snapshot_with_transports(
        serde_json::json!({
            "wallet_address": "0x000000000000000000000000000000000000dead",
            "chain_id": 1,
            "tokens": []
        }),
        Arc::new(MockEvmTransportFactory {
            chain_id_hex: "0x2".to_string(),
        }),
    )
    .await;

    assert_eq!(res.phase, RunPhase::Failed);

    let stream = stores
        .events
        .read_range(res.run_id, 1, None)
        .await
        .expect("events");

    let mut found = false;
    for e in &stream {
        let Event::Kernel(KernelEvent::StateFailed {
            state_id, error, ..
        }) = &e.event
        else {
            continue;
        };
        if state_id.as_str() != "portfolio_tracker.main.chain_id" {
            continue;
        }
        assert_eq!(error.info.code.0, "chain_id_mismatch");
        found = true;
        break;
    }
    assert!(found, "expected StateFailed for chain_id state");
}

#[tokio::test]
async fn snapshot_decimals_fallback_calls_decimals_and_formats_amount() {
    let (res, stores) = run_portfolio_snapshot_with_transports(
        serde_json::json!({
            "wallet_address": "0x000000000000000000000000000000000000dead",
            "chain_id": 1,
            "tokens": [
                {
                    "address": "0x000000000000000000000000000000000000beef",
                    "symbol": "TKN",
                    "decimals": null
                }
            ]
        }),
        Arc::new(MockEvmTransportFactory::default()),
    )
    .await;

    assert_eq!(res.phase, RunPhase::Completed);

    let final_snapshot_id = res.final_snapshot_id.expect("final snapshot id");
    let bytes = stores
        .artifacts
        .get(&final_snapshot_id)
        .await
        .expect("snapshot bytes");
    let snapshot: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot json");

    let out_id = snapshot
        .get("portfolio_tracker.main.snapshot_artifact_id")
        .and_then(|v| v.as_str())
        .expect("snapshot_artifact_id (context key) missing")
        .to_string();

    let out_bytes = stores
        .artifacts
        .get(&ArtifactId(out_id))
        .await
        .expect("output artifact bytes");
    let out: serde_json::Value = serde_json::from_slice(&out_bytes).expect("output json");

    let tokens = out
        .get("tokens")
        .and_then(|v| v.as_array())
        .expect("tokens array");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].get("decimals").and_then(|v| v.as_u64()), Some(6));
    assert_eq!(
        tokens[0].get("amount_dec").and_then(|v| v.as_str()),
        Some("1.000000")
    );
}

#[tokio::test]
async fn snapshot_token_ordering_is_deterministic() {
    let (res, stores) = run_portfolio_snapshot_with_transports(
        serde_json::json!({
            "wallet_address": "0x000000000000000000000000000000000000dead",
            "chain_id": 1,
            "tokens": [
                {
                    "address": "0x000000000000000000000000000000000000beef",
                    "symbol": "BEEF",
                    "decimals": 6
                },
                {
                    "address": "0x0000000000000000000000000000000000000001",
                    "symbol": "ONE",
                    "decimals": 6
                }
            ]
        }),
        Arc::new(MockEvmTransportFactory::default()),
    )
    .await;

    assert_eq!(res.phase, RunPhase::Completed);

    let final_snapshot_id = res.final_snapshot_id.expect("final snapshot id");
    let bytes = stores
        .artifacts
        .get(&final_snapshot_id)
        .await
        .expect("snapshot bytes");
    let snapshot: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot json");

    let out_id = snapshot
        .get("portfolio_tracker.main.snapshot_artifact_id")
        .and_then(|v| v.as_str())
        .expect("snapshot_artifact_id (context key) missing")
        .to_string();

    let out_bytes = stores
        .artifacts
        .get(&ArtifactId(out_id))
        .await
        .expect("output artifact bytes");
    let out: serde_json::Value = serde_json::from_slice(&out_bytes).expect("output json");

    let tokens = out
        .get("tokens")
        .and_then(|v| v.as_array())
        .expect("tokens array");
    assert_eq!(
        tokens
            .iter()
            .filter_map(|t| t.get("address").and_then(|v| v.as_str()))
            .collect::<Vec<_>>(),
        vec![
            "0x0000000000000000000000000000000000000001",
            "0x000000000000000000000000000000000000beef",
        ]
    );
}

#[tokio::test]
async fn snapshot_writes_output_artifact() {
    let (res, stores) = run_portfolio_snapshot_with_transports(
        serde_json::json!({
            "wallet_address": "0x000000000000000000000000000000000000dead",
            "chain_id": 1,
            "tokens": [
                {
                    "address": "0x000000000000000000000000000000000000beef",
                    "symbol": "TKN",
                    "decimals": 6
                }
            ]
        }),
        Arc::new(MockEvmTransportFactory::default()),
    )
    .await;

    assert_eq!(res.phase, RunPhase::Completed);

    let stream = stores
        .events
        .read_range(res.run_id, 1, None)
        .await
        .expect("events");
    let final_snapshot_id =
        op_test_support::run_completed_snapshot_id(&stream).expect("final snapshot id");

    let bytes = stores
        .artifacts
        .get(&final_snapshot_id)
        .await
        .expect("snapshot bytes");
    let snapshot: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot json");

    let out_id = snapshot
        .get("portfolio_tracker.main.snapshot_artifact_id")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("snapshot_artifact_id (context key) missing: {snapshot}"));

    let out_bytes = stores
        .artifacts
        .get(&ArtifactId(out_id.to_string()))
        .await
        .expect("output artifact bytes");
    let out: serde_json::Value = serde_json::from_slice(&out_bytes).expect("output json");

    assert_eq!(
        out.get("wallet_address").and_then(|v| v.as_str()),
        Some("0x000000000000000000000000000000000000dead")
    );
    assert_eq!(out.get("chain_id").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(out.get("block_number").and_then(|v| v.as_u64()), Some(100));
    assert_eq!(
        out.get("tokens")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(1)
    );

    let report_key = portfolio_tracker_report_context_key();
    let report: PortfolioTrackerReport = serde_json::from_value(
        snapshot
            .get(&report_key.0)
            .cloned()
            .unwrap_or_else(|| panic!("report context key missing: {snapshot}")),
    )
    .expect("report decode");
    assert_eq!(report.snapshot_artifact_id, out_id);
    assert_eq!(report.chain_id, 1);
    assert_eq!(report.block_number, 100);
    let native_balance = report.native_balance.expect("native balance in report");
    assert_eq!(native_balance.symbol, "ETH");
    assert_eq!(native_balance.decimals, 18);
    assert_eq!(native_balance.raw_u256_dec, "1000000000000000000");
    assert_eq!(native_balance.amount_dec, "1.000000000000000000");
}
