#![warn(missing_docs)]
//! Strict bounded Bitcoin Core 28+ JSON-RPC session for aggregate balance collection.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bitcoin::{Amount, BlockHash, Txid};
use mfm_bitcoin::{
    BitcoinAddressBalance, BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionResponse,
    BitcoinBalanceSession, BitcoinCapabilityError, BitcoinSessionFuture, BitcoinSourceBinding,
    BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
};
use mfm_capabilities::ProviderDiagnosticCode;
use reqwest::header::CONTENT_TYPE;
use serde::de::{DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const ORDINARY_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MIN_SCAN_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_SCAN_TIMEOUT: Duration = Duration::from_secs(86_400);
const MAX_JSON_RPC_ERROR_MESSAGE_BYTES: usize = 1_024;
const MAX_DESCRIPTOR_BYTES: usize = 4_096;
const MAX_SCRIPT_BYTES: usize = 10_000;
const MAX_BITCOIN_JSON_RPC_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Resolved Basic authentication for one Bitcoin RPC endpoint.
#[derive(Clone)]
pub struct BitcoinRpcAuthentication {
    username: String,
    password: String,
}

impl BitcoinRpcAuthentication {
    /// Creates resolved Basic authentication. Empty components are rejected.
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, BitcoinRpcError> {
        let authentication = Self {
            username: username.into(),
            password: password.into(),
        };
        if authentication.username.is_empty() || authentication.password.is_empty() {
            return Err(BitcoinRpcError::InvalidConfiguration);
        }
        Ok(authentication)
    }
}

impl fmt::Debug for BitcoinRpcAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BitcoinRpcAuthentication")
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .finish()
    }
}

/// One checked endpoint-bound Bitcoin Core session.
pub struct BitcoinRpcSession {
    endpoint: reqwest::Url,
    authentication: Option<BitcoinRpcAuthentication>,
    binding: BitcoinSourceBinding,
    scan_timeout: Duration,
    client: reqwest::Client,
    next_request_id: AtomicU64,
}

