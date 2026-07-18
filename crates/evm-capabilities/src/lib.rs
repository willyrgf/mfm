#![warn(missing_docs)]
//! Source-bound EVM capability contracts.
//!
//! The capability surface has two coherent views: checked external reads and
//! one-transaction authority. A live implementation binds one semantic
//! network to one source before exposing either view. Endpoints and
//! credentials never enter these contracts.
//!
//! ```rust
//! use mfm_capabilities::CapabilitySpec;
//! use mfm_evm_capabilities::{EvmNetworkBinding, EvmReadCapability};
//! use mfm_ids::LocalPublicId;
//!
//! let binding = EvmNetworkBinding::new(LocalPublicId::new("ethereum-mainnet")?, 1)?;
//! assert_eq!(binding.network_id().as_str(), "ethereum-mainnet");
//! assert_eq!(EvmReadCapability::name(), "mfm.evm.read");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::future::Future;
use std::num::NonZeroU64;
use std::pin::Pin;

use alloy_eips::eip2930::AccessList;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use mfm_canonical::sha256_digest_bytes;
pub use mfm_capabilities::ProviderDiagnosticCode;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ExternalMutationAuthorityRole, ProviderDiagnosticValue,
    ReadExternalRole, RedactedProviderDiagnostic,
};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm, LocalPublicId};
use mfm_program_derive::MfmValue;
use serde::{de, Deserialize, Serialize};

/// Result type for EVM capability contracts.
pub type Result<T> = std::result::Result<T, EvmCapabilityError>;

/// Boxed future returned by an EVM session method.
pub type EvmSessionFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Stable implementation id for the bounded JSON-RPC session.
pub const EVM_JSONRPC_SESSION_IMPLEMENTATION_ID: &str = "mfm.evm.jsonrpc.session.v1";
/// EIP-2718 transaction type used by the one admitted EVM transaction flow.
pub const EVM_EIP1559_TRANSACTION_TYPE: u8 = 2;

macro_rules! evm_capability {
    ($(#[$meta:meta])* $ty:ident, $role:ty, $name:literal) => {
        $(#[$meta])*
        pub struct $ty;

        impl CapabilitySpec for $ty {
            type Role = $role;

            fn kind() -> mfm_capabilities::Result<CapabilityKind> {
                CapabilityKind::new(
                    "mfm.evm",
                    $name,
                    DigestAlgorithm::Sha256JcsV1,
                    sha256_digest_bytes(concat!("mfm.evm.capability:", $name).as_bytes()),
                )
                .map_err(|error| CapabilityError::Identity(error.to_string()))
            }

            fn version() -> mfm_capabilities::Result<CapabilityVersion> {
                CapabilityVersion::new(concat!("mfm.evm.", $name, ".v1"))
                    .map_err(|error| CapabilityError::Identity(error.to_string()))
            }

            fn name() -> &'static str {
                concat!("mfm.evm.", $name)
            }
        }
    };
}

evm_capability!(
    /// Source-bound block, balance, code, and call read authority.
    EvmReadCapability,
    ReadExternalRole,
    "read"
);
evm_capability!(
    /// Source-bound single-transaction preparation, submission, and observation authority.
    EvmTransactionCapability,
    ExternalMutationAuthorityRole,
    "transaction"
);

/// Checked semantic EVM network and chain binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmNetworkBinding {
    network_id: LocalPublicId,
    expected_chain_id: NonZeroU64,
}

impl EvmNetworkBinding {
    /// Creates a checked binding. Chain id zero is rejected.
    pub fn new(network_id: LocalPublicId, expected_chain_id: u64) -> Result<Self> {
        let expected_chain_id =
            NonZeroU64::new(expected_chain_id).ok_or(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::ZeroExpectedChainId,
            })?;
        Ok(Self {
            network_id,
            expected_chain_id,
        })
    }

    /// Returns the semantic network id.
    pub const fn network_id(&self) -> &LocalPublicId {
        &self.network_id
    }

    /// Returns the expected chain id.
    pub const fn expected_chain_id(&self) -> u64 {
        self.expected_chain_id.get()
    }
}

