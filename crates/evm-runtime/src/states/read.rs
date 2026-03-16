//! Reusable EVM read/query states.
//!
//! These states record deterministic read facts through the `rpc.control` namespace and write
//! normalized results back into context for downstream states and report assembly.
//!
//! Use this module for read-only chain queries such as raw hex reads, `eth_call`, block-number
//! checks, and native/ERC-20 balance inspection.

use async_trait::async_trait;

use alloy_primitives::Address;
use mfm_collectors_rpc_control::{
    parse_u64_hex_value, EvmIoClient, JsonRpcCall, DEFAULT_CONTROL_SCOPE,
};
use mfm_evm_core::encoding::{
    format_u256_units, parse_hex_string_response, parse_u256_hex_response, parse_u256_hex_value,
    parse_u8_u256, u64_hex_quantity,
};
use mfm_evm_core::util_error::UtilError;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::ctx::{read_u64_required, write_json};
use mfm_state_common::errors::{
    state_error_with_state, state_from_io, state_unknown, state_unknown_msg,
};
use mfm_state_common::states::meta;

pub use mfm_evm_core::encoding::{
    address_hex_lower, address_hex_lower_no0x, encode_erc20_balance_of, encode_erc20_decimals,
};

fn parse_error(err: UtilError) -> StateError {
    state_unknown_msg(err.code, err.message)
}

fn default_control_scope() -> String {
    DEFAULT_CONTROL_SCOPE.to_string()
}

fn managed_call(
    network_id: &str,
    control_scope: &str,
    method: impl Into<String>,
    params: serde_json::Value,
) -> JsonRpcCall {
    JsonRpcCall::new(method, params)
        .with_control_scope(control_scope.to_string())
        .with_network_id(network_id.to_string())
}

trait RpcResponseParser {
    fn parse(
        &self,
        state_id: &StateId,
        response: &serde_json::Value,
    ) -> Result<serde_json::Value, StateError>;
}

#[allow(clippy::too_many_arguments)]
async fn execute_rpc_read<P: RpcResponseParser>(
    state_id: &StateId,
    network_id: &str,
    control_scope: &str,
    method: &str,
    params: &serde_json::Value,
    output_key: &ContextKey,
    parser: &P,
    ctx: &mut dyn DynContext,
    io: &mut dyn IoProvider,
) -> Result<StateOutcome, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let res = client
        .call(managed_call(
            network_id,
            control_scope,
            method.to_string(),
            params.clone(),
        ))
        .await
        .map_err(state_from_io)?;
    let parsed = parser.parse(state_id, &res.response)?;
    write_json(ctx, output_key.clone(), parsed)?;

    Ok(StateOutcome {
        snapshot: SnapshotPolicy::OnSuccess,
    })
}

#[derive(Clone, Debug, Default)]
struct HexStringParser;

impl RpcResponseParser for HexStringParser {
    fn parse(
        &self,
        _state_id: &StateId,
        response: &serde_json::Value,
    ) -> Result<serde_json::Value, StateError> {
        let value = parse_hex_string_response(response).map_err(parse_error)?;
        Ok(serde_json::json!(value))
    }
}

#[derive(Clone, Debug, Default)]
struct U256HexParser;

impl RpcResponseParser for U256HexParser {
    fn parse(
        &self,
        _state_id: &StateId,
        response: &serde_json::Value,
    ) -> Result<serde_json::Value, StateError> {
        let value = parse_u256_hex_response(response).map_err(parse_error)?;
        Ok(serde_json::json!(value))
    }
}

#[derive(Clone, Debug)]
struct U64HexParser {
    expectation: Option<U64Expectation>,
}

impl RpcResponseParser for U64HexParser {
    fn parse(
        &self,
        state_id: &StateId,
        response: &serde_json::Value,
    ) -> Result<serde_json::Value, StateError> {
        let value = parse_u64_hex_value(response)
            .map_err(|_| state_unknown("evm_response_invalid", "evm response was not a hex u64"))?;

        if let Some(expectation) = &self.expectation {
            if value != expectation.expected {
                return Err(state_error_with_state(
                    state_id.clone(),
                    expectation.mismatch_code,
                    expectation.mismatch_category.clone(),
                    expectation.mismatch_retryable,
                    expectation.mismatch_message,
                ));
            }
        }

        Ok(serde_json::json!(value))
    }
}

