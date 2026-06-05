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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmTransactionStyle {
    /// Legacy EIP-155 transaction signing.
    Legacy,
    /// EIP-1559 typed transaction signing.
    Eip1559,
}

impl Default for EvmTransactionStyle {
    fn default() -> Self {
        Self::Eip1559
    }
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
        let signing_hash = legacy_signing_hash(&tx);
        let signing_request = signing_request(
            signer_ref,
            legacy_transaction_purpose_id()?,
            signing_hash,
            expected_from,
        )?;
        Ok(Self {
            transaction: EvmSigningTransaction::Legacy(tx),
            signing_request,
            signing_hash,
            expected_from,
        })
    }

    /// Creates an EIP-1559 transaction signing request.
    pub fn eip1559(
        signer_ref: SignerRef,
        tx: Eip1559TxToSign,
        expected_from: Address,
    ) -> Result<Self> {
        let signing_hash = eip1559_signing_hash(&tx);
        let signing_request = signing_request(
            signer_ref,
            eip1559_transaction_purpose_id()?,
            signing_hash,
            expected_from,
        )?;
        Ok(Self {
            transaction: EvmSigningTransaction::Eip1559(tx),
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

    /// Verifies the provider result and materializes a transient raw signed transaction.
    pub fn materialize_raw_transaction(
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
    SigningAlgorithmId::new(EVM_SIGNING_ALGORITHM_ID).map_err(EvmSigningError::Signing)
}

/// Returns the EVM transaction signing domain identifier.
pub fn evm_transaction_domain_id() -> Result<SigningDomainId> {
    SigningDomainId::new(EVM_TRANSACTION_DOMAIN_ID).map_err(EvmSigningError::Signing)
}

/// Returns the legacy EVM transaction signing purpose identifier.
pub fn legacy_transaction_purpose_id() -> Result<SigningPurposeId> {
    SigningPurposeId::new(EVM_LEGACY_TRANSACTION_PURPOSE_ID).map_err(EvmSigningError::Signing)
}

/// Returns the EIP-1559 EVM transaction signing purpose identifier.
pub fn eip1559_transaction_purpose_id() -> Result<SigningPurposeId> {
    SigningPurposeId::new(EVM_EIP1559_TRANSACTION_PURPOSE_ID).map_err(EvmSigningError::Signing)
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
    Ok(request.require_public_identity(
        ExpectedSignerIdentity::account_id(format!("{expected_from:?}"))
            .map_err(EvmSigningError::Signing)?,
    ))
}

fn digest_from_hash(hash: B256) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array(hash.0)
}

/// Redaction-safe EVM signing bridge error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmSigningError {
    /// Generic signing contract failed.
    Signing(SigningError),
    /// Provider result did not match the request.
    SigningResultMismatch {
        /// Result field that mismatched.
        field: &'static str,
    },
    /// Provider signature was invalid for EVM signing.
    InvalidSignature {
        /// Closed signature failure reason.
        reason: EvmSignatureError,
    },
    /// Recovered address did not match the expected sender.
    RecoveredAddressMismatch,
    /// Raw transaction bytes were invalid.
    InvalidRawTransaction,
}

impl fmt::Display for EvmSigningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signing(error) => write!(f, "EVM signing request failed: {error}"),
            Self::SigningResultMismatch { field } => {
                write!(f, "EVM signing result mismatch for {field}")
            }
            Self::InvalidSignature { reason } => match reason {
                EvmSignatureError::InvalidLength => f.write_str("EVM signature length was invalid"),
                EvmSignatureError::InvalidParity => f.write_str("EVM signature parity was invalid"),
                EvmSignatureError::RecoveryFailed => f.write_str("EVM signature recovery failed"),
            },
            Self::RecoveredAddressMismatch => {
                f.write_str("EVM recovered signer address did not match expected address")
            }
            Self::InvalidRawTransaction => f.write_str("EVM raw transaction was invalid"),
        }
    }
}

impl std::error::Error for EvmSigningError {}