/// Redacted provenance for one checked, source-stable session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "session_evidence",
    version = "1",
    schema = "mfm.evm.session_evidence"
)]
pub struct EvmSessionEvidence {
    network_id: String,
    chain_id: u64,
    source_ref: String,
    implementation_id: String,
}

impl EvmSessionEvidence {
    /// Creates evidence after bind-time chain verification succeeds.
    pub fn new(
        binding: &EvmNetworkBinding,
        source_ref: LocalPublicId,
        implementation_id: LocalPublicId,
    ) -> Self {
        Self {
            network_id: binding.network_id.as_str().to_owned(),
            chain_id: binding.expected_chain_id.get(),
            source_ref: source_ref.as_str().to_owned(),
            implementation_id: implementation_id.as_str().to_owned(),
        }
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the verified chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the process-local source reference retained as audit provenance.
    ///
    /// The reference identifies the route used by this attempt. It is not
    /// semantic binding policy and may change when a later attempt is resumed
    /// under different process-local routing.
    pub fn source_ref(&self) -> &str {
        &self.source_ref
    }

    /// Returns the certified transport implementation identity.
    pub fn implementation_id(&self) -> &str {
        &self.implementation_id
    }

    /// Returns whether this evidence belongs to the binding.
    pub fn matches_binding(&self, binding: &EvmNetworkBinding) -> bool {
        self.network_id == binding.network_id.as_str()
            && self.chain_id == binding.expected_chain_id.get()
    }
}

impl<'de> Deserialize<'de> for EvmSessionEvidence {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            network_id: String,
            chain_id: u64,
            source_ref: String,
            implementation_id: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let network_id = LocalPublicId::new(&wire.network_id).map_err(de::Error::custom)?;
        let binding =
            EvmNetworkBinding::new(network_id, wire.chain_id).map_err(de::Error::custom)?;
        let source_ref = LocalPublicId::new(&wire.source_ref).map_err(de::Error::custom)?;
        let implementation_id =
            LocalPublicId::new(&wire.implementation_id).map_err(de::Error::custom)?;
        Ok(Self::new(&binding, source_ref, implementation_id))
    }
}

/// EVM block selector used by checked sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvmBlockSelector {
    /// Current head.
    Latest,
    /// Exact block number.
    Number(U256),
    /// Exact canonical block hash. Transports issue EIP-1898 `requireCanonical` reads.
    ExactHash(B256),
}

/// Minimal block identity returned by a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmBlock {
    /// Block number.
    pub number: U256,
    /// Block hash.
    pub hash: B256,
}

/// Checked call request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmCall {
    from: Address,
    to: Address,
    value: U256,
    input: Bytes,
    gas_limit: U256,
    access_list: AccessList,
    block: EvmBlockSelector,
}

impl EvmCall {
    /// Creates a fully specified call request. A zero gas limit is rejected.
    pub fn new(
        from: Address,
        to: Address,
        value: U256,
        input: Bytes,
        gas_limit: U256,
        access_list: AccessList,
        block: EvmBlockSelector,
    ) -> Result<Self> {
        if gas_limit.is_zero() {
            return Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::ZeroCallGasLimit,
            });
        }
        Ok(Self {
            from,
            to,
            value,
            input,
            gas_limit,
            access_list,
            block,
        })
    }

    /// Returns the explicit caller.
    pub const fn from(&self) -> Address {
        self.from
    }

    /// Returns the destination.
    pub const fn to(&self) -> Address {
        self.to
    }

    /// Returns the input bytes.
    pub const fn input(&self) -> &Bytes {
        &self.input
    }

    /// Returns the transferred wei value.
    pub const fn value(&self) -> U256 {
        self.value
    }

    /// Returns the call gas bound.
    pub const fn gas_limit(&self) -> U256 {
        self.gas_limit
    }

    /// Returns the EIP-2930 access list.
    pub const fn access_list(&self) -> &AccessList {
        &self.access_list
    }

    /// Returns the block selector.
    pub const fn block(&self) -> &EvmBlockSelector {
        &self.block
    }
}

