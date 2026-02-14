use async_trait::async_trait;

use alloy_primitives::{Address, U256};
use mfm_collectors_evm::{parse_u64_hex_value, EvmIoClient, JsonRpcCall};
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx::{read_u64_required, write_json};
use crate::errors::{state_error_with_state, state_from_io, state_unknown};
use crate::evm_encoding::{ERC20_SELECTOR_BALANCE_OF, ERC20_SELECTOR_DECIMALS};
use crate::states::meta;

fn parse_hex_string_response(value: &serde_json::Value) -> Result<String, StateError> {
    let Some(s) = value.as_str() else {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    };
    let Some(rest) = s.strip_prefix("0x") else {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    };
    if !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    }
    Ok(s.to_string())
}

fn parse_u256_hex_response(value: &serde_json::Value) -> Result<String, StateError> {
    let s = parse_hex_string_response(value)?;
    let rest = s.strip_prefix("0x").unwrap_or_default();
    if rest.is_empty() || rest.len() > 64 {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    }
    Ok(s)
}

#[derive(Clone, Debug)]
pub struct U64Expectation {
    pub expected: u64,
    pub mismatch_code: &'static str,
    pub mismatch_category: ErrorCategory,
    pub mismatch_retryable: bool,
    pub mismatch_message: &'static str,
}

impl U64Expectation {
    pub fn parsing_input(
        expected: u64,
        mismatch_code: &'static str,
        mismatch_message: &'static str,
    ) -> Self {
        Self {
            expected,
            mismatch_code,
            mismatch_category: ErrorCategory::ParsingInput,
            mismatch_retryable: false,
            mismatch_message,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReadHexStringState {
    pub state_id: StateId,
    pub method: String,
    pub params: serde_json::Value,
    pub output_key: ContextKey,
}

impl ReadHexStringState {
    pub fn new(
        state_id: StateId,
        method: impl Into<String>,
        params: serde_json::Value,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            method: method.into(),
            params,
            output_key,
        }
    }
}

#[async_trait]
impl State for ReadHexStringState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new(self.method.clone(), self.params.clone()))
            .await
            .map_err(state_from_io)?;
        let value = parse_hex_string_response(&res.response)?;
        write_json(ctx, self.output_key.clone(), serde_json::json!(value))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ReadU256HexState {
    pub state_id: StateId,
    pub method: String,
    pub params: serde_json::Value,
    pub output_key: ContextKey,
}

impl ReadU256HexState {
    pub fn new(
        state_id: StateId,
        method: impl Into<String>,
        params: serde_json::Value,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            method: method.into(),
            params,
            output_key,
        }
    }
}

#[async_trait]
impl State for ReadU256HexState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new(self.method.clone(), self.params.clone()))
            .await
            .map_err(state_from_io)?;
        let value = parse_u256_hex_response(&res.response)?;
        write_json(ctx, self.output_key.clone(), serde_json::json!(value))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub enum EthCallDecode {
    #[default]
    HexString,
    U64,
    U256Hex,
}

#[derive(Clone, Debug)]
pub struct EthCallState {
    pub state_id: StateId,
    pub to: String,
    pub data: String,
    pub block: serde_json::Value,
    pub output_key: ContextKey,
    pub decode: EthCallDecode,
}

impl EthCallState {
    pub fn new(
        state_id: StateId,
        to: impl Into<String>,
        data: impl Into<String>,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            to: to.into(),
            data: data.into(),
            block: serde_json::json!("latest"),
            output_key,
            decode: EthCallDecode::HexString,
        }
    }

    pub fn with_block(mut self, block: serde_json::Value) -> Self {
        self.block = block;
        self
    }

    pub fn with_decode(mut self, decode: EthCallDecode) -> Self {
        self.decode = decode;
        self
    }
}

#[async_trait]
impl State for EthCallState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new(
                "eth_call",
                serde_json::json!([
                    {"to": self.to.clone(), "data": self.data.clone()},
                    self.block.clone()
                ]),
            ))
            .await
            .map_err(state_from_io)?;

        let parsed = match self.decode {
            EthCallDecode::HexString => {
                serde_json::json!(parse_hex_string_response(&res.response)?)
            }
            EthCallDecode::U64 => serde_json::json!(parse_u64_hex_value(&res.response).map_err(
                |_| state_unknown("evm_response_invalid", "evm response was not a hex u64")
            )?),
            EthCallDecode::U256Hex => serde_json::json!(parse_u256_hex_response(&res.response)?),
        };

        write_json(ctx, self.output_key.clone(), parsed)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ReadU64HexState {
    pub state_id: StateId,
    pub method: String,
    pub params: serde_json::Value,
    pub output_key: ContextKey,
    pub expectation: Option<U64Expectation>,
}