/// Expected `u64` response contract for read states that validate a scalar result.
#[derive(Clone, Debug)]
pub struct U64Expectation {
    /// Expected `u64` value.
    pub expected: u64,
    /// Error code used when the actual value mismatches `expected`.
    pub mismatch_code: &'static str,
    /// Error category used when the actual value mismatches `expected`.
    pub mismatch_category: ErrorCategory,
    /// Whether the mismatch is retryable.
    pub mismatch_retryable: bool,
    /// Human-readable mismatch message.
    pub mismatch_message: &'static str,
}

impl U64Expectation {
    /// Builds a non-retryable parsing-input expectation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use mfm_evm_runtime::states::read::U64Expectation;
    /// use mfm_machine::errors::ErrorCategory;
    ///
    /// let expectation = U64Expectation::parsing_input(
    ///     1,
    ///     "chain_id_mismatch",
    ///     "rpc chain id did not match expected_chain_id",
    /// );
    ///
    /// assert_eq!(expectation.expected, 1);
    /// assert_eq!(expectation.mismatch_category, ErrorCategory::ParsingInput);
    /// assert!(!expectation.mismatch_retryable);
    /// ```
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

/// Generic state that reads a hex-string RPC response into context.
#[derive(Clone, Debug)]
pub struct ReadHexStringState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Stable network identifier that owns the RPC call.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// JSON-RPC method to invoke.
    pub method: String,
    /// JSON-RPC params to pass to `method`.
    pub params: serde_json::Value,
    /// Context key that receives the parsed result.
    pub output_key: ContextKey,
}

impl ReadHexStringState {
    /// Creates a new hex-string read state.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use mfm_evm_runtime::states::read::ReadHexStringState;
    /// use mfm_machine::ids::{ContextKey, StateId};
    ///
    /// let state = ReadHexStringState::new(
    ///     StateId::must_new("evm.main.client_version".to_string()),
    ///     "ethereum-mainnet",
    ///     "web3_clientVersion",
    ///     serde_json::json!([]),
    ///     ContextKey("client_version".to_string()),
    /// );
    ///
    /// assert_eq!(state.method, "web3_clientVersion");
    /// assert_eq!(state.output_key.0, "client_version");
    /// ```
    pub fn new(
        state_id: StateId,
        network_id: impl Into<String>,
        method: impl Into<String>,
        params: serde_json::Value,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            network_id: network_id.into(),
            control_scope: default_control_scope(),
            method: method.into(),
            params,
            output_key,
        }
    }

    /// Overrides the control-plane scope used for the RPC call.
    pub fn with_control_scope(mut self, control_scope: impl Into<String>) -> Self {
        self.control_scope = control_scope.into();
        self
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
        execute_rpc_read(
            &self.state_id,
            &self.network_id,
            &self.control_scope,
            &self.method,
            &self.params,
            &self.output_key,
            &HexStringParser,
            ctx,
            io,
        )
        .await
    }
}

/// Generic state that reads a U256 hex quantity RPC response into context.
#[derive(Clone, Debug)]
pub struct ReadU256HexState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Stable network identifier that owns the RPC call.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// JSON-RPC method to invoke.
    pub method: String,
    /// JSON-RPC params to pass to `method`.
    pub params: serde_json::Value,
    /// Context key that receives the parsed result.
    pub output_key: ContextKey,
}

impl ReadU256HexState {
    /// Creates a new U256-hex read state.
    pub fn new(
        state_id: StateId,
        network_id: impl Into<String>,
        method: impl Into<String>,
        params: serde_json::Value,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            network_id: network_id.into(),
            control_scope: default_control_scope(),
            method: method.into(),
            params,
            output_key,
        }
    }

    /// Overrides the control-plane scope used for the RPC call.
    pub fn with_control_scope(mut self, control_scope: impl Into<String>) -> Self {
        self.control_scope = control_scope.into();
        self
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
        execute_rpc_read(
            &self.state_id,
            &self.network_id,
            &self.control_scope,
            &self.method,
            &self.params,
            &self.output_key,
            &U256HexParser,
            ctx,
            io,
        )
        .await
    }
}

