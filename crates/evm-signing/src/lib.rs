#![warn(missing_docs)]
//! EVM signing bridge contracts.
//!
//! This crate turns reusable EVM transaction models into digest-only MFM
//! signing requests, verifies signer public identity by ECDSA address recovery,
//! and materializes transient raw transactions after a provider returns a
//! signature. It does not load private keys, resolve signer providers, submit
//! transactions, or persist raw signed transactions.

use std::fmt;

use alloy_primitives::{Address, PrimitiveSignature, B256};
use mfm_evm_core::hex::bytes_to_hex_prefixed;
use mfm_evm_core::tx::{
    eip1559_signing_hash, encode_signed_eip1559_tx, encode_signed_legacy_tx, legacy_signing_hash,
    raw_transaction_hash_bytes, Eip1559TxToSign, LegacyTxToSign,
};
use mfm_signing::{
    ExpectedSignerIdentity, SignatureBytes, SignerRef, SigningAlgorithmId, SigningDomainId,
    SigningError, SigningPurposeId, SigningRequest, SigningResult,
};

/// Result type for EVM signing bridge operations.
pub type Result<T> = std::result::Result<T, EvmSigningError>;

/// EVM secp256k1 signing algorithm id.
pub const EVM_SIGNING_ALGORITHM_ID: &str = "evm.secp256k1.keccak256";
/// EVM transaction signing domain id.
pub const EVM_TRANSACTION_DOMAIN_ID: &str = "evm.transaction";
/// Legacy EIP-155 transaction signing purpose id.
pub const EVM_LEGACY_TRANSACTION_PURPOSE_ID: &str = "evm.transaction.legacy";
/// EIP-1559 transaction signing purpose id.
pub const EVM_EIP1559_TRANSACTION_PURPOSE_ID: &str = "evm.transaction.eip1559";

/// EVM transaction signing style.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EvmTransactionStyle {
    /// Legacy EIP-155 transaction signing.
    Legacy,
    /// EIP-1559 typed transaction signing.
    #[default]
    Eip1559,
}

/// EVM transaction plus digest-only generic signing request.
#[derive(Debug)]
pub struct EvmSigningRequest {
    transaction: EvmSigningTransaction,
    signing_request: SigningRequest,
    signing_hash: B256,
    expected_from: Address,
}

impl EvmSigningRequest {
    /// Creates an EIP-155 legacy transaction signing request.
    pub fn legacy(
        signer_ref: SignerRef,
        tx: LegacyTxToSign,
        expected_from: Address,
    ) -> Result<Self> {
        Self::new(signer_ref, EvmSigningTransaction::Legacy(tx), expected_from)
    }

    /// Creates an EIP-1559 transaction signing request.
    pub fn eip1559(
        signer_ref: SignerRef,
        tx: Eip1559TxToSign,
        expected_from: Address,
    ) -> Result<Self> {
        Self::new(
            signer_ref,
            EvmSigningTransaction::Eip1559(tx),
            expected_from,
        )
    }

    fn new(
        signer_ref: SignerRef,
        transaction: EvmSigningTransaction,
        expected_from: Address,
    ) -> Result<Self> {
        let (signing_hash, purpose) = match &transaction {
            EvmSigningTransaction::Legacy(tx) => {
                (legacy_signing_hash(tx), legacy_transaction_purpose_id()?)
            }
            EvmSigningTransaction::Eip1559(tx) => {
                (eip1559_signing_hash(tx), eip1559_transaction_purpose_id()?)
            }
        };
        let signing_request = signing_request(signer_ref, purpose, signing_hash, expected_from)?;
        Ok(Self {
            transaction,
            signing_request,
            signing_hash,
            expected_from,
        })
    }

    /// Returns the transaction style.
    pub const fn style(&self) -> EvmTransactionStyle {
        match self.transaction {
            EvmSigningTransaction::Legacy(_) => EvmTransactionStyle::Legacy,
            EvmSigningTransaction::Eip1559(_) => EvmTransactionStyle::Eip1559,
        }
    }

    /// Returns the generic digest-only signing request.
    pub fn signing_request(&self) -> &SigningRequest {
        &self.signing_request
    }

    /// Returns the EVM signing hash.
    pub const fn signing_hash(&self) -> B256 {
        self.signing_hash
    }

    /// Returns the expected sender address.
    pub const fn expected_from(&self) -> Address {
        self.expected_from
    }

    /// Verifies the provider result and materializes a transient signed payload.
    pub fn materialize_signed_payload(
        &self,
        signing_result: &SigningResult,
    ) -> Result<TransientRawTransaction> {
        if signing_result.signer_ref() != self.signing_request.signer_ref() {
            return Err(EvmSigningError::SigningResultMismatch {
                field: "signer_ref",
            });
        }
        if signing_result.algorithm() != self.signing_request.algorithm() {
            return Err(EvmSigningError::SigningResultMismatch { field: "algorithm" });
        }

        let signature = primitive_signature_from_bytes(signing_result.signature())?;
        let recovered = recover_signing_address(self.signing_hash, signature)?;
        if recovered != self.expected_from {
            return Err(EvmSigningError::RecoveredAddressMismatch);
        }

        let raw = match &self.transaction {
            EvmSigningTransaction::Legacy(tx) => encode_signed_legacy_tx(tx, signature),
            EvmSigningTransaction::Eip1559(tx) => encode_signed_eip1559_tx(tx, signature),
        };
        TransientRawTransaction::new(raw)
    }
}