impl ReadU64HexState {
    pub fn new(
        state_id: StateId,
        method: impl Into<String>,
        params: serde_json::Value,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            method: method.into(),
            params,
            output_key,
            expectation: None,
        }
    }

    pub fn with_expectation(mut self, expectation: U64Expectation) -> Self {
        self.expectation = Some(expectation);
        self
    }
}

#[async_trait]
impl State for ReadU64HexState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new(self.method.clone(), self.params.clone()))
            .await
            .map_err(state_from_io)?;

        let value = parse_u64_hex_value(&res.response)
            .map_err(|_| state_unknown("evm_response_invalid", "evm response was not a hex u64"))?;

        if let Some(expectation) = &self.expectation {
            if value != expectation.expected {
                return Err(state_error_with_state(
                    self.state_id.clone(),
                    expectation.mismatch_code,
                    expectation.mismatch_category.clone(),
                    expectation.mismatch_retryable,
                    expectation.mismatch_message,
                ));
            }
        }

        write_json(ctx, self.output_key.clone(), serde_json::json!(value))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

pub fn address_hex_lower(addr: &Address) -> String {
    // Debug formatting is lowercase and stable.
    format!("{addr:?}")
}

pub fn address_hex_lower_no0x(addr: &Address) -> String {
    address_hex_lower(addr)
        .strip_prefix("0x")
        .unwrap_or("")
        .to_string()
}

fn u64_hex_quantity(n: u64) -> String {
    if n == 0 {
        return "0x0".to_string();
    }
    format!("0x{:x}", n)
}

fn format_u256_units(raw: &U256, decimals: u8) -> String {
    let s = raw.to_string();
    let d = decimals as usize;
    if d == 0 {
        return s;
    }
    if s.len() <= d {
        let mut out = String::with_capacity(2 + d + 1);
        out.push_str("0.");
        out.push_str(&"0".repeat(d - s.len()));
        out.push_str(&s);
        out
    } else {
        let split = s.len() - d;
        let mut out = String::with_capacity(s.len() + 1);
        out.push_str(&s[..split]);
        out.push('.');
        out.push_str(&s[split..]);
        out
    }
}

fn parse_u256_hex(s: &str) -> Result<U256, StateError> {
    let Some(rest) = s.strip_prefix("0x") else {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    };
    if rest.is_empty() || rest.len() > 64 {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    }

    let mut hex_str = rest.to_string();
    if hex_str.len() % 2 == 1 {
        hex_str = format!("0{hex_str}");
    }
    let bytes = hex::decode(hex_str)
        .map_err(|_| state_unknown("evm_response_invalid", "evm response was not a hex u256"))?;
    Ok(U256::from_be_slice(&bytes))
}

fn parse_u256_hex_value(v: &serde_json::Value) -> Result<U256, StateError> {
    let Some(s) = v.as_str() else {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    };
    parse_u256_hex(s)
}

fn parse_u8_u256(v: U256) -> Result<u8, StateError> {
    if v > U256::from(u8::MAX) {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was out of range for u8",
        ));
    }
    Ok(v.to::<u8>())
}

pub fn encode_erc20_balance_of(owner: &Address) -> String {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&ERC20_SELECTOR_BALANCE_OF);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(owner.as_slice());
    format!("0x{}", hex::encode(data))
}

pub fn encode_erc20_decimals() -> String {
    let mut data = Vec::with_capacity(4);
    data.extend_from_slice(&ERC20_SELECTOR_DECIMALS);
    format!("0x{}", hex::encode(data))
}

#[derive(Clone, Debug)]
pub struct NativeBalanceState {
    pub state_id: StateId,
    pub wallet: Address,
    pub block_key: ContextKey,
    pub output_key: ContextKey,
    pub symbol: String,
    pub decimals: u8,
}

impl NativeBalanceState {
    pub fn new(
        state_id: StateId,
        wallet: Address,
        block_key: ContextKey,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            wallet,
            block_key,
            output_key,
            symbol: "ETH".to_string(),
            decimals: 18,
        }
    }
}