/// Deployed code plus its Keccak-256 identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmCode {
    /// Exact deployed bytes.
    pub bytes: Bytes,
    /// Keccak-256 of `bytes`.
    pub hash: B256,
}

/// Source-bound external read view.
pub trait EvmReadSession: Send + Sync {
    /// Returns the one bind-time provenance value for this session.
    fn evidence(&self) -> &EvmSessionEvidence;

    /// Reads one block identity.
    fn read_block<'a>(&'a self, selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock>;

    /// Reads an account balance.
    fn read_balance<'a>(
        &'a self,
        account: Address,
        block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256>;

    /// Reads deployed code.
    fn read_code<'a>(
        &'a self,
        address: Address,
        block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode>;

    /// Executes a read-only call.
    fn call<'a>(&'a self, request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes>;
}

/// Checked EIP-1559 fee inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmFeeInputs {
    /// Observed base fee per gas.
    pub base_fee_per_gas: U256,
    /// Suggested priority fee per gas.
    pub max_priority_fee_per_gas: U256,
    /// Checked `base_fee * 2 + priority_fee` maximum fee.
    pub max_fee_per_gas: U256,
}

impl EvmFeeInputs {
    /// Builds the one admitted fee policy with checked arithmetic.
    pub fn from_base_and_priority(
        base_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<Self> {
        let max_fee_per_gas = base_fee_per_gas
            .checked_mul(U256::from(2))
            .and_then(|fee| fee.checked_add(max_priority_fee_per_gas))
            .ok_or(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::FeeOverflow,
            })?;
        Ok(Self {
            base_fee_per_gas,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        })
    }
}

/// Exact pre-gas-limit EIP-1559 transaction description supplied to `eth_estimateGas`.
///
/// Construction admits nonce and fee observations into Alloy's narrower
/// EIP-1559 widths before any estimation IO. Adding the returned gas limit to
/// this description produces the signing authority; callers must not rebuild
/// transaction fields from the original intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTransactionEstimate {
    chain_id: U256,
    nonce: U256,
    from: Address,
    to: TxKind,
    value: U256,
    input: Bytes,
    access_list: AccessList,
    max_fee_per_gas: U256,
    max_priority_fee_per_gas: U256,
}

impl EvmTransactionEstimate {
    /// Creates one checked type-2 gas-estimation request.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: U256,
        nonce: U256,
        from: Address,
        to: TxKind,
        value: U256,
        input: Bytes,
        access_list: AccessList,
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<Self> {
        let request = Self {
            chain_id,
            nonce,
            from,
            to,
            value,
            input,
            access_list,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        };
        request.validate_alloy_widths()?;
        Ok(request)
    }

    /// Returns the EIP-2718 transaction type.
    pub const fn transaction_type(&self) -> u8 {
        EVM_EIP1559_TRANSACTION_TYPE
    }

    /// Returns the exact chain id.
    pub const fn chain_id(&self) -> U256 {
        self.chain_id
    }

    /// Returns the exact pending sender nonce.
    pub const fn nonce(&self) -> U256 {
        self.nonce
    }

    /// Returns the maximum total fee per gas.
    pub const fn max_fee_per_gas(&self) -> U256 {
        self.max_fee_per_gas
    }

    /// Returns the maximum priority fee per gas.
    pub const fn max_priority_fee_per_gas(&self) -> U256 {
        self.max_priority_fee_per_gas
    }