impl BitcoinRpcSession {
    /// Creates a session from resolved endpoint, authentication, binding, and scan deadline.
    pub fn new(
        endpoint: impl AsRef<str>,
        authentication: Option<BitcoinRpcAuthentication>,
        binding: BitcoinSourceBinding,
        scan_timeout: Duration,
    ) -> Result<Self, BitcoinRpcError> {
        if !(MIN_SCAN_TIMEOUT..=MAX_SCAN_TIMEOUT).contains(&scan_timeout) {
            return Err(BitcoinRpcError::InvalidConfiguration);
        }
        let endpoint = reqwest::Url::parse(endpoint.as_ref())
            .map_err(|_| BitcoinRpcError::InvalidConfiguration)?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(BitcoinRpcError::InvalidConfiguration);
        }
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| BitcoinRpcError::InvalidConfiguration)?;
        Ok(Self {
            endpoint,
            authentication,
            binding,
            scan_timeout,
            client,
            next_request_id: AtomicU64::new(1),
        })
    }

    /// Returns the exact checked semantic binding of this endpoint.
    pub const fn binding(&self) -> &BitcoinSourceBinding {
        &self.binding
    }

    async fn execute_collection(
        &self,
        request: &BitcoinBalanceCollectionRequest,
    ) -> Result<BitcoinBalanceCollectionResponse, BitcoinRpcError> {
        if request.binding() != &self.binding {
            return Err(BitcoinRpcError::SourceMismatch);
        }

        let info: BlockchainInfo = self
            .rpc(
                "getblockchaininfo",
                serde_json::json!([]),
                ORDINARY_REQUEST_TIMEOUT,
            )
            .await?;
        if info.chain != self.binding.bitcoin_network().as_str() || info.initial_block_download {
            return Err(BitcoinRpcError::SourceMismatch);
        }

        let descriptors = request
            .addresses()
            .iter()
            .map(|address| address.scan_descriptor())
            .collect::<Vec<_>>();
        let scan: ScanTxOutSetResult = self
            .rpc(
                "scantxoutset",
                serde_json::json!(["start", descriptors]),
                self.scan_timeout,
            )
            .await?;
        let reduced = reduce_scan(request, scan)?;

        let final_hash: String = self
            .rpc(
                "getblockhash",
                serde_json::json!([reduced.height]),
                ORDINARY_REQUEST_TIMEOUT,
            )
            .await?;
        let final_hash =
            BlockHash::from_str(&final_hash).map_err(|_| BitcoinRpcError::ResponseInvalid)?;
        if final_hash != reduced.anchor_hash {
            return Err(BitcoinRpcError::Reorganization);
        }

        Ok(BitcoinBalanceCollectionResponse::new(
            self.binding.clone(),
            BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
            reduced.height,
            reduced.anchor_hash,
            reduced.balances,
            final_hash,
        ))
    }

    async fn rpc<T>(
        &self,
        method: &'static str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<T, BitcoinRpcError>
    where
        T: DeserializeOwned,
    {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let body = RpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };
        let mut builder = self
            .client
            .post(self.endpoint.clone())
            .header(CONTENT_TYPE, "application/json")
            .timeout(timeout)
            .json(&body);
        if let Some(authentication) = &self.authentication {
            builder = builder.basic_auth(&authentication.username, Some(&authentication.password));
        }
        let mut response = builder.send().await.map_err(classify_reqwest_error)?;
        let status = response.status();
        if let Some(length) = response.content_length() {
            if length > MAX_BITCOIN_JSON_RPC_BODY_BYTES as u64 {
                return Err(BitcoinRpcError::BodyTooLarge);
            }
        }
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .unwrap_or(0)
                .min(MAX_BITCOIN_JSON_RPC_BODY_BYTES as u64) as usize,
        );
        while let Some(chunk) = response.chunk().await.map_err(classify_reqwest_error)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_BITCOIN_JSON_RPC_BODY_BYTES {
                return Err(BitcoinRpcError::BodyTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(BitcoinRpcError::HttpStatus(status.as_u16()));
        }

        let envelope: RpcEnvelope = decode_unique(&bytes)?;
        if envelope.jsonrpc.as_deref() != Some("2.0") || envelope.id != Some(id) {
            return Err(BitcoinRpcError::ProtocolViolation);
        }
        match (envelope.result, envelope.error) {
            (Some(result), None) => decode_unique(result.get().as_bytes()),
            (None, Some(error)) => {
                let scan_busy = method == "scantxoutset"
                    && error.code == -8
                    && error.message.starts_with("Scan already in progress");
                Err(BitcoinRpcError::Rpc {
                    code: error.code,
                    scan_busy,
                })
            }
            _ => Err(BitcoinRpcError::ProtocolViolation),
        }
    }
}

impl fmt::Debug for BitcoinRpcSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BitcoinRpcSession")
            .field("endpoint", &"<redacted>")
            .field(
                "authentication",
                &self.authentication.as_ref().map(|_| "<redacted>"),
            )
            .field("binding", &self.binding)
            .field("scan_timeout", &self.scan_timeout)
            .finish_non_exhaustive()
    }
}

impl BitcoinBalanceSession for BitcoinRpcSession {
    fn implementation_id(&self) -> &'static str {
        BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID
    }

    fn validate_binding(
        &self,
        binding: &BitcoinSourceBinding,
    ) -> Result<(), BitcoinCapabilityError> {
        if binding == &self.binding {
            Ok(())
        } else {
            Err(BitcoinCapabilityError::SourceMismatch)
        }
    }

    fn collect_balances<'a>(
        &'a self,
        request: &'a BitcoinBalanceCollectionRequest,
    ) -> BitcoinSessionFuture<'a, BitcoinBalanceCollectionResponse> {
        Box::pin(async move {
            self.execute_collection(request)
                .await
                .map_err(capability_error)
        })
    }
}

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: serde_json::Value,
}

struct RpcEnvelope {
    jsonrpc: Option<String>,
    id: Option<u64>,
    result: Option<Box<serde_json::value::RawValue>>,
    error: Option<RpcErrorObject>,
}

