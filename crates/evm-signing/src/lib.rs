#![warn(missing_docs)]
//! Canonical EIP-1559 transaction signing.
//!
//! This crate admits one checked Alloy EIP-1559 envelope, converts its
//! signing digest into the generic MFM signing contract, verifies the exact
//! deterministic signature profile and expected sender, and returns a
//! transient signed envelope. Alloy is the only transaction hashing and
//! encoding implementation.
//!
//! ```rust
//! use alloy_eips::eip2930::AccessList;
//! use alloy_primitives::{address, Bytes, TxKind, U256};
//! use mfm_evm_signing::UnsignedEip1559Envelope;
//!
//! let envelope = UnsignedEip1559Envelope::new(
//!     U256::from(1),
//!     U256::ZERO,
//!     U256::from(1_000_000_000_u64),
//!     U256::from(2_000_000_000_u64),
//!     U256::from(21_000),
//!     TxKind::Call(address!("1111111111111111111111111111111111111111")),
//!     U256::ZERO,
//!     AccessList::default(),
//!     Bytes::new(),
//! )?;
//! assert_ne!(envelope.signing_digest(), alloy_primitives::B256::ZERO);
//! # Ok::<(), mfm_evm_signing::EvmSigningError>(())
//! ```

use std::fmt;

use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_eips::eip2930::AccessList;
use alloy_primitives::{keccak256, Address, Bytes, PrimitiveSignature, TxKind, B256, U256};
use mfm_signing::{
    DeterministicSigningProvider, ExpectedSignerIdentity, SignerRef, SigningAlgorithmId,
    SigningDomainId, SigningError, SigningProfileId, SigningPurposeId, SigningRequest,
    SigningResult, SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID,
    SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};

/// Result type for canonical EVM signing operations.
pub type Result<T> = std::result::Result<T, EvmSigningError>;

/// EVM transaction signing domain id.
pub const EVM_TRANSACTION_DOMAIN_ID: &str = "evm.transaction";
/// EIP-1559 transaction signing purpose id.
pub const EVM_EIP1559_TRANSACTION_PURPOSE_ID: &str = "evm.transaction.eip1559";

/// One checked, unsigned Alloy EIP-1559 envelope.
#[derive(Clone, PartialEq, Eq)]
pub struct UnsignedEip1559Envelope {
    transaction: TxEip1559,
}