#[async_trait]
impl State for NativeBalanceState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let block = read_u64_required(
            ctx,
            &self.block_key,
            "missing_block_number",
            "missing block_number in context",
        )?;

        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new(
                "eth_getBalance",
                serde_json::json!([address_hex_lower(&self.wallet), u64_hex_quantity(block)]),
            ))
            .await
            .map_err(state_from_io)?;

        let wei = parse_u256_hex_value(&res.response)?;
        let native = serde_json::json!({
            "symbol": self.symbol,
            "raw_u256_dec": wei.to_string(),
            "decimals": self.decimals,
            "amount_dec": format_u256_units(&wei, self.decimals),
        });

        write_json(ctx, self.output_key.clone(), native)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TokenBalanceState {
    pub state_id: StateId,
    pub token: Address,
    pub wallet: Address,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
    pub block_key: ContextKey,
    pub output_key: ContextKey,
}

impl TokenBalanceState {
    pub fn new(
        state_id: StateId,
        token: Address,
        wallet: Address,
        symbol: Option<String>,
        decimals: Option<u8>,
        block_key: ContextKey,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            token,
            wallet,
            symbol,
            decimals,
            block_key,
            output_key,
        }
    }
}

#[async_trait]
impl State for TokenBalanceState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let block = read_u64_required(
            ctx,
            &self.block_key,
            "missing_block_number",
            "missing block_number in context",
        )?;

        let token_to = address_hex_lower(&self.token);
        let at_block = u64_hex_quantity(block);
        let mut client = EvmIoClient::new(self.state_id.clone(), io);

        let decimals: u8 = match self.decimals {
            Some(d) => d,
            None => {
                let res = client
                    .call(JsonRpcCall::new(
                        "eth_call",
                        serde_json::json!([
                            {"to": token_to, "data": encode_erc20_decimals()},
                            at_block.clone()
                        ]),
                    ))
                    .await
                    .map_err(state_from_io)?;
                let v = parse_u256_hex_value(&res.response)?;
                parse_u8_u256(v)?
            }
        };

        let res = client
            .call(JsonRpcCall::new(
                "eth_call",
                serde_json::json!([
                    {"to": address_hex_lower(&self.token), "data": encode_erc20_balance_of(&self.wallet)},
                    at_block
                ]),
            ))
            .await
            .map_err(state_from_io)?;

        let raw = parse_u256_hex_value(&res.response)?;
        let token_obj = serde_json::json!({
            "address": address_hex_lower(&self.token),
            "symbol": self.symbol.clone(),
            "decimals": decimals,
            "raw_u256_dec": raw.to_string(),
            "amount_dec": format_u256_units(&raw, decimals),
        });

        write_json(ctx, self.output_key.clone(), token_obj)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use mfm_machine::errors::ContextError;
    use mfm_machine::errors::{IoError, RunError};
    use mfm_machine::events::DomainEvent;
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};

    #[derive(Default)]
    struct MapContext {
        values: HashMap<String, serde_json::Value>,
    }

    impl DynContext for MapContext {
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
            Ok(self.values.get(&key.0).cloned())
        }

        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
            self.values.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
            self.values.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<serde_json::Value, ContextError> {
            let mut out = serde_json::Map::new();
            for (k, v) in &self.values {
                out.insert(k.clone(), v.clone());
            }
            Ok(serde_json::Value::Object(out))
        }
    }

    #[derive(Default)]
    struct FixedIo {
        responses: HashMap<String, serde_json::Value>,
    }

    #[async_trait]
    impl IoProvider for FixedIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            let method = call
                .request
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let Some(response) = self.responses.get(&method) else {
                return Err(IoError::Other(crate::errors::info(
                    "unknown_method",
                    ErrorCategory::Rpc,
                    false,
                    "unknown rpc method",
                )));
            };

            Ok(IoResult {
                response: response.clone(),
                recorded_payload_id: None,
            })
        }

        async fn get_recorded_fact(
            &mut self,
            _key: &FactKey,
        ) -> Result<Option<ArtifactId>, IoError> {
            Ok(None)
        }

        async fn now_millis(&mut self) -> Result<u64, IoError> {
            Ok(0)
        }

        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
            Ok(vec![0u8; n])
        }
    }

    struct NoopRecorder;

    #[async_trait]
    impl EventRecorder for NoopRecorder {
        async fn emit(&mut self, _event: DomainEvent) -> Result<(), RunError> {
            Ok(())
        }

        async fn emit_many(&mut self, _events: Vec<DomainEvent>) -> Result<(), RunError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn reads_u64_and_writes_context() {
        let state = ReadU64HexState::new(
            StateId("m.main.chain_id".to_string()),
            "eth_chainId",
            serde_json::json!([]),
            ContextKey("chain_id".to_string()),
        );

        let mut ctx = MapContext::default();
        let mut io = FixedIo::default();
        io.responses
            .insert("eth_chainId".to_string(), serde_json::json!("0x1"));
        let mut rec = NoopRecorder;

        let out = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("state handle");
        assert_eq!(out.snapshot, SnapshotPolicy::OnSuccess);
        assert_eq!(
            ctx.read(&ContextKey("chain_id".to_string())).expect("read"),
            Some(serde_json::json!(1))
        );
    }

    #[tokio::test]
    async fn invalid_hex_response_fails_with_stable_code() {
        let state = ReadU64HexState::new(
            StateId("m.main.chain_id".to_string()),
            "eth_chainId",
            serde_json::json!([]),
            ContextKey("chain_id".to_string()),
        );

        let mut ctx = MapContext::default();
        let mut io = FixedIo::default();
        io.responses
            .insert("eth_chainId".to_string(), serde_json::json!("not_hex"));
        let mut rec = NoopRecorder;

        let err = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect_err("expected parse error");
        assert_eq!(err.info.code.0, "evm_response_invalid");
    }

    #[tokio::test]
    async fn expectation_mismatch_fails_with_state_scoped_error() {
        let state = ReadU64HexState::new(
            StateId("m.main.chain_id".to_string()),
            "eth_chainId",
            serde_json::json!([]),
            ContextKey("chain_id".to_string()),
        )
        .with_expectation(U64Expectation::parsing_input(
            1,
            "chain_id_mismatch",
            "rpc chain_id did not match configured chain_id",
        ));

        let mut ctx = MapContext::default();
        let mut io = FixedIo::default();
        io.responses
            .insert("eth_chainId".to_string(), serde_json::json!("0x2"));
        let mut rec = NoopRecorder;

        let err = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect_err("expected mismatch");
        assert_eq!(err.info.code.0, "chain_id_mismatch");
        assert_eq!(err.state_id, Some(StateId("m.main.chain_id".to_string())));
    }

    #[test]
    fn io_error_mapping_preserves_error_info() {
        let io_err = IoError::Other(mfm_machine::errors::ErrorInfo {
            code: ErrorCode("io_code".to_string()),
            category: ErrorCategory::Rpc,
            retryable: false,
            message: "io message".to_string(),
            details: None,
        });
        let state_err = state_from_io(io_err);
        assert_eq!(state_err.info.code.0, "io_code");
        assert_eq!(state_err.info.message, "io message");
    }

    #[tokio::test]
    async fn read_hex_string_writes_string_value() {
        let state = ReadHexStringState::new(
            StateId("m.main.client_version".to_string()),
            "web3_clientVersion",
            serde_json::json!([]),
            ContextKey("client_version".to_string()),
        );

        let mut ctx = MapContext::default();
        let mut io = FixedIo::default();
        io.responses.insert(
            "web3_clientVersion".to_string(),
            serde_json::json!("0xdeadbeef"),
        );
        let mut rec = NoopRecorder;

        state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("state");
        assert_eq!(
            ctx.read(&ContextKey("client_version".to_string()))
                .expect("read"),
            Some(serde_json::json!("0xdeadbeef"))
        );
    }

    #[tokio::test]
    async fn read_u256_hex_rejects_overflow() {
        let state = ReadU256HexState::new(
            StateId("m.main.balance".to_string()),
            "eth_getBalance",
            serde_json::json!(["0xabc", "latest"]),
            ContextKey("balance".to_string()),
        );

        let mut ctx = MapContext::default();
        let mut io = FixedIo::default();
        io.responses.insert(
            "eth_getBalance".to_string(),
            serde_json::json!(format!("0x{}", "f".repeat(65))),
        );
        let mut rec = NoopRecorder;

        let err = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect_err("overflow");
        assert_eq!(err.info.code.0, "evm_response_invalid");
    }

    #[tokio::test]
    async fn eth_call_u64_decode_writes_numeric_value() {
        let state = EthCallState::new(
            StateId("m.main.eth_call".to_string()),
            "0x0000000000000000000000000000000000000000",
            "0x313ce567",
            ContextKey("decimals".to_string()),
        )
        .with_decode(EthCallDecode::U64);

        let mut ctx = MapContext::default();
        let mut io = FixedIo::default();
        io.responses
            .insert("eth_call".to_string(), serde_json::json!("0x12"));
        let mut rec = NoopRecorder;

        state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("state");
        assert_eq!(
            ctx.read(&ContextKey("decimals".to_string())).expect("read"),
            Some(serde_json::json!(18))
        );
    }
}