impl<'de> Deserialize<'de> for RpcEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EnvelopeVisitor;
        impl<'de> Visitor<'de> for EnvelopeVisitor {
            type Value = RpcEnvelope;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a unique-member JSON-RPC 2.0 envelope")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = BTreeSet::new();
                let mut jsonrpc = None;
                let mut id = None;
                let mut result = None;
                let mut error = None;
                while let Some(key) = map.next_key::<String>()? {
                    require_unique::<A::Error>(&mut seen, &key)?;
                    match key.as_str() {
                        "jsonrpc" => jsonrpc = Some(map.next_value()?),
                        "id" => id = Some(map.next_value()?),
                        "result" => result = Some(map.next_value()?),
                        "error" => error = Some(map.next_value()?),
                        _ => {
                            map.next_value::<DuplicateRejectingIgnored>()?;
                        }
                    }
                }
                Ok(RpcEnvelope {
                    jsonrpc,
                    id,
                    result,
                    error,
                })
            }
        }
        deserializer.deserialize_map(EnvelopeVisitor)
    }
}

struct RpcErrorObject {
    code: i64,
    message: String,
}

impl<'de> Deserialize<'de> for RpcErrorObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ErrorVisitor;
        impl<'de> Visitor<'de> for ErrorVisitor {
            type Value = RpcErrorObject;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON-RPC error object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = BTreeSet::new();
                let mut code = None;
                let mut message: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    require_unique::<A::Error>(&mut seen, &key)?;
                    match key.as_str() {
                        "code" => code = Some(map.next_value()?),
                        "message" => message = Some(map.next_value()?),
                        _ => {
                            map.next_value::<DuplicateRejectingIgnored>()?;
                        }
                    }
                }
                let code = code.ok_or_else(|| serde::de::Error::missing_field("code"))?;
                let message = message.ok_or_else(|| serde::de::Error::missing_field("message"))?;
                if message.len() > MAX_JSON_RPC_ERROR_MESSAGE_BYTES {
                    return Err(serde::de::Error::custom(
                        "JSON-RPC error message was oversized",
                    ));
                }
                Ok(RpcErrorObject { code, message })
            }
        }
        deserializer.deserialize_map(ErrorVisitor)
    }
}

struct DuplicateRejectingIgnored;

impl<'de> Deserialize<'de> for DuplicateRejectingIgnored {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IgnoreVisitor;
        impl<'de> Visitor<'de> for IgnoreVisitor {
            type Value = DuplicateRejectingIgnored;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("any JSON value with unique object members")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    require_unique::<A::Error>(&mut seen, &key)?;
                    map.next_value::<DuplicateRejectingIgnored>()?;
                }
                Ok(DuplicateRejectingIgnored)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                while sequence
                    .next_element::<DuplicateRejectingIgnored>()?
                    .is_some()
                {}
                Ok(DuplicateRejectingIgnored)
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
                Ok(DuplicateRejectingIgnored)
            }
            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
                Ok(DuplicateRejectingIgnored)
            }
            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
                Ok(DuplicateRejectingIgnored)
            }
            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
                Ok(DuplicateRejectingIgnored)
            }
            fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
                Ok(DuplicateRejectingIgnored)
            }
            fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
                Ok(DuplicateRejectingIgnored)
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(DuplicateRejectingIgnored)
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(DuplicateRejectingIgnored)
            }
            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }
            fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }
        }
        deserializer.deserialize_any(IgnoreVisitor)
    }
}

struct BlockchainInfo {
    chain: String,
    initial_block_download: bool,
}

impl<'de> Deserialize<'de> for BlockchainInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct InfoVisitor;
        impl<'de> Visitor<'de> for InfoVisitor {
            type Value = BlockchainInfo;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Bitcoin Core blockchain-info result")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = BTreeSet::new();
                let mut chain = None;
                let mut initial_block_download = None;
                while let Some(key) = map.next_key::<String>()? {
                    require_unique::<A::Error>(&mut seen, &key)?;
                    match key.as_str() {
                        "chain" => chain = Some(map.next_value()?),
                        "initialblockdownload" => initial_block_download = Some(map.next_value()?),
                        _ => {
                            map.next_value::<DuplicateRejectingIgnored>()?;
                        }
                    }
                }
                Ok(BlockchainInfo {
                    chain: chain.ok_or_else(|| serde::de::Error::missing_field("chain"))?,
                    initial_block_download: initial_block_download
                        .ok_or_else(|| serde::de::Error::missing_field("initialblockdownload"))?,
                })
            }
        }
        deserializer.deserialize_map(InfoVisitor)
    }
}

