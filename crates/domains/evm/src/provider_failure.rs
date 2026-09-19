//! Provider-owned operation and validation facts; client extraction stays in live adapters.

use super::*;
use mfm_program_derive::PersistedSchema;
use mfm_values::DiagnosticEvidence;

/// Whether an observed size is exact or only a lower bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "observed-size",
    version = "1",
    schema = "mfm.diagnostics.observed-size"
)]
pub enum ObservedSize {
    /// Exact measured size.
    Exact {
        /// Measured bytes or items.
        value: u64,
    },
    /// Reading stopped after this many bytes or items.
    AtLeast {
        /// Known lower bound.
        value: u64,
    },
}

/// Reviewed RPC methods used by first-party EVM and development funding adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "rpc-method",
    version = "2",
    schema = "mfm.evm-rpc-method"
)]
pub enum EvmRpcMethod {
    /// Chain identity.
    ChainId,
    /// Number-selected block.
    GetBlockByNumber,
    /// Native balance.
    GetBalance,
    /// Anchored contract call.
    Call,
    /// Contract bytecode.
    GetCode,
    /// Pending account nonce.
    GetTransactionCount,
    /// Transaction receipt.
    GetTransactionReceipt,
    /// Exact transaction presence, pending or mined.
    GetTransactionByHash,
    /// Exact signed transaction submission.
    SendRawTransaction,
    /// Development node accounts.
    Accounts,
    /// Development node funding submission.
    SendTransaction,
}
impl EvmRpcMethod {
    /// Returns the protocol method spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChainId => "eth_chainId",
            Self::GetBlockByNumber => "eth_getBlockByNumber",
            Self::GetBalance => "eth_getBalance",
            Self::Call => "eth_call",
            Self::GetCode => "eth_getCode",
            Self::GetTransactionCount => "eth_getTransactionCount",
            Self::GetTransactionReceipt => "eth_getTransactionReceipt",
            Self::GetTransactionByHash => "eth_getTransactionByHash",
            Self::SendRawTransaction => "eth_sendRawTransaction",
            Self::Accounts => "eth_accounts",
            Self::SendTransaction => "eth_sendTransaction",
        }
    }
}

/// Originating provider request stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "rpc-stage",
    version = "1",
    schema = "mfm.evm-rpc-stage"
)]
pub enum RpcStage {
    /// Request construction and send.
    Send,
    /// Received HTTP status.
    Status,
    /// Bounded response body read.
    Body,
    /// JSON-RPC envelope admission.
    Envelope,
    /// Typed result decoding and validation.
    Result,
    /// Transaction adapter checks of provider observations.
    Validation,
}

/// Reviewed result field whose admission failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "rpc-field",
    version = "1",
    schema = "mfm.evm-rpc-field"
)]
pub enum RpcField {
    /// Development funding source account.
    FundingAccount,
    /// Complete response result.
    Result,
    /// Chain identity.
    ChainId,
    /// Account nonce.
    Nonce,
    /// Expected block.
    Block,
    /// Genesis block number.
    GenesisNumber,
    /// Token decimals.
    Decimals,
    /// DATA bytes.
    Data,
    /// One ABI word.
    AbiWord,
    /// Receipt outcome fields.
    ReceiptOutcome,
}

