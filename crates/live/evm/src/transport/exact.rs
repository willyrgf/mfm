use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::ops::Range;
use std::str::FromStr;

use alloy_primitives::{Address, TxKind, B256, U256};
use mfm_evm::{
    EvmBlockAnchor, EvmWalletAccessListEntry, EvmWalletObservedTransaction, EvmWalletReceipt,
    EvmWalletReceiptLog, EvmWalletReceiptStatus, EvmWalletTransactionPlacement,
    TransientSignedEip1559Envelope, EVM_READ_MAX_RESPONSE_BYTES,
    EVM_WALLET_ACCESS_LIST_MAX_ENTRIES, EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS,
    EVM_WALLET_DATA_MAX_BYTES, EVM_WALLET_RECEIPT_LOG_DATA_MAX_BYTES, EVM_WALLET_RECEIPT_LOG_LIMIT,
    EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES, EVM_WALLET_TRANSACTION_TYPE,
};
use serde::Deserialize;
use serde_json::value::RawValue;
use zeroize::Zeroizing;

use super::{
    EVM_BLOCK_BY_NUMBER_METHOD, EVM_PENDING_NONCE_METHOD, EVM_RECEIPT_BY_HASH_METHOD,
    EVM_SEND_RAW_TRANSACTION_METHOD, EVM_TRANSACTION_BY_HASH_METHOD, EXACT_ALREADY_KNOWN_CODE,
    EXACT_ALREADY_KNOWN_MESSAGE,
};

#[cfg(test)]
tokio::task_local! {
    static ENCODE_PROBE: std::sync::Arc<std::sync::atomic::AtomicBool>;
}

#[cfg(test)]
pub(super) async fn with_encode_probe<Future>(
    probe: std::sync::Arc<std::sync::atomic::AtomicBool>,
    future: Future,
) -> Future::Output
where
    Future: std::future::Future,
{
    ENCODE_PROBE.scope(probe, future).await
}

const REQUEST_PREFIX: &[u8] = br#"{"jsonrpc":"2.0","id":1,"method":""#;
const PARAMS_PREFIX: &[u8] = br#"","params":"#;
const JSON_RPC_ERROR_MESSAGE_MAX_BYTES: usize = 16 * 1024;
const JSON_OBJECT_KEY_MAX_BYTES: usize = 16 * 1024;
const PROJECTION_MAX_DEPTH: usize = 64;
const PROJECTION_MAX_CONTAINER_ITEMS: usize = 4_096;
const HEX: &[u8; 16] = b"0123456789abcdef";