    fn validate_alloy_widths(&self) -> Result<()> {
        let chain_id =
            u64::try_from(self.chain_id).map_err(|_| EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::ChainIdOutOfRange,
            })?;
        if chain_id == 0 {
            return Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::ZeroExpectedChainId,
            });
        }
        u64::try_from(self.nonce).map_err(|_| EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::NonceOutOfRange,
        })?;
        let max_fee = u128::try_from(self.max_fee_per_gas).map_err(|_| {
            EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::MaxFeePerGasOutOfRange,
            }
        })?;
        let priority = u128::try_from(self.max_priority_fee_per_gas).map_err(|_| {
            EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::MaxPriorityFeePerGasOutOfRange,
            }
        })?;
        if priority > max_fee {
            return Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::PriorityFeeExceedsMaxFee,
            });
        }
        Ok(())
    }

    /// Returns the sender.
    pub const fn from(&self) -> Address {
        self.from
    }

    /// Returns call or creation destination kind.
    pub const fn to(&self) -> TxKind {
        self.to
    }

    /// Returns transferred value.
    pub const fn value(&self) -> U256 {
        self.value
    }

    /// Returns calldata or init code.
    pub const fn input(&self) -> &Bytes {
        &self.input
    }

    /// Returns the access list.
    pub const fn access_list(&self) -> &AccessList {
        &self.access_list
    }
}

/// Optional block placement of an observed transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTransactionPlacement {
    /// Exact inclusion block identity.
    pub block: EvmBlock,
    /// Transaction index within the block.
    pub transaction_index: U256,
}

/// Public fields of an observed EIP-1559 transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmObservedTransaction {
    /// Transaction hash.
    pub transaction_hash: B256,
    /// Chain id.
    pub chain_id: U256,
    /// Sender nonce.
    pub nonce: U256,
    /// Sender.
    pub from: Address,
    /// Call or creation destination kind.
    pub to: TxKind,
    /// Transferred value.
    pub value: U256,
    /// Calldata or init code.
    pub input: Bytes,
    /// Gas limit.
    pub gas_limit: U256,
    /// Maximum fee per gas.
    pub max_fee_per_gas: U256,
    /// Maximum priority fee per gas.
    pub max_priority_fee_per_gas: U256,
    /// Access list.
    pub access_list: AccessList,
    /// Block placement, or none while pending.
    pub placement: Option<EvmTransactionPlacement>,
}

/// Strict receipt execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvmReceiptStatus {
    /// Execution succeeded.
    Success,
    /// Execution reverted after consuming nonce and gas.
    Reverted,
}

/// Complete log entry carried by a transaction receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmReceiptLog {
    /// Emitting address.
    pub address: Address,
    /// Indexed topics.
    pub topics: Vec<B256>,
    /// Unindexed log data.
    pub data: Bytes,
    /// Exact inclusion block identity.
    pub block: EvmBlock,
    /// Enclosing transaction hash.
    pub transaction_hash: B256,
    /// Transaction index within the block.
    pub transaction_index: U256,
    /// Log index within the block.
    pub log_index: U256,
    /// Whether the provider marked the log removed.
    pub removed: bool,
}

/// Strict transaction receipt observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmReceipt {
    /// Transaction hash.
    pub transaction_hash: B256,
    /// Transaction index within the block.
    pub transaction_index: U256,
    /// Exact inclusion block identity.
    pub block: EvmBlock,
    /// Sender.
    pub from: Address,
    /// Destination, or none for creation.
    pub to: Option<Address>,
    /// Created contract address, when reported.
    pub contract_address: Option<Address>,
    /// Execution status.
    pub status: EvmReceiptStatus,
    /// Gas used by this transaction.
    pub gas_used: U256,
    /// Cumulative gas used in the block.
    pub cumulative_gas_used: U256,
    /// Complete, internally coherent logs.
    pub logs: Vec<EvmReceiptLog>,
}

impl EvmReceipt {
    /// Validates execution status, log identity, and removal status.
    pub fn validate(&self) -> Result<()> {
        if matches!(self.status, EvmReceiptStatus::Reverted) && !self.logs.is_empty() {
            return Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::IncoherentReceipt,
            });
        }
        for log in &self.logs {
            if log.removed
                || log.transaction_hash != self.transaction_hash
                || log.transaction_index != self.transaction_index
                || log.block != self.block
            {
                return Err(EvmCapabilityError::InvalidRequest {
                    reason: EvmInvalidRequest::IncoherentReceipt,
                });
            }
        }
        Ok(())
    }
}