/// Checked local result rejection, with no arbitrary response text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "rpc-rejection",
    version = "1",
    schema = "mfm.evm-rpc-rejection"
)]
pub enum RpcRejection {
    /// A received result required a value rather than absence.
    #[error("RPC result rejected")]
    Missing,
    /// The received encoding violates the field grammar.
    #[error("RPC result rejected")]
    Encoding,
    /// A numeric value did not fit the checked field.
    #[error("RPC result rejected")]
    Range {
        /// Inclusive minimum.
        minimum: u64,
        /// Inclusive maximum.
        maximum: u64,
        /// Received numeric value.
        observed: EvmU256,
    },
    /// A checked EVM value rejected the parsed data.
    #[error("RPC result rejected")]
    Domain {
        /// Original domain construction error.
        #[source]
        cause: EvmDomainError,
    },
    /// Checked receipt fields form no supported outcome.
    #[error("RPC result rejected")]
    ReceiptOutcome {
        /// Received status quantity.
        status: EvmU256,
        /// Received recipient.
        to: Option<EvmAddress>,
        /// Received created-contract address.
        contract_address: Option<EvmAddress>,
    },
    /// A declared or observed size exceeded the bound.
    #[error("RPC result rejected")]
    Size {
        /// Inclusive permitted size.
        limit: u64,
        /// Exact or lower-bound observation. At Body stage, an exact declared
        /// Content-Length is not a claim that those bytes were physically read.
        #[mfm(persisted)]
        observed: ObservedSize,
    },
    /// Received transaction identity disagreed with the submitted identity.
    #[error("RPC result rejected")]
    HashMismatch {
        /// Expected checked hash.
        expected: EvmHash,
        /// Observed checked hash.
        observed: EvmHash,
    },
    /// Canonical block differed from the receipt's block.
    #[error("RPC result rejected")]
    AnchorMismatch {
        /// Receipt anchor.
        expected: EvmBlockAnchor,
        /// Provider's canonical anchor.
        observed: EvmBlockAnchor,
    },
}

/// Origin of the provider failure; upstream layers remain in diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "provider-failure-kind",
    version = "1",
    schema = "mfm.evm-provider-failure-kind"
)]
pub enum ProviderFailureKind {
    /// Concrete client or parser error.
    #[error("provider request failed")]
    Client,
    /// Non-success HTTP status.
    #[error("provider request failed")]
    HttpStatus,
    /// Checked JSON-RPC error envelope.
    #[error("provider request failed")]
    RpcError,
    /// Checked local field validation.
    #[error("provider request failed")]
    Rejected {
        /// Field under validation.
        field: RpcField,
        /// Exact reviewed rejection.
        #[source]
        cause: RpcRejection,
    },
}

/// Reviewed provider failure retaining its operation, stage and causal evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[error("evm provider failure")]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "provider-failure",
    version = "2",
    schema = "mfm.evm-provider-failure"
)]
pub struct ProviderFailure {
    /// Originating RPC method.
    pub method: EvmRpcMethod,
    /// Originating stage.
    pub stage: RpcStage,
    /// Local checked facts or upstream failure category.
    #[source]
    pub failure: ProviderFailureKind,
    /// Selected response and exposed native source data.
    #[mfm(persisted)]
    pub diagnostics: DiagnosticEvidence,
}

/// Provider-owned operational category; classification remains error-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "operational-kind",
    version = "1",
    schema = "mfm.evm-operational-kind"
)]
pub enum EvmOperationalKind {
    /// The provider did not produce a usable response.
    Unavailable,
    /// The bounded provider deadline expired.
    Timeout,
    /// The provider explicitly limited request traffic.
    RateLimited,
}

/// Reviewed operational category paired with its complete provider-owned cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[error("evm provider operation failed")]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "operational-error",
    version = "3",
    schema = "mfm.evm-operational-error"
)]
pub struct EvmOperationalError {
    kind: EvmOperationalKind,
    // Keep the callback's success path small; boxing is transparent on the wire.
    source: Box<ProviderFailure>,
}
impl EvmOperationalError {
    /// Retains the explicit owner category and reviewed provider source together.
    pub fn new(kind: EvmOperationalKind, source: ProviderFailure) -> Self {
        Self {
            kind,
            source: Box::new(source),
        }
    }
    /// Returns the provider-owned category without reclassifying evidence.
    pub fn kind(&self) -> EvmOperationalKind {
        self.kind
    }
    /// Borrows the retained cause independently of its classification.
    pub fn provider_failure(&self) -> &ProviderFailure {
        &self.source
    }
}

/// Provider operation within transaction execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-provider-operation",
    version = "2",
    schema = "mfm.evm-transaction-provider-operation"
)]
pub enum TransactionProviderOperation {
    /// Verify chain identity before proceeding.
    VerifyChain,
    /// Observe nonce before reservation.
    ObserveNonce,
    /// Observe a submitted transaction's receipt.
    Receipt,
    /// Reconcile presence before rebroadcasting retained bytes.
    TransactionKnown,
    /// Submit retained signed bytes.
    Submit,
    /// Confirm the receipt's canonical block.
    CanonicalBlock,
}