struct ScanTxOutSetResult {
    success: bool,
    height: u64,
    best_block: String,
    txouts: u64,
    unspents: Vec<ScanUnspent>,
    total_amount: Box<serde_json::value::RawValue>,
}

impl<'de> Deserialize<'de> for ScanTxOutSetResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ScanVisitor;
        impl<'de> Visitor<'de> for ScanVisitor {
            type Value = ScanTxOutSetResult;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Bitcoin Core scantxoutset result")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = BTreeSet::new();
                let mut success = None;
                let mut height = None;
                let mut best_block = None;
                let mut txouts = None;
                let mut unspents = None;
                let mut total_amount = None;
                while let Some(key) = map.next_key::<String>()? {
                    require_unique::<A::Error>(&mut seen, &key)?;
                    match key.as_str() {
                        "success" => success = Some(map.next_value()?),
                        "height" => height = Some(map.next_value()?),
                        "bestblock" => best_block = Some(map.next_value()?),
                        "txouts" => txouts = Some(map.next_value()?),
                        "unspents" => unspents = Some(map.next_value()?),
                        "total_amount" => total_amount = Some(map.next_value()?),
                        _ => {
                            map.next_value::<DuplicateRejectingIgnored>()?;
                        }
                    }
                }
                Ok(ScanTxOutSetResult {
                    success: success.ok_or_else(|| serde::de::Error::missing_field("success"))?,
                    height: height.ok_or_else(|| serde::de::Error::missing_field("height"))?,
                    best_block: best_block
                        .ok_or_else(|| serde::de::Error::missing_field("bestblock"))?,
                    txouts: txouts.ok_or_else(|| serde::de::Error::missing_field("txouts"))?,
                    unspents: unspents
                        .ok_or_else(|| serde::de::Error::missing_field("unspents"))?,
                    total_amount: total_amount
                        .ok_or_else(|| serde::de::Error::missing_field("total_amount"))?,
                })
            }
        }
        deserializer.deserialize_map(ScanVisitor)
    }
}

struct ScanUnspent {
    txid: String,
    vout: u64,
    script_pub_key: String,
    descriptor: String,
    amount: Box<serde_json::value::RawValue>,
    height: u64,
}

impl<'de> Deserialize<'de> for ScanUnspent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UnspentVisitor;
        impl<'de> Visitor<'de> for UnspentVisitor {
            type Value = ScanUnspent;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Bitcoin Core scan unspent")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = BTreeSet::new();
                let mut txid = None;
                let mut vout = None;
                let mut script_pub_key = None;
                let mut descriptor = None;
                let mut amount = None;
                let mut height = None;
                while let Some(key) = map.next_key::<String>()? {
                    require_unique::<A::Error>(&mut seen, &key)?;
                    match key.as_str() {
                        "txid" => txid = Some(map.next_value()?),
                        "vout" => vout = Some(map.next_value()?),
                        "scriptPubKey" => script_pub_key = Some(map.next_value()?),
                        "desc" => descriptor = Some(map.next_value()?),
                        "amount" => amount = Some(map.next_value()?),
                        "height" => height = Some(map.next_value()?),
                        _ => {
                            map.next_value::<DuplicateRejectingIgnored>()?;
                        }
                    }
                }
                Ok(ScanUnspent {
                    txid: txid.ok_or_else(|| serde::de::Error::missing_field("txid"))?,
                    vout: vout.ok_or_else(|| serde::de::Error::missing_field("vout"))?,
                    script_pub_key: script_pub_key
                        .ok_or_else(|| serde::de::Error::missing_field("scriptPubKey"))?,
                    descriptor: descriptor
                        .ok_or_else(|| serde::de::Error::missing_field("desc"))?,
                    amount: amount.ok_or_else(|| serde::de::Error::missing_field("amount"))?,
                    height: height.ok_or_else(|| serde::de::Error::missing_field("height"))?,
                })
            }
        }
        deserializer.deserialize_map(UnspentVisitor)
    }
}

struct ReducedScan {
    height: u64,
    anchor_hash: BlockHash,
    balances: Vec<BitcoinAddressBalance>,
}