/// Source-bound one-transaction view.
pub trait EvmTransactionSession: Send + Sync {
    /// Returns the one bind-time provenance value for this session.
    fn evidence(&self) -> &EvmSessionEvidence;

    /// Reads the pending nonce for an account.
    fn pending_nonce<'a>(&'a self, account: Address) -> EvmSessionFuture<'a, U256>;

    /// Reads the checked EIP-1559 fee inputs.
    fn fee_inputs(&self) -> EvmSessionFuture<'_, EvmFeeInputs>;

    /// Estimates gas for one transaction intent.
    fn estimate_gas<'a>(
        &'a self,
        request: &'a EvmTransactionEstimate,
    ) -> EvmSessionFuture<'a, U256>;

    /// Submits exact signed bytes and returns the provider-reported hash for caller validation.
    fn submit_raw_transaction<'a>(
        &'a self,
        signed_bytes: &'a [u8],
        expected_hash: B256,
    ) -> EvmSessionFuture<'a, B256>;

    /// Looks up a transaction by its exact hash.
    fn transaction_by_hash(
        &self,
        transaction_hash: B256,
    ) -> EvmSessionFuture<'_, Option<EvmObservedTransaction>>;

    /// Looks up a receipt by its exact transaction hash.
    fn receipt_by_hash(&self, transaction_hash: B256) -> EvmSessionFuture<'_, Option<EvmReceipt>>;

    /// Reads a block identity for confirmation checks.
    fn read_block<'a>(&'a self, selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock>;
}

/// Closed invalid-request reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmInvalidRequest {
    /// Expected chain id was zero.
    ZeroExpectedChainId,
    /// Checked EIP-1559 fee arithmetic overflowed U256.
    FeeOverflow,
    /// Chain id exceeded Alloy's exact EIP-1559 representation.
    ChainIdOutOfRange,
    /// Sender nonce exceeded Alloy's exact EIP-1559 representation.
    NonceOutOfRange,
    /// Maximum fee exceeded Alloy's exact EIP-1559 representation.
    MaxFeePerGasOutOfRange,
    /// Priority fee exceeded Alloy's exact EIP-1559 representation.
    MaxPriorityFeePerGasOutOfRange,
    /// Priority fee exceeded the maximum total fee.
    PriorityFeeExceedsMaxFee,
    /// A read-only call supplied a zero gas limit.
    ZeroCallGasLimit,
    /// Signed transaction bytes were empty.
    EmptySignedTransaction,
    /// Receipt/log identities were inconsistent, a log was removed, or a reverted receipt had
    /// logs.
    IncoherentReceipt,
    /// A persisted block anchor was malformed or non-canonical.
    InvalidBlockAnchor,
    /// A bound session violated the requested semantic or implementation authority.
    SessionAuthorityMismatch,
}

/// Execution phase used to classify a typed EVM capability failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmCapabilityPhase {
    /// A read-only request with no external mutation authority.
    ReadOnly,
    /// Transaction preparation or guarded observation before a submission exchange.
    BeforeSubmission,
    /// Transaction observation or recovery after submission is durably possible.
    AfterSubmission,
}

/// Closed runtime disposition of a typed EVM capability failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmCapabilityFailureDisposition {
    /// Process-local provider authority can be repaired and the same attempt resumed.
    OperationalBlock,
    /// The request or response violated a deterministic certified contract.
    TerminalValidation,
}

/// Redaction-safe EVM capability error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmCapabilityError {
    /// Request or result failed contract validation.
    #[error("EVM capability request or result was invalid")]
    InvalidRequest {
        /// Closed invalid-request reason.
        reason: EvmInvalidRequest,
    },
    /// Provider failed without exposing concrete source details.
    #[error("EVM capability provider failed: {diagnostic}")]
    Provider {
        /// Closed redacted provider diagnostic.
        diagnostic: RedactedProviderDiagnostic,
    },
    /// Bind-time chain identity did not match the semantic binding.
    #[error("EVM source did not match semantic network binding: {diagnostic}")]
    SourceMismatch {
        /// Closed redacted source-mismatch diagnostic.
        diagnostic: RedactedProviderDiagnostic,
    },
}