impl UnsignedEip1559Envelope {
    /// Admits U256 transaction quantities into Alloy's EIP-1559 model.
    ///
    /// Alloy currently represents the chain id, nonce, and gas limit as
    /// `u64`, and fee fields as `u128`. Values outside those exact ranges are
    /// rejected; they are never truncated, clamped, or defaulted.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: U256,
        nonce: U256,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
        gas_limit: U256,
        to: TxKind,
        value: U256,
        access_list: AccessList,
        input: Bytes,
    ) -> Result<Self> {
        let chain_id = to_u64(chain_id, Eip1559QuantityField::ChainId)?;
        if chain_id == 0 {
            return Err(EvmSigningError::ZeroChainId);
        }
        let nonce = to_u64(nonce, Eip1559QuantityField::Nonce)?;
        let gas_limit = to_u64(gas_limit, Eip1559QuantityField::GasLimit)?;
        let max_priority_fee_per_gas = to_u128(
            max_priority_fee_per_gas,
            Eip1559QuantityField::MaxPriorityFeePerGas,
        )?;
        let max_fee_per_gas = to_u128(max_fee_per_gas, Eip1559QuantityField::MaxFeePerGas)?;
        if max_priority_fee_per_gas > max_fee_per_gas {
            return Err(EvmSigningError::PriorityFeeExceedsMaxFee);
        }

        Ok(Self {
            transaction: TxEip1559 {
                chain_id,
                nonce,
                gas_limit,
                max_fee_per_gas,
                max_priority_fee_per_gas,
                to,
                value,
                access_list,
                input,
            },
        })
    }

    /// Returns the chain id as the canonical in-memory quantity type.
    pub fn chain_id(&self) -> U256 {
        U256::from(self.transaction.chain_id)
    }

    /// Returns the sender nonce as the canonical in-memory quantity type.
    pub fn nonce(&self) -> U256 {
        U256::from(self.transaction.nonce)
    }

    /// Returns the maximum priority fee per gas.
    pub fn max_priority_fee_per_gas(&self) -> U256 {
        U256::from(self.transaction.max_priority_fee_per_gas)
    }

    /// Returns the maximum total fee per gas.
    pub fn max_fee_per_gas(&self) -> U256 {
        U256::from(self.transaction.max_fee_per_gas)
    }

    /// Returns the gas limit as the canonical in-memory quantity type.
    pub fn gas_limit(&self) -> U256 {
        U256::from(self.transaction.gas_limit)
    }

    /// Returns the transaction destination kind.
    pub const fn to(&self) -> TxKind {
        self.transaction.to
    }

    /// Returns the transferred value.
    pub const fn value(&self) -> U256 {
        self.transaction.value
    }

    /// Returns the access list.
    pub const fn access_list(&self) -> &AccessList {
        &self.transaction.access_list
    }

    /// Returns the call data or creation init code.
    pub const fn input(&self) -> &Bytes {
        &self.transaction.input
    }

    /// Returns the EIP-1559 signing digest produced by Alloy.
    pub fn signing_digest(&self) -> B256 {
        self.transaction.signature_hash()
    }

    /// Builds the one generic signing request for this envelope.
    pub fn signing_request(
        &self,
        signer_ref: SignerRef,
        expected_sender: Address,
    ) -> Result<SigningRequest> {
        let request = SigningRequest::from_digest(
            signer_ref,
            evm_signing_algorithm_id()?,
            evm_deterministic_signing_profile_id()?,
            SigningDomainId::new(EVM_TRANSACTION_DOMAIN_ID)?,
            SigningPurposeId::new(EVM_EIP1559_TRANSACTION_PURPOSE_ID)?,
            mfm_ids::DigestBytes::from_array(self.signing_digest().0),
        );
        Ok(
            request.require_public_identity(ExpectedSignerIdentity::account_id(format!(
                "{expected_sender:?}"
            ))?),
        )
    }

    /// Verifies one provider result and finalizes the exact Alloy envelope.
    pub fn finalize_signed(
        &self,
        request: &SigningRequest,
        expected_sender: Address,
        signing_result: &SigningResult,
    ) -> Result<TransientSignedEip1559Envelope> {
        verify_result_contract(request, signing_result, expected_sender)?;
        let signature = strict_primitive_signature(signing_result.signature().as_bytes())?;
        let recovered = signature
            .recover_address_from_prehash(&self.signing_digest())
            .map_err(|_| EvmSigningError::InvalidSignature {
                reason: EvmSignatureError::RecoveryFailed,
            })?;
        if recovered != expected_sender {
            return Err(EvmSigningError::RecoveredAddressMismatch);
        }

        let signed = self.transaction.clone().into_signed(signature);
        let mut bytes = Vec::with_capacity(signed.eip2718_encoded_length());
        signed.eip2718_encode(&mut bytes);
        let transaction_hash = keccak256(&bytes);
        if transaction_hash != *signed.hash() {
            return Err(EvmSigningError::SignedHashMismatch);
        }
        Ok(TransientSignedEip1559Envelope {
            bytes,
            transaction_hash,
        })
    }
}

impl fmt::Debug for UnsignedEip1559Envelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnsignedEip1559Envelope")
            .field("chain_id", &self.chain_id())
            .field("nonce", &self.nonce())
            .field("max_priority_fee_per_gas", &self.max_priority_fee_per_gas())
            .field("max_fee_per_gas", &self.max_fee_per_gas())
            .field("gas_limit", &self.gas_limit())
            .field("to", &self.to())
            .field("value", &self.value())
            .field("access_list", &self.access_list())
            .field("input", &"<redacted>")
            .finish()
    }
}

/// Transient, bearer-material signed EIP-1559 envelope.
///
/// This type is intentionally neither serializable nor cloneable. Its bytes
/// may cross only an explicit submission or user-selected output boundary.
#[derive(PartialEq, Eq)]
pub struct TransientSignedEip1559Envelope {
    bytes: Vec<u8>,
    transaction_hash: B256,
}

impl TransientSignedEip1559Envelope {
    /// Returns the exact EIP-2718 signed transaction bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the Keccak-256 hash of the exact signed bytes.
    pub const fn transaction_hash(&self) -> B256 {
        self.transaction_hash
    }
}

impl fmt::Debug for TransientSignedEip1559Envelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransientSignedEip1559Envelope")
            .field("bytes", &"<redacted>")
            .field("transaction_hash", &self.transaction_hash)
            .finish()
    }
}

/// Signs one checked envelope through one generic provider call.
pub async fn sign_eip1559(
    envelope: &UnsignedEip1559Envelope,
    signer_ref: SignerRef,
    expected_sender: Address,
    provider: &dyn DeterministicSigningProvider,
) -> Result<TransientSignedEip1559Envelope> {
    if provider.deterministic_profile_id() != SECP256K1_RFC6979_LOW_S_PROFILE_ID {
        return Err(EvmSigningError::DeterministicProfileMismatch);
    }
    let request = envelope.signing_request(signer_ref, expected_sender)?;
    let result = provider.sign(&request).await?;
    envelope.finalize_signed(&request, expected_sender, &result)
}