/// Closed EVM signature validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmSignatureError {
    /// Signature bytes were not 65 bytes.
    InvalidLength,
    /// Signature parity byte was invalid.
    InvalidParity,
    /// Public address recovery failed.
    RecoveryFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use mfm_signing::PublicSigningIdentity;

    fn signer_ref() -> SignerRef {
        SignerRef::new("deployer").expect("signer ref")
    }

    fn legacy_tx() -> LegacyTxToSign {
        LegacyTxToSign {
            to: None,
            value_wei: 0,
            chain_id: 1,
            nonce: 7,
            gas_price_wei: 1,
            gas_limit: 21_000,
            data: vec![0xde, 0xad, 0xbe, 0xef],
        }
    }

    fn eip1559_tx() -> Eip1559TxToSign {
        Eip1559TxToSign {
            to: Address::from([0x11; 20]),
            value_wei: 0,
            chain_id: 1,
            nonce: 7,
            max_fee_per_gas: 2,
            max_priority_fee_per_gas: 1,
            gas_limit: 21_000,
            data: vec![0xca, 0xfe],
        }
    }

    fn expected_sender() -> Address {
        address!("0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e")
    }

    fn recovered_signature_bytes() -> SignatureBytes {
        SignatureBytes::new(
            hex_literal(
                "48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353\
                 efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b",
            )
            .to_vec(),
        )
        .expect("signature bytes")
    }

    fn provider_identity(account: Address) -> PublicSigningIdentity {
        PublicSigningIdentity::new(
            evm_signing_algorithm_id().expect("algorithm"),
            None,
            Some(format!("{account:?}")),
        )
        .expect("identity")
    }

    #[test]
    fn default_transaction_style_is_eip1559() {
        assert_eq!(EvmTransactionStyle::default(), EvmTransactionStyle::Eip1559);
    }

    #[test]
    fn legacy_tx_hash_is_stable() {
        let request = EvmSigningRequest::legacy(signer_ref(), legacy_tx(), expected_sender())
            .expect("request");

        assert_eq!(
            format!("{:?}", request.signing_hash()),
            "0x7b5763c12ba4587de9d52aac395936808bf4d52eb0d6cc0a8803011825d8aa55"
        );
        assert_eq!(
            request.signing_request().purpose().as_str(),
            EVM_LEGACY_TRANSACTION_PURPOSE_ID
        );
        assert_eq!(request.expected_from(), expected_sender());
        assert!(request.signing_request().expected_identity().is_some());
    }

    #[test]
    fn eip1559_tx_hash_is_stable() {
        let request = EvmSigningRequest::eip1559(signer_ref(), eip1559_tx(), expected_sender())
            .expect("request");

        assert_eq!(
            format!("{:?}", request.signing_hash()),
            "0x57806671c35732b1b46a1f8c8a7d63844b94639d997b46c482ea0062d86a8185"
        );
        assert_eq!(
            request.signing_request().purpose().as_str(),
            EVM_EIP1559_TRANSACTION_PURPOSE_ID
        );
        assert_eq!(request.expected_from(), expected_sender());
        assert!(request.signing_request().expected_identity().is_some());
    }

    #[test]
    fn recovered_address_matches_expected_signer_address() {
        let signing_hash = "0x5eb4f5a33c621f32a8622d5f943b6b102994dfe4e5aebbefe69bb1b2aa0fc93e"
            .parse::<B256>()
            .expect("hash");
        let signature =
            primitive_signature_from_bytes(&recovered_signature_bytes()).expect("signature");

        assert_eq!(
            recover_signing_address(signing_hash, signature).expect("recover"),
            address!("0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e")
        );
    }

    #[test]
    fn materializes_transient_legacy_raw_transaction() {
        let expected = expected_sender();
        let signing_hash = "0x5eb4f5a33c621f32a8622d5f943b6b102994dfe4e5aebbefe69bb1b2aa0fc93e"
            .parse::<B256>()
            .expect("hash");
        let request = EvmSigningRequest {
            transaction: EvmSigningTransaction::Legacy(legacy_tx()),
            signing_request: SigningRequest::from_digest(
                signer_ref(),
                evm_signing_algorithm_id().expect("algorithm"),
                evm_transaction_domain_id().expect("domain"),
                legacy_transaction_purpose_id().expect("purpose"),
                digest_from_hash(signing_hash),
            )
            .require_public_identity(
                ExpectedSignerIdentity::account_id(format!("{expected:?}")).expect("expected"),
            ),
            signing_hash,
            expected_from: expected,
        };
        let result = SigningResult::for_request(
            request.signing_request(),
            provider_identity(expected),
            recovered_signature_bytes(),
        )
        .expect("signing result");

        let raw = request
            .materialize_raw_transaction(&result)
            .expect("raw transaction");

        assert!(raw.hex().starts_with("0x"));
        assert!(raw.transaction_hash().starts_with("0x"));
        assert!(!format!("{raw:?}").contains(raw.hex().as_str()));
    }

    #[test]
    fn materialize_rejects_recovered_address_mismatch() {
        let expected = address!("0x1111111111111111111111111111111111111111");
        let signing_hash = "0x5eb4f5a33c621f32a8622d5f943b6b102994dfe4e5aebbefe69bb1b2aa0fc93e"
            .parse::<B256>()
            .expect("hash");
        let request = EvmSigningRequest {
            transaction: EvmSigningTransaction::Legacy(legacy_tx()),
            signing_request: SigningRequest::from_digest(
                signer_ref(),
                evm_signing_algorithm_id().expect("algorithm"),
                evm_transaction_domain_id().expect("domain"),
                legacy_transaction_purpose_id().expect("purpose"),
                digest_from_hash(signing_hash),
            )
            .require_public_identity(
                ExpectedSignerIdentity::account_id(format!("{expected:?}")).expect("expected"),
            ),
            signing_hash,
            expected_from: expected,
        };
        let result = SigningResult::for_request(
            request.signing_request(),
            provider_identity(expected),
            recovered_signature_bytes(),
        )
        .expect("signing result");

        assert_eq!(
            request.materialize_raw_transaction(&result),
            Err(EvmSigningError::RecoveredAddressMismatch)
        );
    }

    fn hex_literal(raw: &str) -> Vec<u8> {
        let compact = raw.split_whitespace().collect::<String>();
        hex::decode(compact).expect("hex")
    }
}
