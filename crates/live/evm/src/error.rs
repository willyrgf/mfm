use alloy_primitives::ruint::{BaseConvertError, ParseError};
use mfm_evm::{
    EvmAddress, EvmAuthorityEpoch, EvmBalanceTarget, EvmBlockAnchor, EvmChainInstance,
    EvmDomainError, EvmHash, EvmReadSubject, EvmTransactionBinding, NonceDomain,
    ReadCapabilityFamily, Reservation,
};
use mfm_ids::{ContentRef, EffectId};
use serde::Serialize;

use crate::{codec::EvmCodecError, transaction::ProviderReceiptResult};

/// Actual Live-owned local checks and native conversions, never a persisted domain failure.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdapterFailure {
    #[error("read binding does not match the registered route")]
    ReadBinding {
        expected_route: ContentRef,
        observed_route: ContentRef,
        expected_chain: u64,
        observed_chain: u64,
    },
    #[error("read subject does not match the registered capability")]
    ReadSubject {
        expected: ReadCapabilityFamily,
        observed: EvmReadSubject,
    },
    #[error("transaction binding does not match the registered binding")]
    TransactionBinding {
        expected: EvmTransactionBinding,
        observed: EvmTransactionBinding,
    },
    #[error("transaction authority epoch does not match")]
    AuthorityEpoch {
        expected: EvmAuthorityEpoch,
        observed: EvmAuthorityEpoch,
    },
    #[error("transaction reservation is missing")]
    MissingReservation { effect_id: EffectId },
    #[error("retained reservation does not match the command")]
    RetainedReservation {
        expected: Reservation,
        observed: Reservation,
    },
    #[error("reservation identity does not match the requested authority")]
    ReservationBinding {
        expected_effect: EffectId,
        expected_command: ContentRef,
        expected_domain: NonceDomain,
        observed: Reservation,
    },
    #[error("prepared transaction is missing")]
    MissingPrepared { reservation: Reservation },
    #[error("prepared transaction hash does not match")]
    PreparedHash {
        expected: EvmHash,
        observed: EvmHash,
    },
    #[error("recovered signer does not match the command sender")]
    RecoveredSender {
        expected: EvmAddress,
        observed: EvmAddress,
    },
    #[error("receipt identity does not match the prepared transaction")]
    ReceiptIdentity {
        expected_hash: EvmHash,
        observed_hash: EvmHash,
        expected_sender: EvmAddress,
        observed_sender: EvmAddress,
    },
    #[error("receipt outcome does not match the command action")]
    ReceiptShape {
        expected_target: Option<EvmAddress>,
        observed: ProviderReceiptResult,
    },
    #[error("created address does not match the command sender and nonce")]
    CreatedAddress {
        expected: EvmAddress,
        observed: EvmAddress,
    },
    #[error("provider chain instance does not match the command")]
    ChainInstance {
        expected: EvmChainInstance,
        observed: EvmChainInstance,
    },
    #[error("token call has no token address")]
    MissingToken { balance_target: EvmBalanceTarget },
    #[error("anchored result construction failed")]
    AnchoredResult {
        anchor: EvmBlockAnchor,
        return_bytes: usize,
        #[source]
        source: EvmDomainError,
    },
    #[error("prepared transaction qualification failed")]
    QualifyPrepared(#[source] EvmCodecError),
    #[error("transaction signing digest construction failed")]
    SigningDigest(#[source] EvmCodecError),
    #[error("signed transaction encoding failed")]
    EncodeSignedTransaction(#[source] EvmCodecError),
    #[error("signer public key recovery failed")]
    RecoverPublicKey(#[source] mfm_signing::SigningError),
    #[error("adapter pure task failed")]
    Task {
        operation: TaskOperation,
        outcome: &'static str,
        panic_payload: &'static str,
    },
    #[error("block tag parsing failed")]
    BlockTag {
        #[source]
        #[serde(serialize_with = "parse_error")]
        source: ParseError,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskOperation {
    QualifyPrepared,
    SigningDigest,
    EncodeSignedTransaction,
}

impl AdapterFailure {
    pub(crate) fn task(operation: TaskOperation, error: tokio::task::JoinError) -> Self {
        // The SDK's panic payload can contain signer custody. Extract only reviewed task facts;
        // withholding is explicit and no formatted payload or raw JoinError crosses this boundary.
        let panicked = error.is_panic();
        Self::Task {
            operation,
            outcome: if panicked { "panicked" } else { "cancelled" },
            panic_payload: if panicked { "withheld" } else { "not_present" },
        }
    }
}

pub(crate) fn invariant<E, O>(source: E) -> mfm_capabilities::AdapterError<O>
where
    E: Serialize,
{
    mfm_capabilities::AdapterError::Invariant(mfm_values::InvocationDiagnostic::from_fields(
        "adapter_invariant",
        "invariant",
        &source,
        None,
    ))
}

fn parse_error<S: serde::Serializer>(
    source: &ParseError,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "snake_case")]
    enum Conversion {
        Overflow,
        InvalidBase { base: u64 },
        InvalidDigit { digit: u64, base: u64 },
    }
    #[derive(Serialize)]
    #[serde(rename_all = "snake_case")]
    enum Parse {
        InvalidDigit { character: char },
        InvalidRadix { radix: u64 },
        BaseConvert { source: Conversion },
    }
    // The pinned parser retains only these facts, never the rejected decimal string.
    let projected = match source {
        ParseError::InvalidDigit(character) => Parse::InvalidDigit {
            character: *character,
        },
        ParseError::InvalidRadix(radix) => Parse::InvalidRadix { radix: *radix },
        ParseError::BaseConvertError(source) => Parse::BaseConvert {
            source: match source {
                BaseConvertError::Overflow => Conversion::Overflow,
                BaseConvertError::InvalidBase(base) => Conversion::InvalidBase { base: *base },
                BaseConvertError::InvalidDigit(digit, base) => Conversion::InvalidDigit {
                    digit: *digit,
                    base: *base,
                },
            },
        },
    };
    projected.serialize(serializer)
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