/// Returns the generic recoverable secp256k1 EVM signing algorithm id.
pub fn evm_signing_algorithm_id() -> Result<SigningAlgorithmId> {
    Ok(SigningAlgorithmId::new(
        SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID,
    )?)
}

/// Returns the deterministic canonical EVM signing profile id.
pub fn evm_deterministic_signing_profile_id() -> Result<SigningProfileId> {
    Ok(SigningProfileId::new(SECP256K1_RFC6979_LOW_S_PROFILE_ID)?)
}

fn verify_result_contract(
    request: &SigningRequest,
    result: &SigningResult,
    expected_sender: Address,
) -> Result<()> {
    if result.signer_ref() != request.signer_ref() {
        return Err(EvmSigningError::SigningResultMismatch {
            field: "signer_ref",
        });
    }
    if result.algorithm() != request.algorithm() {
        return Err(EvmSigningError::SigningResultMismatch { field: "algorithm" });
    }
    if result.profile() != request.profile() {
        return Err(EvmSigningError::SigningResultMismatch { field: "profile" });
    }
    let expected_account = format!("{expected_sender:?}");
    if result.public_identity().account_id() != Some(expected_account.as_str()) {
        return Err(EvmSigningError::SigningResultMismatch {
            field: "public_identity",
        });
    }
    Ok(())
}

fn strict_primitive_signature(bytes: &[u8]) -> Result<PrimitiveSignature> {
    let raw: &[u8; 65] = bytes
        .try_into()
        .map_err(|_| EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::InvalidLength,
        })?;
    if !matches!(raw[64], 27 | 28) {
        return Err(EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::InvalidParity,
        });
    }
    let signature =
        PrimitiveSignature::from_raw_array(raw).map_err(|_| EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::InvalidParity,
        })?;
    if signature.normalize_s().is_some() {
        return Err(EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::HighS,
        });
    }
    Ok(signature)
}

fn to_u64(value: U256, field: Eip1559QuantityField) -> Result<u64> {
    value
        .try_into()
        .map_err(|_| EvmSigningError::QuantityOutOfRange { field })
}

fn to_u128(value: U256, field: Eip1559QuantityField) -> Result<u128> {
    value
        .try_into()
        .map_err(|_| EvmSigningError::QuantityOutOfRange { field })
}

/// EIP-1559 quantity fields with narrower Alloy representations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eip1559QuantityField {
    /// Chain id.
    ChainId,
    /// Sender nonce.
    Nonce,
    /// Gas limit.
    GasLimit,
    /// Maximum fee per gas.
    MaxFeePerGas,
    /// Maximum priority fee per gas.
    MaxPriorityFeePerGas,
}

/// Redaction-safe EVM signing failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmSigningError {
    /// A U256 input was outside Alloy's exact representation.
    #[error("EIP-1559 quantity was outside the supported range for {field:?}")]
    QuantityOutOfRange {
        /// Field that could not be represented exactly.
        field: Eip1559QuantityField,
    },
    /// Chain id zero is not admitted.
    #[error("EIP-1559 chain id must be non-zero")]
    ZeroChainId,
    /// Priority fee exceeded the maximum total fee.
    #[error("EIP-1559 priority fee must not exceed maximum fee")]
    PriorityFeeExceedsMaxFee,
    /// Generic signing contract failed.
    #[error("EVM signing request failed: {0}")]
    Signing(#[from] SigningError),
    /// Bound provider did not certify the required deterministic profile.
    #[error("EVM signer did not certify the required deterministic profile")]
    DeterministicProfileMismatch,
    /// Provider result did not match the exact request.
    #[error("EVM signing result mismatch for {field}")]
    SigningResultMismatch {
        /// Result field that mismatched.
        field: &'static str,
    },
    /// Provider signature failed strict canonical validation.
    #[error("{reason}")]
    InvalidSignature {
        /// Closed signature failure reason.
        reason: EvmSignatureError,
    },
    /// Recovered address did not match the expected sender.
    #[error("EVM recovered signer address did not match expected address")]
    RecoveredAddressMismatch,
    /// Alloy's signed hash disagreed with the exact encoded bytes.
    #[error("EVM signed transaction hash did not match encoded bytes")]
    SignedHashMismatch,
}

/// Closed strict-signature validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmSignatureError {
    /// Signature was not exactly 65 bytes.
    #[error("EVM signature length was invalid")]
    InvalidLength,
    /// Signature did not use canonical 27/28 recoverable parity.
    #[error("EVM signature parity was invalid")]
    InvalidParity,
    /// Signature used a malleable high-s scalar.
    #[error("EVM signature was not low-s canonical")]
    HighS,
    /// Public address recovery failed.
    #[error("EVM signature recovery failed")]
    RecoveryFailed,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