/// Output decoding mode for [`EthCallState`].
#[derive(Clone, Debug, Default)]
pub enum EthCallDecode {
    /// Preserve the raw hex string.
    #[default]
    HexString,
    /// Parse the result as a hex-encoded `u64`.
    U64,
    /// Parse the result as a hex-encoded U256 quantity string.
    U256Hex,
}

/// State that executes `eth_call` and writes the decoded response to context.
#[derive(Clone, Debug)]
pub struct EthCallState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Stable network identifier that owns the RPC call.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Contract address targeted by the call.
    pub to: String,
    /// Hex-encoded calldata.
    pub data: String,
    /// Block selector passed as the second `eth_call` parameter.
    pub block: serde_json::Value,
    /// Context key that receives the parsed result.
    pub output_key: ContextKey,
    /// Decoding mode applied to the RPC response.
    pub decode: EthCallDecode,
}

impl EthCallState {
    /// Creates a new `eth_call` state that defaults to the `latest` block and hex-string decode.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use mfm_evm_runtime::states::read::{EthCallDecode, EthCallState};
    /// use mfm_machine::ids::{ContextKey, StateId};
    ///
    /// let state = EthCallState::new(
    ///     StateId::must_new("evm.main.call".to_string()),
    ///     "ethereum-mainnet",
    ///     "0x0000000000000000000000000000000000000001",
    ///     "0x70a08231",
    ///     ContextKey("call_result".to_string()),
    /// )
    /// .with_decode(EthCallDecode::HexString);
    ///
    /// assert_eq!(state.output_key.0, "call_result");
    /// assert_eq!(state.block, serde_json::json!("latest"));
    /// ```
    pub fn new(
        state_id: StateId,
        network_id: impl Into<String>,
        to: impl Into<String>,
        data: impl Into<String>,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            network_id: network_id.into(),
            control_scope: default_control_scope(),
            to: to.into(),
            data: data.into(),
            block: serde_json::json!("latest"),
            output_key,
            decode: EthCallDecode::HexString,
        }
    }

    /// Overrides the control-plane scope used for the RPC call.
    pub fn with_control_scope(mut self, control_scope: impl Into<String>) -> Self {
        self.control_scope = control_scope.into();
        self
    }

    /// Overrides the block selector used by the `eth_call`.
    pub fn with_block(mut self, block: serde_json::Value) -> Self {
        self.block = block;
        self
    }

    /// Overrides the response decoding mode.
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
            .call(managed_call(
                &self.network_id,
                &self.control_scope,
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
                serde_json::json!(parse_hex_string_response(&res.response).map_err(parse_error)?)
            }
            EthCallDecode::U64 => serde_json::json!(parse_u64_hex_value(&res.response).map_err(
                |_| state_unknown("evm_response_invalid", "evm response was not a hex u64")
            )?),
            EthCallDecode::U256Hex => {
                serde_json::json!(parse_u256_hex_response(&res.response).map_err(parse_error)?)
            }
        };

        write_json(ctx, self.output_key.clone(), parsed)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Generic state that reads a `u64` hex quantity RPC response into context.
#[derive(Clone, Debug)]
pub struct ReadU64HexState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Stable network identifier that owns the RPC call.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// JSON-RPC method to invoke.
    pub method: String,
    /// JSON-RPC params to pass to `method`.
    pub params: serde_json::Value,
    /// Context key that receives the parsed result.
    pub output_key: ContextKey,
    /// Optional expectation enforced against the parsed value.
    pub expectation: Option<U64Expectation>,
}

impl ReadU64HexState {
    /// Creates a new `u64`-hex read state.
    pub fn new(
        state_id: StateId,
        network_id: impl Into<String>,
        method: impl Into<String>,
        params: serde_json::Value,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            network_id: network_id.into(),
            control_scope: default_control_scope(),
            method: method.into(),
            params,
            output_key,
            expectation: None,
        }
    }

    /// Overrides the control-plane scope used for the RPC call.
    pub fn with_control_scope(mut self, control_scope: impl Into<String>) -> Self {
        self.control_scope = control_scope.into();
        self
    }

    /// Configures an exact-value expectation for the parsed response.
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
        execute_rpc_read(
            &self.state_id,
            &self.network_id,
            &self.control_scope,
            &self.method,
            &self.params,
            &self.output_key,
            &U64HexParser {
                expectation: self.expectation.clone(),
            },
            ctx,
            io,
        )
        .await
    }
}