fn reduce_scan(
    request: &BitcoinBalanceCollectionRequest,
    scan: ScanTxOutSetResult,
) -> Result<ReducedScan, BitcoinRpcError> {
    if !scan.success {
        return Err(BitcoinRpcError::OperationIncomplete);
    }
    let _validated_txouts = scan.txouts;
    let anchor_hash =
        BlockHash::from_str(&scan.best_block).map_err(|_| BitcoinRpcError::ResponseInvalid)?;
    let mut script_to_index = BTreeMap::new();
    for (index, address) in request.addresses().iter().enumerate() {
        if script_to_index
            .insert(address.script_pubkey().as_bytes().to_vec(), index)
            .is_some()
        {
            return Err(BitcoinRpcError::ResponseInvalid);
        }
    }
    let mut balances = vec![0_u64; request.addresses().len()];
    let mut outpoints = BTreeSet::new();
    let mut observed_total = 0_u64;
    for unspent in scan.unspents {
        let txid = Txid::from_str(&unspent.txid).map_err(|_| BitcoinRpcError::ResponseInvalid)?;
        let vout = u32::try_from(unspent.vout).map_err(|_| BitcoinRpcError::ResponseInvalid)?;
        if !outpoints.insert((txid, vout)) || unspent.height > scan.height {
            return Err(BitcoinRpcError::ResponseInvalid);
        }
        if unspent.descriptor.len() > MAX_DESCRIPTOR_BYTES {
            return Err(BitcoinRpcError::ResponseInvalid);
        }
        let script = decode_script(&unspent.script_pub_key)?;
        let index = script_to_index
            .get(&script)
            .copied()
            .ok_or(BitcoinRpcError::ResponseInvalid)?;
        let amount = parse_btc_amount(unspent.amount.get())?;
        balances[index] = balances[index]
            .checked_add(amount)
            .filter(|subtotal| *subtotal <= Amount::MAX_MONEY.to_sat())
            .ok_or(BitcoinRpcError::ResponseInvalid)?;
        observed_total = observed_total
            .checked_add(amount)
            .filter(|total| *total <= Amount::MAX_MONEY.to_sat())
            .ok_or(BitcoinRpcError::ResponseInvalid)?;
    }
    if observed_total != parse_btc_amount(scan.total_amount.get())? {
        return Err(BitcoinRpcError::ResponseInvalid);
    }
    let balances = request
        .addresses()
        .iter()
        .zip(balances)
        .map(|(address, sats)| BitcoinAddressBalance::new(address.as_str().to_owned(), sats))
        .collect();
    Ok(ReducedScan {
        height: scan.height,
        anchor_hash,
        balances,
    })
}

fn decode_script(value: &str) -> Result<Vec<u8>, BitcoinRpcError> {
    if value.len() > MAX_SCRIPT_BYTES * 2 || !value.len().is_multiple_of(2) {
        return Err(BitcoinRpcError::ResponseInvalid);
    }
    hex::decode(value).map_err(|_| BitcoinRpcError::ResponseInvalid)
}

fn parse_btc_amount(raw: &str) -> Result<u64, BitcoinRpcError> {
    if raw.is_empty()
        || raw.starts_with(['-', '+'])
        || raw.contains(['e', 'E'])
        || raw.matches('.').count() > 1
    {
        return Err(BitcoinRpcError::ResponseInvalid);
    }
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 8
    {
        return Err(BitcoinRpcError::ResponseInvalid);
    }
    let whole = whole
        .parse::<u64>()
        .map_err(|_| BitcoinRpcError::ResponseInvalid)?;
    let mut fractional = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .map_err(|_| BitcoinRpcError::ResponseInvalid)?
    };
    for _ in fraction.len()..8 {
        fractional = fractional
            .checked_mul(10)
            .ok_or(BitcoinRpcError::ResponseInvalid)?;
    }
    let sats = whole
        .checked_mul(100_000_000)
        .and_then(|value| value.checked_add(fractional))
        .filter(|value| *value <= Amount::MAX_MONEY.to_sat())
        .ok_or(BitcoinRpcError::ResponseInvalid)?;
    Ok(sats)
}