pub(super) enum ExactRpcRequest {
    ChainIdentity,
    LatestAnchor,
    NativeBalance {
        account: Address,
        block_hash: B256,
    },
    TokenDecimals {
        contract: Address,
        block_hash: B256,
    },
    TokenBalance {
        contract: Address,
        account: Address,
        block_hash: B256,
    },
    ConfirmAnchor {
        number: U256,
    },
    PendingNonce {
        sender: Address,
    },
    SendRawTransaction {
        signed: TransientSignedEip1559Envelope,
    },
    TransactionByHash {
        transaction_hash: B256,
    },
    ReceiptByHash {
        transaction_hash: B256,
    },
    FinalizedHead,
    InclusionBlock {
        number: U256,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExactRpcOperation {
    ChainIdentity,
    LatestAnchor,
    NativeBalance,
    TokenDecimals,
    TokenBalance,
    ConfirmAnchor,
    PendingNonce,
    SendRawTransaction,
    TransactionByHash,
    ReceiptByHash,
    FinalizedHead,
    InclusionBlock,
}

impl ExactRpcOperation {
    pub(super) const fn method(self) -> &'static str {
        match self {
            Self::ChainIdentity => "eth_chainId",
            Self::LatestAnchor | Self::FinalizedHead | Self::InclusionBlock => {
                EVM_BLOCK_BY_NUMBER_METHOD
            }
            Self::NativeBalance => "eth_getBalance",
            Self::TokenDecimals | Self::TokenBalance => "eth_call",
            Self::ConfirmAnchor => EVM_BLOCK_BY_NUMBER_METHOD,
            Self::PendingNonce => EVM_PENDING_NONCE_METHOD,
            Self::SendRawTransaction => EVM_SEND_RAW_TRANSACTION_METHOD,
            Self::TransactionByHash => EVM_TRANSACTION_BY_HASH_METHOD,
            Self::ReceiptByHash => EVM_RECEIPT_BY_HASH_METHOD,
        }
    }

    const fn is_read(self) -> bool {
        matches!(
            self,
            Self::ChainIdentity
                | Self::LatestAnchor
                | Self::NativeBalance
                | Self::TokenDecimals
                | Self::TokenBalance
                | Self::ConfirmAnchor
                | Self::PendingNonce
        )
    }
}

pub(super) struct EncodedRpcRequest {
    pub(super) operation: ExactRpcOperation,
    pub(super) body: Zeroizing<Vec<u8>>,
}

impl ExactRpcRequest {
    pub(super) fn encode(self) -> Result<EncodedRpcRequest, ()> {
        #[cfg(test)]
        let _ = ENCODE_PROBE.try_with(|probe| {
            probe.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let operation = self.operation();
        let params_len = self.params_len()?;
        let capacity = REQUEST_PREFIX
            .len()
            .checked_add(operation.method().len())
            .and_then(|length| length.checked_add(PARAMS_PREFIX.len()))
            .and_then(|length| length.checked_add(params_len))
            .and_then(|length| length.checked_add(1))
            .ok_or(())?;
        let mut body = Zeroizing::new(Vec::with_capacity(capacity));
        body.extend_from_slice(REQUEST_PREFIX);
        body.extend_from_slice(operation.method().as_bytes());
        body.extend_from_slice(PARAMS_PREFIX);
        match self {
            Self::ChainIdentity => body.extend_from_slice(b"[]"),
            Self::LatestAnchor => body.extend_from_slice(br#"["latest",false]"#),
            Self::NativeBalance {
                account,
                block_hash,
            } => {
                body.extend_from_slice(br#"["0x"#);
                push_hex(&mut body, account.as_slice());
                body.extend_from_slice(br#"",{"blockHash":"0x"#);
                push_hex(&mut body, block_hash.as_slice());
                body.extend_from_slice(br#"","requireCanonical":true}]"#);
            }
            Self::TokenDecimals {
                contract,
                block_hash,
            } => {
                body.extend_from_slice(br#"[{"to":"0x"#);
                push_hex(&mut body, contract.as_slice());
                body.extend_from_slice(br#"","data":"0x313ce567"},{"blockHash":"0x"#);
                push_hex(&mut body, block_hash.as_slice());
                body.extend_from_slice(br#"","requireCanonical":true}]"#);
            }
            Self::TokenBalance {
                contract,
                account,
                block_hash,
            } => {
                body.extend_from_slice(br#"[{"to":"0x"#);
                push_hex(&mut body, contract.as_slice());
                body.extend_from_slice(br#"","data":"0x70a08231"#);
                body.extend_from_slice(b"000000000000000000000000");
                push_hex(&mut body, account.as_slice());
                body.extend_from_slice(br#""},{"blockHash":"0x"#);
                push_hex(&mut body, block_hash.as_slice());
                body.extend_from_slice(br#"","requireCanonical":true}]"#);
            }
            Self::ConfirmAnchor { number } | Self::InclusionBlock { number } => {
                body.extend_from_slice(br#"["#);
                push_quantity(&mut body, number);
                body.extend_from_slice(br#",false]"#);
            }
            Self::PendingNonce { sender } => {
                body.extend_from_slice(br#"["0x"#);
                push_hex(&mut body, sender.as_slice());
                body.extend_from_slice(br#"","pending"]"#);
            }
            Self::SendRawTransaction { signed } => {
                body.extend_from_slice(br#"["0x"#);
                push_hex(&mut body, signed.bytes());
                body.extend_from_slice(br#""]"#);
                drop(signed);
            }
            Self::TransactionByHash { transaction_hash }
            | Self::ReceiptByHash { transaction_hash } => {
                body.extend_from_slice(br#"["0x"#);
                push_hex(&mut body, transaction_hash.as_slice());
                body.extend_from_slice(br#""]"#);
            }
            Self::FinalizedHead => body.extend_from_slice(br#"["finalized",false]"#),
        }
        body.push(b'}');
        if body.len() != capacity {
            return Err(());
        }
        Ok(EncodedRpcRequest { operation, body })
    }

    fn operation(&self) -> ExactRpcOperation {
        match self {
            Self::ChainIdentity => ExactRpcOperation::ChainIdentity,
            Self::LatestAnchor => ExactRpcOperation::LatestAnchor,
            Self::NativeBalance { .. } => ExactRpcOperation::NativeBalance,
            Self::TokenDecimals { .. } => ExactRpcOperation::TokenDecimals,
            Self::TokenBalance { .. } => ExactRpcOperation::TokenBalance,
            Self::ConfirmAnchor { .. } => ExactRpcOperation::ConfirmAnchor,
            Self::PendingNonce { .. } => ExactRpcOperation::PendingNonce,
            Self::SendRawTransaction { .. } => ExactRpcOperation::SendRawTransaction,
            Self::TransactionByHash { .. } => ExactRpcOperation::TransactionByHash,
            Self::ReceiptByHash { .. } => ExactRpcOperation::ReceiptByHash,
            Self::FinalizedHead => ExactRpcOperation::FinalizedHead,
            Self::InclusionBlock { .. } => ExactRpcOperation::InclusionBlock,
        }
    }

    fn params_len(&self) -> Result<usize, ()> {
        Ok(match self {
            Self::ChainIdentity => b"[]".len(),
            Self::LatestAnchor => br#"["latest",false]"#.len(),
            Self::NativeBalance { .. } => {
                br#"["0x"#.len()
                    + 40
                    + br#"",{"blockHash":"0x"#.len()
                    + 64
                    + br#"","requireCanonical":true}]"#.len()
            }
            Self::TokenDecimals { .. } => {
                br#"[{"to":"0x"#.len()
                    + 40
                    + br#"","data":"0x313ce567"},{"blockHash":"0x"#.len()
                    + 64
                    + br#"","requireCanonical":true}]"#.len()
            }
            Self::TokenBalance { .. } => {
                br#"[{"to":"0x"#.len()
                    + 40
                    + br#"","data":"0x70a08231"#.len()
                    + 64
                    + br#""},{"blockHash":"0x"#.len()
                    + 64
                    + br#"","requireCanonical":true}]"#.len()
            }
            Self::ConfirmAnchor { number } | Self::InclusionBlock { number } => {
                12 + quantity_digits_len(*number)
            }
            Self::PendingNonce { .. } => br#"["0x"#.len() + 40 + br#"","pending"]"#.len(),
            Self::SendRawTransaction { signed } => {
                signed_transaction_params_len(signed.bytes().len())?
            }
            Self::TransactionByHash { .. } | Self::ReceiptByHash { .. } => {
                br#"["0x"#.len() + 64 + br#""]"#.len()
            }
            Self::FinalizedHead => br#"["finalized",false]"#.len(),
        })
    }
}

fn signed_transaction_params_len(signed_len: usize) -> Result<usize, ()> {
    if signed_len > EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES {
        return Err(());
    }
    6_usize
        .checked_add(signed_len.checked_mul(2).ok_or(())?)
        .ok_or(())
}

fn push_hex(output: &mut Vec<u8>, bytes: &[u8]) {
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)]);
        output.push(HEX[usize::from(byte & 0x0f)]);
    }
}

fn quantity_digits_len(value: U256) -> usize {
    let bytes = value.to_be_bytes::<32>();
    let Some((index, first)) = bytes.iter().enumerate().find(|(_, byte)| **byte != 0) else {
        return 1;
    };
    (bytes.len() - index) * 2 - usize::from(first >> 4 == 0)
}

fn push_quantity(output: &mut Vec<u8>, value: U256) {
    output.extend_from_slice(b"\"0x");
    let bytes = value.to_be_bytes::<32>();
    let Some((index, first)) = bytes.iter().enumerate().find(|(_, byte)| **byte != 0) else {
        output.push(b'0');
        output.push(b'"');
        return;
    };
    if first >> 4 != 0 {
        output.push(HEX[usize::from(first >> 4)]);
    }
    output.push(HEX[usize::from(first & 0x0f)]);
    push_hex(output, &bytes[index + 1..]);
    output.push(b'"');
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ExactRpcResponse {
    ChainIdentity(u64),
    LatestAnchor(EvmBlockAnchor),
    NativeBalance(U256),
    TokenDecimals(u8),
    TokenBalance(U256),
    ConfirmAnchor(EvmBlockAnchor),
    PendingNonce(U256),
    SendRawTransaction(WalletBroadcastResponse),
    TransactionByHash(Option<EvmWalletObservedTransaction>),
    ReceiptByHash(Option<EvmWalletReceipt>),
    FinalizedHead(EvmBlockAnchor),
    InclusionBlock(Option<EvmBlockAnchor>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalletBroadcastResponse {
    Accepted(B256),
    AlreadyKnown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DecodeFailure {
    MalformedEnvelope,
    MissingResult,
    InvalidResult,
    ResultTooLarge(usize),
    JsonRpcError(i64),
}

pub(super) fn decode_response(
    operation: ExactRpcOperation,
    bytes: &[u8],
) -> Result<ExactRpcResponse, DecodeFailure> {
    let envelope = RawObject::extract(bytes).map_err(|_| DecodeFailure::MalformedEnvelope)?;
    if !envelope.has_only(&["jsonrpc", "id", "result", "error"])
        || envelope.get("jsonrpc").and_then(borrowed_json_string) != Some("2.0")
        || envelope.get("id") != Some(b"1".as_slice())
    {
        return Err(DecodeFailure::MalformedEnvelope);
    }
    let result = match (envelope.get("result"), envelope.get("error")) {
        (Some(result), None) if envelope.len() == 3 => result,
        (None, Some(error)) if envelope.len() == 3 => {
            let error =
                decode_exact::<&RawValue>(error).map_err(|_| DecodeFailure::MalformedEnvelope)?;
            return decode_error(operation, error);
        }
        (None, None) if envelope.len() == 2 => return Err(DecodeFailure::MissingResult),
        _ => return Err(DecodeFailure::MalformedEnvelope),
    };
    if operation.is_read() && result.len() > EVM_READ_MAX_RESPONSE_BYTES {
        return Err(DecodeFailure::ResultTooLarge(result.len()));
    }
    let result = decode_exact::<&RawValue>(result).map_err(|_| DecodeFailure::InvalidResult)?;
    decode_result(operation, result).map_err(|_| DecodeFailure::InvalidResult)
}

fn decode_error(
    operation: ExactRpcOperation,
    raw: &RawValue,
) -> Result<ExactRpcResponse, DecodeFailure> {
    let error =
        RawObject::parse(raw.get().as_bytes()).map_err(|_| DecodeFailure::MalformedEnvelope)?;
    if !error.has_only(&["code", "message", "data"]) {
        return Err(DecodeFailure::MalformedEnvelope);
    }
    let code = error
        .get("code")
        .and_then(parse_i64)
        .ok_or(DecodeFailure::MalformedEnvelope)?;
    let message = error
        .get("message")
        .ok_or(DecodeFailure::MalformedEnvelope)
        .and_then(|value| {
            decode_zeroizing_json_string(
                value,
                JSON_RPC_ERROR_MESSAGE_MAX_BYTES,
                JSON_RPC_ERROR_MESSAGE_MAX_BYTES,
            )
            .map_err(|()| DecodeFailure::MalformedEnvelope)
        })?;
    let data_present = error.get("data").is_some();
    let expected_len = if data_present { 3 } else { 2 };
    if error.len() != expected_len {
        return Err(DecodeFailure::MalformedEnvelope);
    }
    if operation == ExactRpcOperation::SendRawTransaction
        && code == EXACT_ALREADY_KNOWN_CODE
        && message.as_str() == EXACT_ALREADY_KNOWN_MESSAGE
        && !data_present
    {
        return Ok(ExactRpcResponse::SendRawTransaction(
            WalletBroadcastResponse::AlreadyKnown,
        ));
    }
    Err(DecodeFailure::JsonRpcError(code))
}

fn decode_result(operation: ExactRpcOperation, raw: &RawValue) -> Result<ExactRpcResponse, ()> {
    match operation {
        ExactRpcOperation::ChainIdentity => {
            let quantity = parse_quantity(borrowed_json_string(raw.get().as_bytes()).ok_or(())?)?;
            let chain_id = u64::try_from(quantity).map_err(|_| ())?;
            if chain_id == 0 {
                return Err(());
            }
            Ok(ExactRpcResponse::ChainIdentity(chain_id))
        }
        ExactRpcOperation::LatestAnchor => Ok(ExactRpcResponse::LatestAnchor(parse_block(raw)?)),
        ExactRpcOperation::NativeBalance => Ok(ExactRpcResponse::NativeBalance(parse_quantity(
            borrowed_json_string(raw.get().as_bytes()).ok_or(())?,
        )?)),
        ExactRpcOperation::TokenDecimals => Ok(ExactRpcResponse::TokenDecimals(parse_abi_u8(
            borrowed_json_string(raw.get().as_bytes()).ok_or(())?,
        )?)),
        ExactRpcOperation::TokenBalance => Ok(ExactRpcResponse::TokenBalance(parse_abi_u256(
            borrowed_json_string(raw.get().as_bytes()).ok_or(())?,
        )?)),
        ExactRpcOperation::ConfirmAnchor => Ok(ExactRpcResponse::ConfirmAnchor(parse_block(raw)?)),
        ExactRpcOperation::PendingNonce => Ok(ExactRpcResponse::PendingNonce(parse_quantity(
            borrowed_json_string(raw.get().as_bytes()).ok_or(())?,
        )?)),
        ExactRpcOperation::SendRawTransaction => {
            let hash = parse_hash(borrowed_json_string(raw.get().as_bytes()).ok_or(())?)?;
            Ok(ExactRpcResponse::SendRawTransaction(
                WalletBroadcastResponse::Accepted(hash),
            ))
        }
        ExactRpcOperation::TransactionByHash => {
            let transaction = if raw.get() == "null" {
                None
            } else {
                Some(parse_transaction(raw)?)
            };
            Ok(ExactRpcResponse::TransactionByHash(transaction))
        }
        ExactRpcOperation::ReceiptByHash => {
            let receipt = if raw.get() == "null" {
                None
            } else {
                Some(parse_receipt(raw)?)
            };
            Ok(ExactRpcResponse::ReceiptByHash(receipt))
        }
        ExactRpcOperation::FinalizedHead => Ok(ExactRpcResponse::FinalizedHead(parse_block(raw)?)),
        ExactRpcOperation::InclusionBlock => {
            let block = if raw.get() == "null" {
                None
            } else {
                Some(parse_block(raw)?)
            };
            Ok(ExactRpcResponse::InclusionBlock(block))
        }
    }
}

fn decode_exact<'de, T>(bytes: &'de [u8]) -> Result<T, ()>
where
    T: Deserialize<'de>,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(&mut deserializer).map_err(|_| ())?;
    deserializer.end().map_err(|_| ())?;
    Ok(value)
}

struct SensitiveKey(Zeroizing<String>);

impl Borrow<str> for SensitiveKey {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialEq for SensitiveKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl Eq for SensitiveKey {}

impl PartialOrd for SensitiveKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SensitiveKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_str().cmp(other.0.as_str())
    }
}

struct RawObject<'a> {
    source: &'a [u8],
    members: BTreeMap<SensitiveKey, Range<usize>>,
}

impl<'a> RawObject<'a> {
    fn parse(source: &'a [u8]) -> Result<Self, ()> {
        RawParser::new(source).root_object(true)
    }

    fn extract(source: &'a [u8]) -> Result<Self, ()> {
        RawParser::new(source).root_object(false)
    }

    fn get(&self, key: &str) -> Option<&'a [u8]> {
        self.members
            .get(key)
            .map(|range| &self.source[range.clone()])
    }

    fn len(&self) -> usize {
        self.members.len()
    }

    fn has_only(&self, allowed: &[&str]) -> bool {
        self.members
            .keys()
            .all(|key| allowed.contains(&key.0.as_str()))
    }
}

struct RawArray<'a> {
    source: &'a [u8],
    items: Vec<Range<usize>>,
}

impl<'a> RawArray<'a> {
    fn parse(source: &'a [u8]) -> Result<Self, ()> {
        RawParser::new(source).root_array()
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    fn iter(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
        self.items.iter().map(|range| &self.source[range.clone()])
    }
}

struct RawParser<'a> {
    source: &'a [u8],
    cursor: usize,
}

enum JsonScanTask {
    Value,
    ArrayFirst,
    ArrayRest,
    ObjectFirst,
    ObjectRest,
}

impl<'a> RawParser<'a> {
    fn new(source: &'a [u8]) -> Self {
        Self { source, cursor: 0 }
    }

    fn root_object(mut self, validate_values: bool) -> Result<RawObject<'a>, ()> {
        self.skip_whitespace();
        let object = self.parse_object(0, validate_values)?;
        self.skip_whitespace();
        (self.cursor == self.source.len())
            .then_some(object)
            .ok_or(())
    }

    fn root_array(mut self) -> Result<RawArray<'a>, ()> {
        self.skip_whitespace();
        let array = self.parse_array(0)?;
        self.skip_whitespace();
        (self.cursor == self.source.len())
            .then_some(array)
            .ok_or(())
    }

    fn parse_value(&mut self, depth: usize) -> Result<Range<usize>, ()> {
        if depth > PROJECTION_MAX_DEPTH {
            return Err(());
        }
        self.skip_whitespace();
        let start = self.cursor;
        match self.peek().ok_or(())? {
            b'{' => {
                self.parse_object(depth, true)?;
            }
            b'[' => {
                self.parse_array(depth)?;
            }
            b'"' => {
                self.scan_string()?;
            }
            _ => self.scan_scalar()?,
        }
        Ok(start..self.cursor)
    }

    fn skip_value_unbounded(&mut self) -> Result<Range<usize>, ()> {
        self.skip_whitespace();
        let start = self.cursor;
        let mut tasks = vec![JsonScanTask::Value];
        while let Some(task) = tasks.pop() {
            match task {
                JsonScanTask::Value => {
                    self.skip_whitespace();
                    match self.peek().ok_or(())? {
                        b'{' => {
                            self.cursor += 1;
                            tasks.push(JsonScanTask::ObjectFirst);
                        }
                        b'[' => {
                            self.cursor += 1;
                            tasks.push(JsonScanTask::ArrayFirst);
                        }
                        b'"' => {
                            self.scan_string()?;
                        }
                        b't' => self.scan_literal(b"true")?,
                        b'f' => self.scan_literal(b"false")?,
                        b'n' => self.scan_literal(b"null")?,
                        b'-' | b'0'..=b'9' => self.scan_number()?,
                        _ => return Err(()),
                    }
                }
                JsonScanTask::ArrayFirst => {
                    self.skip_whitespace();
                    if !self.consume(b']') {
                        tasks.push(JsonScanTask::ArrayRest);
                        tasks.push(JsonScanTask::Value);
                    }
                }
                JsonScanTask::ArrayRest => {
                    self.skip_whitespace();
                    if !self.consume(b']') {
                        self.require(b',')?;
                        tasks.push(JsonScanTask::ArrayRest);
                        tasks.push(JsonScanTask::Value);
                    }
                }
                JsonScanTask::ObjectFirst => {
                    self.skip_whitespace();
                    if !self.consume(b'}') {
                        self.scan_string()?;
                        self.skip_whitespace();
                        self.require(b':')?;
                        tasks.push(JsonScanTask::ObjectRest);
                        tasks.push(JsonScanTask::Value);
                    }
                }
                JsonScanTask::ObjectRest => {
                    self.skip_whitespace();
                    if !self.consume(b'}') {
                        self.require(b',')?;
                        self.skip_whitespace();
                        self.scan_string()?;
                        self.skip_whitespace();
                        self.require(b':')?;
                        tasks.push(JsonScanTask::ObjectRest);
                        tasks.push(JsonScanTask::Value);
                    }
                }
            }
        }
        Ok(start..self.cursor)
    }

    fn parse_object(&mut self, depth: usize, validate_values: bool) -> Result<RawObject<'a>, ()> {
        if validate_values && depth > PROJECTION_MAX_DEPTH {
            return Err(());
        }
        self.require(b'{')?;
        let mut members = BTreeMap::new();
        self.skip_whitespace();
        if self.consume(b'}') {
            return Ok(RawObject {
                source: self.source,
                members,
            });
        }
        loop {
            if members.len() >= PROJECTION_MAX_CONTAINER_ITEMS {
                return Err(());
            }
            let key_range = self.scan_string()?;
            let key = SensitiveKey(decode_zeroizing_json_string(
                &self.source[key_range],
                JSON_OBJECT_KEY_MAX_BYTES,
                JSON_OBJECT_KEY_MAX_BYTES,
            )?);
            self.skip_whitespace();
            self.require(b':')?;
            let value = if validate_values {
                self.parse_value(depth + 1)?
            } else {
                self.skip_value_unbounded()?
            };
            if members.insert(key, value).is_some() {
                return Err(());
            }
            self.skip_whitespace();
            if self.consume(b'}') {
                break;
            }
            self.require(b',')?;
            self.skip_whitespace();
        }
        Ok(RawObject {
            source: self.source,
            members,
        })
    }

    fn parse_array(&mut self, depth: usize) -> Result<RawArray<'a>, ()> {
        if depth > PROJECTION_MAX_DEPTH {
            return Err(());
        }
        self.require(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.consume(b']') {
            return Ok(RawArray {
                source: self.source,
                items,
            });
        }
        loop {
            if items.len() >= PROJECTION_MAX_CONTAINER_ITEMS {
                return Err(());
            }
            items.push(self.parse_value(depth + 1)?);
            self.skip_whitespace();
            if self.consume(b']') {
                break;
            }
            self.require(b',')?;
        }
        Ok(RawArray {
            source: self.source,
            items,
        })
    }

    fn scan_string(&mut self) -> Result<Range<usize>, ()> {
        let start = self.cursor;
        self.require(b'"')?;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.cursor += 1;
                    std::str::from_utf8(&self.source[start..self.cursor]).map_err(|_| ())?;
                    return Ok(start..self.cursor);
                }
                b'\\' => {
                    self.cursor += 1;
                    let escape = self.peek().ok_or(())?;
                    self.cursor += 1;
                    match escape {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                        b'u' => {
                            let end = self.cursor.checked_add(4).ok_or(())?;
                            let digits = self.source.get(self.cursor..end).ok_or(())?;
                            parse_hex_u16(digits)?;
                            self.cursor = end;
                        }
                        _ => return Err(()),
                    }
                }
                byte if byte < 0x20 => return Err(()),
                _ => self.cursor += 1,
            }
        }
        Err(())
    }

    fn scan_literal(&mut self, literal: &[u8]) -> Result<(), ()> {
        let end = self.cursor.checked_add(literal.len()).ok_or(())?;
        if self.source.get(self.cursor..end) != Some(literal) {
            return Err(());
        }
        self.cursor = end;
        Ok(())
    }

    fn scan_number(&mut self) -> Result<(), ()> {
        self.consume(b'-');
        match self.peek().ok_or(())? {
            b'0' => {
                self.cursor += 1;
                if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(());
                }
            }
            b'1'..=b'9' => {
                self.cursor += 1;
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.cursor += 1;
                }
            }
            _ => return Err(()),
        }
        if self.consume(b'.') {
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(());
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor += 1;
            }
        }
        if self.peek().is_some_and(|byte| matches!(byte, b'e' | b'E')) {
            self.cursor += 1;
            if self.peek().is_some_and(|byte| matches!(byte, b'+' | b'-')) {
                self.cursor += 1;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(());
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor += 1;
            }
        }
        Ok(())
    }

    fn scan_scalar(&mut self) -> Result<(), ()> {
        match self.peek().ok_or(())? {
            b't' => self.scan_literal(b"true"),
            b'f' => self.scan_literal(b"false"),
            b'n' => self.scan_literal(b"null"),
            b'-' | b'0'..=b'9' => self.scan_number(),
            _ => Err(()),
        }
    }

    fn skip_whitespace(&mut self) {
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.cursor += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.cursor).copied()
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn require(&mut self, expected: u8) -> Result<(), ()> {
        self.consume(expected).then_some(()).ok_or(())
    }
}

fn borrowed_json_string(raw: &[u8]) -> Option<&str> {
    if raw.len() < 2
        || raw.first() != Some(&b'"')
        || raw.last() != Some(&b'"')
        || raw[1..raw.len() - 1]
            .iter()
            .any(|byte| *byte == b'\\' || *byte < 0x20)
    {
        return None;
    }
    std::str::from_utf8(&raw[1..raw.len() - 1]).ok()
}

fn decode_zeroizing_json_string(
    raw: &[u8],
    encoded_maximum: usize,
    decoded_maximum: usize,
) -> Result<Zeroizing<String>, ()> {
    if raw.len() < 2
        || raw.first() != Some(&b'"')
        || raw.last() != Some(&b'"')
        || raw.len() - 2 > encoded_maximum
    {
        return Err(());
    }
    let mut decoded = Zeroizing::new(String::with_capacity(
        raw.len().saturating_sub(2).min(decoded_maximum),
    ));
    let mut cursor = 1_usize;
    let end = raw.len() - 1;
    let mut segment_start = cursor;
    while cursor < end {
        match raw[cursor] {
            b'\\' => {
                push_string_segment(&mut decoded, &raw[segment_start..cursor], decoded_maximum)?;
                cursor += 1;
                let escape = *raw.get(cursor).ok_or(())?;
                cursor += 1;
                match escape {
                    b'"' => decoded.push('"'),
                    b'\\' => decoded.push('\\'),
                    b'/' => decoded.push('/'),
                    b'b' => decoded.push('\u{0008}'),
                    b'f' => decoded.push('\u{000c}'),
                    b'n' => decoded.push('\n'),
                    b'r' => decoded.push('\r'),
                    b't' => decoded.push('\t'),
                    b'u' => {
                        let (character, next) = decode_unicode_escape(raw, cursor, end)?;
                        decoded.push(character);
                        cursor = next;
                    }
                    _ => return Err(()),
                }
                if decoded.len() > decoded_maximum {
                    return Err(());
                }
                segment_start = cursor;
            }
            byte if byte < 0x20 => return Err(()),
            _ => cursor += 1,
        }
    }
    push_string_segment(&mut decoded, &raw[segment_start..end], decoded_maximum)?;
    Ok(decoded)
}

fn push_string_segment(output: &mut String, bytes: &[u8], maximum: usize) -> Result<(), ()> {
    let value = std::str::from_utf8(bytes).map_err(|_| ())?;
    if output.len().checked_add(value.len()).ok_or(())? > maximum {
        return Err(());
    }
    output.push_str(value);
    Ok(())
}

fn decode_unicode_escape(raw: &[u8], cursor: usize, end: usize) -> Result<(char, usize), ()> {
    let first_end = cursor.checked_add(4).ok_or(())?;
    if first_end > end {
        return Err(());
    }
    let first = parse_hex_u16(&raw[cursor..first_end])?;
    if (0xd800..=0xdbff).contains(&first) {
        let second_start = first_end;
        let second_digits = second_start.checked_add(2).ok_or(())?;
        let second_end = second_digits.checked_add(4).ok_or(())?;
        if second_end > end || &raw[second_start..second_digits] != b"\\u" {
            return Err(());
        }
        let second = parse_hex_u16(&raw[second_digits..second_end])?;
        if !(0xdc00..=0xdfff).contains(&second) {
            return Err(());
        }
        let scalar = 0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00);
        char::from_u32(scalar)
            .map(|value| (value, second_end))
            .ok_or(())
    } else if (0xdc00..=0xdfff).contains(&first) {
        Err(())
    } else {
        char::from_u32(u32::from(first))
            .map(|value| (value, first_end))
            .ok_or(())
    }
}

fn parse_hex_u16(bytes: &[u8]) -> Result<u16, ()> {
    bytes.iter().try_fold(0_u16, |value, byte| {
        let digit = match *byte {
            b'0'..=b'9' => u16::from(*byte - b'0'),
            b'a'..=b'f' => u16::from(*byte - b'a') + 10,
            b'A'..=b'F' => u16::from(*byte - b'A') + 10,
            _ => return Err(()),
        };
        value
            .checked_mul(16)
            .and_then(|value| value.checked_add(digit))
            .ok_or(())
    })
}

fn parse_i64(raw: &[u8]) -> Option<i64> {
    let value = std::str::from_utf8(raw).ok()?;
    if value.is_empty()
        || value.starts_with('+')
        || (value.starts_with('0') && value.len() > 1)
        || (value.starts_with("-0") && value.len() > 2)
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && byte != b'-')
    {
        return None;
    }
    value.parse().ok()
}

fn parse_block(raw: &RawValue) -> Result<EvmBlockAnchor, ()> {
    let block = RawObject::parse(raw.get().as_bytes())?;
    let block = EvmBlockAnchor::new(
        parse_quantity(required_string(&block, "number")?)?,
        parse_hash(required_string(&block, "hash")?)?,
    );
    block.validate().map_err(|_| ())?;
    Ok(block)
}

#[derive(Clone, Copy)]
enum NullableString<'a> {
    Null,
    String(&'a str),
}

fn parse_transaction(raw: &RawValue) -> Result<EvmWalletObservedTransaction, ()> {
    let transaction = RawObject::parse(raw.get().as_bytes())?;
    if parse_quantity(required_string(&transaction, "type")?)?
        != U256::from(EVM_WALLET_TRANSACTION_TYPE)
    {
        return Err(());
    }
    let to = match required_nullable_string(&transaction, "to")? {
        NullableString::Null => TxKind::Create,
        NullableString::String(value) => TxKind::Call(parse_address(value)?),
    };
    let placement = parse_optional_placement(
        required_nullable_string(&transaction, "blockNumber")?,
        required_nullable_string(&transaction, "blockHash")?,
        required_nullable_string(&transaction, "transactionIndex")?,
    )?;
    EvmWalletObservedTransaction::new(
        parse_hash(required_string(&transaction, "hash")?)?,
        parse_quantity(required_string(&transaction, "chainId")?)?,
        parse_quantity(required_string(&transaction, "nonce")?)?,
        parse_address(required_string(&transaction, "from")?)?,
        to,
        parse_quantity(required_string(&transaction, "value")?)?,
        parse_bounded_hex(
            required_string(&transaction, "input")?,
            EVM_WALLET_DATA_MAX_BYTES,
        )?,
        parse_quantity(required_string(&transaction, "gas")?)?,
        parse_quantity(required_string(&transaction, "maxFeePerGas")?)?,
        parse_quantity(required_string(&transaction, "maxPriorityFeePerGas")?)?,
        parse_access_list(transaction.get("accessList").ok_or(())?)?,
        placement,
    )
    .map_err(|_| ())
}

fn parse_optional_placement(
    number: NullableString<'_>,
    hash: NullableString<'_>,
    index: NullableString<'_>,
) -> Result<Option<EvmWalletTransactionPlacement>, ()> {
    match (number, hash, index) {
        (NullableString::Null, NullableString::Null, NullableString::Null) => Ok(None),
        (
            NullableString::String(number),
            NullableString::String(hash),
            NullableString::String(index),
        ) => EvmWalletTransactionPlacement::new(
            EvmBlockAnchor::new(parse_quantity(number)?, parse_hash(hash)?),
            parse_quantity(index)?,
        )
        .map(Some)
        .map_err(|_| ()),
        _ => Err(()),
    }
}

fn parse_access_list(raw: &[u8]) -> Result<Vec<EvmWalletAccessListEntry>, ()> {
    let values = RawArray::parse(raw)?;
    if values.len() > EVM_WALLET_ACCESS_LIST_MAX_ENTRIES {
        return Err(());
    }
    let mut storage_key_count = 0_usize;
    values
        .iter()
        .map(|entry| {
            let entry = RawObject::parse(entry)?;
            let storage_keys = RawArray::parse(entry.get("storageKeys").ok_or(())?)?;
            storage_key_count = storage_key_count
                .checked_add(storage_keys.len())
                .ok_or(())?;
            if storage_key_count > EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS {
                return Err(());
            }
            let keys = storage_keys
                .iter()
                .map(|value| parse_hash(borrowed_json_string(value).ok_or(())?))
                .collect::<Result<Vec<_>, _>>()?;
            EvmWalletAccessListEntry::new(parse_address(required_string(&entry, "address")?)?, keys)
                .map_err(|_| ())
        })
        .collect()
}

fn parse_receipt(raw: &RawValue) -> Result<EvmWalletReceipt, ()> {
    let receipt = RawObject::parse(raw.get().as_bytes())?;
    if let Some(transaction_type) = receipt.get("type") {
        if parse_quantity(borrowed_json_string(transaction_type).ok_or(())?)?
            != U256::from(EVM_WALLET_TRANSACTION_TYPE)
        {
            return Err(());
        }
    }
    let log_values = RawArray::parse(receipt.get("logs").ok_or(())?)?;
    if log_values.len() > EVM_WALLET_RECEIPT_LOG_LIMIT {
        return Err(());
    }
    let block = EvmBlockAnchor::new(
        parse_quantity(required_string(&receipt, "blockNumber")?)?,
        parse_hash(required_string(&receipt, "blockHash")?)?,
    );
    let status = match parse_quantity(required_string(&receipt, "status")?)? {
        value if value == U256::from(1) => EvmWalletReceiptStatus::Success,
        value if value == U256::ZERO => EvmWalletReceiptStatus::Reverted,
        _ => return Err(()),
    };
    let transaction_hash = parse_hash(required_string(&receipt, "transactionHash")?)?;
    let transaction_index = parse_quantity(required_string(&receipt, "transactionIndex")?)?;
    let logs = log_values
        .iter()
        .map(|value| parse_receipt_log(value, &block, transaction_hash, transaction_index))
        .collect::<Result<Vec<_>, _>>()?;
    EvmWalletReceipt::new(
        transaction_hash,
        transaction_index,
        block,
        parse_address(required_string(&receipt, "from")?)?,
        parse_nullable_address(required_nullable_string(&receipt, "to")?)?,
        parse_nullable_address(required_nullable_string(&receipt, "contractAddress")?)?,
        status,
        parse_quantity(required_string(&receipt, "gasUsed")?)?,
        parse_quantity(required_string(&receipt, "cumulativeGasUsed")?)?,
        logs,
    )
    .map_err(|_| ())
}

fn parse_receipt_log(
    value: &[u8],
    expected_block: &EvmBlockAnchor,
    expected_transaction_hash: B256,
    expected_transaction_index: U256,
) -> Result<EvmWalletReceiptLog, ()> {
    let value = RawObject::parse(value)?;
    let block = EvmBlockAnchor::new(
        parse_quantity(required_string(&value, "blockNumber")?)?,
        parse_hash(required_string(&value, "blockHash")?)?,
    );
    let transaction_hash = parse_hash(required_string(&value, "transactionHash")?)?;
    let transaction_index = parse_quantity(required_string(&value, "transactionIndex")?)?;
    if &block != expected_block
        || transaction_hash != expected_transaction_hash
        || transaction_index != expected_transaction_index
    {
        return Err(());
    }
    let topics = RawArray::parse(value.get("topics").ok_or(())?)?
        .iter()
        .map(|value| parse_hash(borrowed_json_string(value).ok_or(())?))
        .collect::<Result<Vec<_>, _>>()?;
    EvmWalletReceiptLog::new(
        parse_address(required_string(&value, "address")?)?,
        topics,
        parse_bounded_hex(
            required_string(&value, "data")?,
            EVM_WALLET_RECEIPT_LOG_DATA_MAX_BYTES,
        )?,
        block,
        transaction_hash,
        transaction_index,
        parse_quantity(required_string(&value, "logIndex")?)?,
        parse_bool(value.get("removed").ok_or(())?)?,
    )
    .map_err(|_| ())
}

fn parse_nullable_address(value: NullableString<'_>) -> Result<Option<Address>, ()> {
    match value {
        NullableString::Null => Ok(None),
        NullableString::String(value) => parse_address(value).map(Some),
    }
}

fn required_string<'a>(object: &RawObject<'a>, key: &str) -> Result<&'a str, ()> {
    object.get(key).and_then(borrowed_json_string).ok_or(())
}

fn required_nullable_string<'a>(
    object: &RawObject<'a>,
    key: &str,
) -> Result<NullableString<'a>, ()> {
    let value = object.get(key).ok_or(())?;
    if value == b"null" {
        Ok(NullableString::Null)
    } else {
        borrowed_json_string(value)
            .map(NullableString::String)
            .ok_or(())
    }
}

fn parse_bool(raw: &[u8]) -> Result<bool, ()> {
    match raw {
        b"true" => Ok(true),
        b"false" => Ok(false),
        _ => Err(()),
    }
}

fn parse_quantity(raw: &str) -> Result<U256, ()> {
    let digits = raw.strip_prefix("0x").ok_or(())?;
    if digits.is_empty()
        || digits.len() > 64
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(());
    }
    U256::from_str_radix(digits, 16).map_err(|_| ())
}

fn parse_hash(raw: &str) -> Result<B256, ()> {
    if raw.len() != 66 || !is_lower_hex(raw, 64) {
        return Err(());
    }
    B256::from_str(raw).map_err(|_| ())
}

fn parse_address(raw: &str) -> Result<Address, ()> {
    if raw.len() != 42 || !is_lower_hex(raw, 40) {
        return Err(());
    }
    Address::from_str(raw).map_err(|_| ())
}

fn is_lower_hex(raw: &str, digits: usize) -> bool {
    raw.len() == digits + 2
        && raw.starts_with("0x")
        && raw[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn parse_abi_u8(raw: &str) -> Result<u8, ()> {
    let bytes = parse_bounded_hex(raw, 32)?;
    if bytes.len() != 32 || bytes[..31].iter().any(|byte| *byte != 0) {
        return Err(());
    }
    Ok(bytes[31])
}

fn parse_abi_u256(raw: &str) -> Result<U256, ()> {
    let bytes = parse_bounded_hex(raw, 32)?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| ())?;
    Ok(U256::from_be_bytes(bytes))
}

fn parse_bounded_hex(raw: &str, maximum: usize) -> Result<Vec<u8>, ()> {
    let digits = raw.strip_prefix("0x").ok_or(())?;
    if !digits.len().is_multiple_of(2)
        || digits.len() / 2 > maximum
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(());
    }
    hex::decode(digits).map_err(|_| ())
}

#[cfg(test)]
#[path = "exact_tests.rs"]
mod tests;