impl EvmCapabilityError {
    /// Builds a provider failure from a closed redacted diagnostic.
    pub fn provider_failure(diagnostic: RedactedProviderDiagnostic) -> Self {
        Self::Provider { diagnostic }
    }

    /// Returns the closed diagnostic carried by this error.
    pub const fn redacted_diagnostic(&self) -> Option<&RedactedProviderDiagnostic> {
        match self {
            Self::Provider { diagnostic } | Self::SourceMismatch { diagnostic } => Some(diagnostic),
            Self::InvalidRequest { .. } => None,
        }
    }

    /// Classifies this failure without inspecting provider text.
    ///
    /// Once submission is durably possible, no provider, route, transport,
    /// response, or source-binding failure can prove that the transaction
    /// failed or authorize another mutation. Invalid request material remains
    /// terminal in every phase.
    pub fn failure_disposition(
        &self,
        phase: EvmCapabilityPhase,
    ) -> EvmCapabilityFailureDisposition {
        use EvmCapabilityFailureDisposition::{OperationalBlock, TerminalValidation};

        match self {
            Self::InvalidRequest { .. } => TerminalValidation,
            Self::SourceMismatch { .. } => OperationalBlock,
            Self::Provider { .. } if matches!(phase, EvmCapabilityPhase::AfterSubmission) => {
                OperationalBlock
            }
            Self::Provider { diagnostic } => match diagnostic.code() {
                ProviderDiagnosticCode::ProviderConfigurationMissing
                | ProviderDiagnosticCode::ProviderConfigurationInvalid
                | ProviderDiagnosticCode::RouteUnavailable
                | ProviderDiagnosticCode::SourceUnavailable
                | ProviderDiagnosticCode::TransportFailed
                | ProviderDiagnosticCode::RpcHttpStatus
                | ProviderDiagnosticCode::OperationIncomplete => OperationalBlock,
                ProviderDiagnosticCode::SourceNotAllowed
                | ProviderDiagnosticCode::RpcJsonError
                | ProviderDiagnosticCode::ResponseInvalid
                | ProviderDiagnosticCode::ResponseMissingResult
                | ProviderDiagnosticCode::SourceMismatch
                | ProviderDiagnosticCode::UnsupportedOperation => TerminalValidation,
            },
        }
    }
}

/// Builds a closed redacted source mismatch.
pub fn source_mismatch_error(
    binding: &EvmNetworkBinding,
    observed_chain_id: U256,
    source_ref: &LocalPublicId,
) -> EvmCapabilityError {
    let diagnostic = evm_diagnostic(ProviderDiagnosticCode::SourceMismatch)
        .with_field(
            public_id("network_id"),
            ProviderDiagnosticValue::Id(binding.network_id.clone()),
        )
        .with_field(
            public_id("expected_chain_id"),
            ProviderDiagnosticValue::U64(binding.expected_chain_id()),
        )
        .with_field(
            public_id("source_ref"),
            ProviderDiagnosticValue::Id(source_ref.clone()),
        );
    let diagnostic = match u64::try_from(observed_chain_id) {
        Ok(observed_chain_id) => diagnostic.with_field(
            public_id("observed_chain_id"),
            ProviderDiagnosticValue::U64(observed_chain_id),
        ),
        Err(_) => diagnostic.with_field(
            public_id("observed_chain_id_out_of_range"),
            ProviderDiagnosticValue::Bool(true),
        ),
    };
    EvmCapabilityError::SourceMismatch { diagnostic }
}

/// Builds a closed redacted EVM provider diagnostic.
pub fn evm_diagnostic(code: ProviderDiagnosticCode) -> RedactedProviderDiagnostic {
    RedactedProviderDiagnostic::new(public_id("evm"), code)
}

fn public_id(value: &str) -> LocalPublicId {
    LocalPublicId::new(value).expect("EVM diagnostic id must be checked public text")
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