fn decode_unique<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, BitcoinRpcError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(&mut deserializer).map_err(|_| BitcoinRpcError::ResponseInvalid)?;
    deserializer
        .end()
        .map_err(|_| BitcoinRpcError::ResponseInvalid)?;
    Ok(value)
}

fn require_unique<E>(seen: &mut BTreeSet<String>, key: &str) -> Result<(), E>
where
    E: serde::de::Error,
{
    if seen.insert(key.to_owned()) {
        Ok(())
    } else {
        Err(E::custom("duplicate JSON object member"))
    }
}

fn classify_reqwest_error(error: reqwest::Error) -> BitcoinRpcError {
    if error.is_timeout() {
        BitcoinRpcError::Timeout
    } else {
        BitcoinRpcError::Transport
    }
}

fn capability_error(error: BitcoinRpcError) -> BitcoinCapabilityError {
    match error {
        BitcoinRpcError::SourceMismatch => BitcoinCapabilityError::SourceMismatch,
        BitcoinRpcError::Timeout | BitcoinRpcError::Transport => BitcoinCapabilityError::provider(
            ProviderDiagnosticCode::TransportFailed,
            "aggregate_read",
            true,
        ),
        BitcoinRpcError::HttpStatus(status) => BitcoinCapabilityError::provider(
            ProviderDiagnosticCode::RpcHttpStatus,
            "aggregate_read",
            status >= 500,
        ),
        BitcoinRpcError::Rpc {
            scan_busy: true, ..
        } => BitcoinCapabilityError::provider(
            ProviderDiagnosticCode::RpcJsonError,
            "scantxoutset",
            true,
        ),
        BitcoinRpcError::Rpc { .. } => BitcoinCapabilityError::provider(
            ProviderDiagnosticCode::RpcJsonError,
            "aggregate_read",
            false,
        ),
        BitcoinRpcError::OperationIncomplete => BitcoinCapabilityError::provider(
            ProviderDiagnosticCode::OperationIncomplete,
            "scantxoutset",
            false,
        ),
        BitcoinRpcError::InvalidConfiguration => BitcoinCapabilityError::provider(
            ProviderDiagnosticCode::ProviderConfigurationInvalid,
            "session",
            false,
        ),
        BitcoinRpcError::BodyTooLarge
        | BitcoinRpcError::ProtocolViolation
        | BitcoinRpcError::ResponseInvalid
        | BitcoinRpcError::Reorganization => BitcoinCapabilityError::provider(
            ProviderDiagnosticCode::ResponseInvalid,
            "aggregate_read",
            false,
        ),
    }
}

/// Redacted transport failure. No variant stores URLs, credentials, bodies, or provider messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BitcoinRpcError {
    /// Resolved endpoint/authentication/timeout input was invalid.
    #[error("Bitcoin RPC configuration was invalid")]
    InvalidConfiguration,
    /// Selected session binding did not match the request.
    #[error("Bitcoin RPC source binding did not match")]
    SourceMismatch,
    /// HTTP request failed before a response was available.
    #[error("Bitcoin RPC transport failed")]
    Transport,
    /// Request exceeded its explicit overall deadline.
    #[error("Bitcoin RPC request timed out")]
    Timeout,
    /// Provider returned a non-200 response.
    #[error("Bitcoin RPC returned HTTP status {0}")]
    HttpStatus(u16),
    /// Response exceeded the supported body ceiling.
    #[error("Bitcoin RPC response exceeded the supported size")]
    BodyTooLarge,
    /// JSON-RPC 2.0 envelope contract was violated.
    #[error("Bitcoin RPC envelope was invalid")]
    ProtocolViolation,
    /// JSON-RPC error was returned; provider text is deliberately discarded.
    #[error("Bitcoin RPC returned error code {code}")]
    Rpc {
        /// Numeric public protocol code.
        code: i64,
        /// Whether this was the exact scan-busy classification.
        scan_busy: bool,
    },
    /// Typed result failed strict semantic validation.
    #[error("Bitcoin RPC response was invalid")]
    ResponseInvalid,
    /// Scan did not report successful completion.
    #[error("Bitcoin RPC scan did not complete")]
    OperationIncomplete,
    /// Scan anchor was no longer canonical.
    #[error("Bitcoin RPC scan anchor reorganized")]
    Reorganization,
}

#[cfg(test)]
mod tests;