/// State that reads the native-token balance for a wallet at a context-provided block.
#[derive(Clone, Debug)]
pub struct NativeBalanceState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Stable network identifier that owns the RPC call.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Wallet address whose native balance should be queried.
    pub wallet: Address,
    /// Context key that contains the block number to query against.
    pub block_key: ContextKey,
    /// Context key that receives the formatted balance object.
    pub output_key: ContextKey,
    /// Native token symbol written into the output object.
    pub symbol: String,
    /// Native token decimals written into the output object.
    pub decimals: u8,
}

impl NativeBalanceState {
    /// Creates a native-balance state with `ETH`/18-decimal defaults.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use alloy_primitives::Address;
    /// use mfm_evm_runtime::states::read::NativeBalanceState;
    /// use mfm_machine::ids::{ContextKey, StateId};
    ///
    /// let state = NativeBalanceState::new(
    ///     StateId::must_new("evm.main.native_balance".to_string()),
    ///     "ethereum-mainnet",
    ///     Address::from([0u8; 20]),
    ///     ContextKey("block_number".to_string()),
    ///     ContextKey("native_balance".to_string()),
    /// );
    ///
    /// assert_eq!(state.symbol, "ETH");
    /// assert_eq!(state.decimals, 18);
    /// ```
    pub fn new(
        state_id: StateId,
        network_id: impl Into<String>,
        wallet: Address,
        block_key: ContextKey,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            network_id: network_id.into(),
            control_scope: default_control_scope(),
            wallet,
            block_key,
            output_key,
            symbol: "ETH".to_string(),
            decimals: 18,
        }
    }

    /// Overrides the control-plane scope used for the RPC call.
    pub fn with_control_scope(mut self, control_scope: impl Into<String>) -> Self {
        self.control_scope = control_scope.into();
        self
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
            .call(managed_call(
                &self.network_id,
                &self.control_scope,
                "eth_getBalance",
                serde_json::json!([address_hex_lower(&self.wallet), u64_hex_quantity(block)]),
            ))
            .await
            .map_err(state_from_io)?;

        let wei = parse_u256_hex_value(&res.response).map_err(parse_error)?;
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

/// State that reads an ERC-20 token balance for a wallet at a context-provided block.
#[derive(Clone, Debug)]
pub struct TokenBalanceState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Stable network identifier that owns the RPC call.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Token contract address.
    pub token: Address,
    /// Wallet address whose token balance should be queried.
    pub wallet: Address,
    /// Optional token symbol to surface in the output object.
    pub symbol: Option<String>,
    /// Optional token decimals override; fetched on-chain when absent.
    pub decimals: Option<u8>,
    /// Context key that contains the block number to query against.
    pub block_key: ContextKey,
    /// Context key that receives the formatted balance object.
    pub output_key: ContextKey,
}

impl TokenBalanceState {
    /// Creates a token-balance state.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_id: StateId,
        network_id: impl Into<String>,
        token: Address,
        wallet: Address,
        symbol: Option<String>,
        decimals: Option<u8>,
        block_key: ContextKey,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            network_id: network_id.into(),
            control_scope: default_control_scope(),
            token,
            wallet,
            symbol,
            decimals,
            block_key,
            output_key,
        }
    }

    /// Overrides the control-plane scope used for the RPC call.
    pub fn with_control_scope(mut self, control_scope: impl Into<String>) -> Self {
        self.control_scope = control_scope.into();
        self
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
                    .call(managed_call(
                        &self.network_id,
                        &self.control_scope,
                        "eth_call",
                        serde_json::json!([
                            {"to": token_to, "data": encode_erc20_decimals()},
                            at_block.clone()
                        ]),
                    ))
                    .await
                    .map_err(state_from_io)?;
                let v = parse_u256_hex_value(&res.response).map_err(parse_error)?;
                parse_u8_u256(v).map_err(parse_error)?
            }
        };

        let res = client
            .call(managed_call(
                &self.network_id,
                &self.control_scope,
                "eth_call",
                serde_json::json!([
                    {"to": address_hex_lower(&self.token), "data": encode_erc20_balance_of(&self.wallet)},
                    at_block
                ]),
            ))
            .await
            .map_err(state_from_io)?;

        let raw = parse_u256_hex_value(&res.response).map_err(parse_error)?;
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
#[path = "tests/read_tests.rs"]
mod read_tests;