/// Transient EVM transaction model held while preparing a signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmSigningTransaction {
    /// Legacy EIP-155 transaction fields.
    Legacy(LegacyTxToSign),
    /// EIP-1559 transaction fields.
    Eip1559(Eip1559TxToSign),
}

/// Transient raw signed transaction. This type is intentionally not serializable.
#[derive(Clone, PartialEq, Eq)]
pub struct TransientRawTransaction {
    raw: Vec<u8>,
    transaction_hash: String,
}

impl TransientRawTransaction {
    /// Creates a transient raw transaction and computes its transaction hash.
    fn new(raw: Vec<u8>) -> Result<Self> {
        if raw.is_empty() {
            return Err(EvmSigningError::InvalidRawTransaction);
        }
        let transaction_hash =
            raw_transaction_hash_bytes(&raw).map_err(|_| EvmSigningError::InvalidRawTransaction)?;
        Ok(Self {
            raw,
            transaction_hash,
        })
    }

    /// Returns raw signed transaction bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }

    /// Returns raw signed transaction bytes as `0x`-prefixed hex.
    pub fn hex(&self) -> String {
        bytes_to_hex_prefixed(&self.raw)
    }

    /// Returns the EVM transaction hash as `0x`-prefixed hex.
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }
}

impl fmt::Debug for TransientRawTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransientRawTransaction")
            .field("raw", &"<redacted>")
            .field("transaction_hash", &self.transaction_hash)
            .finish()
    }
}

/// Returns the EVM signing algorithm identifier.
pub fn evm_signing_algorithm_id() -> Result<SigningAlgorithmId> {
    Ok(SigningAlgorithmId::new(EVM_SIGNING_ALGORITHM_ID)?)
}

/// Returns the EVM transaction signing domain identifier.
pub fn evm_transaction_domain_id() -> Result<SigningDomainId> {
    Ok(SigningDomainId::new(EVM_TRANSACTION_DOMAIN_ID)?)
}

/// Returns the legacy EVM transaction signing purpose identifier.
pub fn legacy_transaction_purpose_id() -> Result<SigningPurposeId> {
    Ok(SigningPurposeId::new(EVM_LEGACY_TRANSACTION_PURPOSE_ID)?)
}

/// Returns the EIP-1559 EVM transaction signing purpose identifier.
pub fn eip1559_transaction_purpose_id() -> Result<SigningPurposeId> {
    Ok(SigningPurposeId::new(EVM_EIP1559_TRANSACTION_PURPOSE_ID)?)
}

/// Converts a provider signature into a normalized EVM primitive signature.
pub fn primitive_signature_from_bytes(signature: &SignatureBytes) -> Result<PrimitiveSignature> {
    let bytes = signature.as_bytes();
    let raw: &[u8; 65] = bytes
        .try_into()
        .map_err(|_| EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::InvalidLength,
        })?;
    PrimitiveSignature::from_raw_array(raw)
        .map(PrimitiveSignature::normalized_s)
        .map_err(|_| EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::InvalidParity,
        })
}

/// Recovers the EVM signing address from a hash and normalized signature.
pub fn recover_signing_address(
    signing_hash: B256,
    signature: PrimitiveSignature,
) -> Result<Address> {
    signature
        .recover_address_from_prehash(&signing_hash)
        .map_err(|_| EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::RecoveryFailed,
        })
}

fn signing_request(
    signer_ref: SignerRef,
    purpose: SigningPurposeId,
    signing_hash: B256,
    expected_from: Address,
) -> Result<SigningRequest> {
    let request = SigningRequest::from_digest(
        signer_ref,
        evm_signing_algorithm_id()?,
        evm_transaction_domain_id()?,
        purpose,
        digest_from_hash(signing_hash),
    );
    Ok(
        request.require_public_identity(ExpectedSignerIdentity::account_id(format!(
            "{expected_from:?}"
        ))?),
    )
}

fn digest_from_hash(hash: B256) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array(hash.0)
}

/// Redaction-safe EVM signing bridge error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmSigningError {
    /// Generic signing contract failed.
    #[error("EVM signing request failed: {0}")]
    Signing(#[from] SigningError),
    /// Provider result did not match the request.
    #[error("EVM signing result mismatch for {field}")]
    SigningResultMismatch {
        /// Result field that mismatched.
        field: &'static str,
    },
    /// Provider signature was invalid for EVM signing.
    #[error("{reason}")]
    InvalidSignature {
        /// Closed signature failure reason.
        reason: EvmSignatureError,
    },
    /// Recovered address did not match the expected sender.
    #[error("EVM recovered signer address did not match expected address")]
    RecoveredAddressMismatch,
    /// Raw transaction bytes were invalid.
    #[error("EVM raw transaction was invalid")]
    InvalidRawTransaction,
}

/// Closed EVM signature validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmSignatureError {
    /// Signature bytes were not 65 bytes.
    #[error("EVM signature length was invalid")]
    InvalidLength,
    /// Signature parity byte was invalid.
    #[error("EVM signature parity was invalid")]
    InvalidParity,
    /// Public address recovery failed.
    #[error("EVM signature recovery failed")]
    RecoveryFailed,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
