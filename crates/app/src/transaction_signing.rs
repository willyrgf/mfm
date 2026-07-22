use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use alloy_primitives::{Address, Bytes, TxKind, U256};
use mfm_evm::{EvmSigningError, TransientSignedEip1559Envelope, UnsignedEip1559Envelope};
use mfm_keystore::KeystoreSignerProvider;
use mfm_runtime_config::{RuntimeConfig, RuntimeConfigErrorKind};
use mfm_signing::{SignerRef, SigningError};
use serde::Serialize;

use crate::{ErrorClass, PublicError, MFM_RUNTIME_CONFIG_FILE};

/// Raw command input for one explicit EIP-1559 signing request.
#[derive(Debug, Clone)]
pub struct EvmTransactionSigningRequest {
    /// Explicit runtime configuration path, or `None` to use the environment binding.
    pub runtime_config_path: Option<PathBuf>,
    /// Exact process-local signer reference.
    pub signer_ref: String,
    /// Raw EIP-1559 envelope fields to parse canonically.
    pub envelope: EvmTransactionSigningEnvelopeInput,
}

/// Raw command fields used to construct one checked EIP-1559 envelope.
#[derive(Debug, Clone)]
pub struct EvmTransactionSigningEnvelopeInput {
    /// Expected canonical sender address.
    pub expected_from: String,
    /// Canonical destination address.
    pub to: String,
    /// Transaction value in canonical decimal or hexadecimal quantity form.
    pub value_wei: String,
    /// Chain id in canonical decimal or hexadecimal quantity form.
    pub chain_id: String,
    /// Sender nonce in canonical decimal or hexadecimal quantity form.
    pub nonce: String,
    /// Maximum fee per gas in canonical decimal or hexadecimal quantity form.
    pub max_fee_per_gas: String,
    /// Maximum priority fee per gas in canonical decimal or hexadecimal quantity form.
    pub max_priority_fee_per_gas: String,
    /// Gas limit in canonical decimal or hexadecimal quantity form.
    pub gas_limit: String,
    /// Canonical lower-case `0x`-prefixed calldata bytes.
    pub data: String,
}

/// Redacted public metadata for one explicitly signed EIP-1559 transaction.
#[derive(Debug, Clone, Serialize)]
pub struct EvmTransactionSigningMetadata {
    from: String,
    to: String,
    nonce: String,
    chain_id: String,
    signing_digest: String,
    transaction_hash: String,
}

impl fmt::Display for EvmTransactionSigningMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Signed EIP-1559 transaction {} from {} to {} and wrote bearer output locally",
            self.transaction_hash, self.from, self.to
        )
    }
}

/// Redacted signing metadata plus the transient bearer envelope.
pub struct SignedEvmTransaction {
    metadata: EvmTransactionSigningMetadata,
    bearer: TransientSignedEip1559Envelope,
}

impl SignedEvmTransaction {
    /// Separates redacted public metadata from transient bearer material.
    pub fn into_parts(
        self,
    ) -> (
        EvmTransactionSigningMetadata,
        TransientSignedEip1559Envelope,
    ) {
        (self.metadata, self.bearer)
    }
}

/// Parses, constructs, and signs one direct EIP-1559 command request.
///
/// Semantic parsing and envelope construction live at this non-binary boundary. The returned
/// bearer must be written only to the caller-selected protected local file.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), mfm_app::PublicError> {
/// use mfm_app::{
///     sign_evm_transaction_command, EvmTransactionSigningEnvelopeInput,
///     EvmTransactionSigningRequest,
/// };
///
/// let signed = sign_evm_transaction_command(EvmTransactionSigningRequest {
///     runtime_config_path: Some("runtime.toml".into()),
///     signer_ref: "treasury".to_owned(),
///     envelope: EvmTransactionSigningEnvelopeInput {
///         expected_from: "0x1111111111111111111111111111111111111111".to_owned(),
///         to: "0x2222222222222222222222222222222222222222".to_owned(),
///         value_wei: "0".to_owned(),
///         chain_id: "1".to_owned(),
///         nonce: "0".to_owned(),
///         max_fee_per_gas: "2000000000".to_owned(),
///         max_priority_fee_per_gas: "1000000000".to_owned(),
///         gas_limit: "21000".to_owned(),
///         data: "0x".to_owned(),
///     },
/// })
/// .await?;
/// let (_public_metadata, transient_bearer) = signed.into_parts();
/// // Write `transient_bearer.bytes()` only to the explicitly selected protected output.
/// # drop(transient_bearer);
/// # Ok(())
/// # }
/// ```
pub async fn sign_evm_transaction_command(
    request: EvmTransactionSigningRequest,
) -> Result<SignedEvmTransaction, PublicError> {
    let signer_ref = SignerRef::new(&request.signer_ref)
        .map_err(|_| PublicError::bad_request("invalid_signer_ref", "Signer ref is invalid"))?;
    let expected_from = parse_address(&request.envelope.expected_from, "from")?;
    let to = parse_address(&request.envelope.to, "to")?;
    let chain_id = parse_quantity(&request.envelope.chain_id, "chain-id")?;
    let nonce = parse_quantity(&request.envelope.nonce, "nonce")?;
    let max_priority_fee_per_gas = parse_quantity(
        &request.envelope.max_priority_fee_per_gas,
        "max-priority-fee-per-gas",
    )?;
    let max_fee_per_gas = parse_quantity(&request.envelope.max_fee_per_gas, "max-fee-per-gas")?;
    let gas_limit = parse_quantity(&request.envelope.gas_limit, "gas-limit")?;
    let value = parse_quantity(&request.envelope.value_wei, "value-wei")?;
    let data = parse_data(&request.envelope.data)?;
    let envelope = UnsignedEip1559Envelope::new(
        chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit,
        TxKind::Call(to),
        value,
        Default::default(),
        data,
    )
    .map_err(|_| {
        PublicError::bad_request(
            "invalid_eip1559_transaction",
            "EIP-1559 transaction inputs are invalid",
        )
    })?;
    let signing_digest = envelope.signing_digest();
    let bearer = sign_checked_eip1559_transaction(
        request.runtime_config_path.as_deref(),
        signer_ref,
        expected_from,
        &envelope,
    )
    .await?;
    let transaction_hash = bearer.transaction_hash();
    Ok(SignedEvmTransaction {
        metadata: EvmTransactionSigningMetadata {
            from: format!("{expected_from:?}"),
            to: format!("{to:?}"),
            nonce: nonce.to_string(),
            chain_id: chain_id.to_string(),
            signing_digest: format!("{signing_digest:?}"),
            transaction_hash: format!("{transaction_hash:?}"),
        },
        bearer,
    })
}

/// Signs one checked EIP-1559 envelope through the exact runtime signer binding.
///
/// The explicit path takes precedence over [`MFM_RUNTIME_CONFIG_FILE`]. Runtime
/// configuration and keystore work stay below this application boundary; the
/// returned signed envelope remains transient bearer material.
async fn sign_checked_eip1559_transaction(
    runtime_config_path: Option<&Path>,
    signer_ref: SignerRef,
    expected_sender: Address,
    envelope: &UnsignedEip1559Envelope,
) -> Result<TransientSignedEip1559Envelope, PublicError> {
    let runtime_config_path = runtime_config_path
        .map(Path::to_path_buf)
        .or_else(|| env::var_os(MFM_RUNTIME_CONFIG_FILE).map(PathBuf::from))
        .ok_or_else(|| {
            PublicError::bad_request(
                "RuntimeConfigRequired",
                "EVM signing requires --runtime-config or MFM_RUNTIME_CONFIG_FILE",
            )
        })?;
    let provider_signer_ref = signer_ref.clone();
    let provider = tokio::task::spawn_blocking(move || {
        assemble_keystore_signer(runtime_config_path, provider_signer_ref)
    })
    .await
    .map_err(|_| {
        PublicError::backend(
            ErrorClass::Internal,
            "SignerAssemblyWorkerFailed",
            "EVM signer assembly worker failed",
        )
    })??;

    mfm_evm::sign_eip1559(envelope, signer_ref, expected_sender, &provider)
        .await
        .map_err(public_signing_error)
}

fn parse_address(raw: &str, field: &'static str) -> Result<Address, PublicError> {
    let address = Address::from_str(raw)
        .map_err(|_| invalid_input(field, "must be a canonical 0x-prefixed 20-byte address"))?;
    if format!("{address:#x}") != raw {
        return Err(invalid_input(
            field,
            "must be a canonical 0x-prefixed 20-byte address",
        ));
    }
    Ok(address)
}

fn parse_quantity(raw: &str, field: &'static str) -> Result<U256, PublicError> {
    let quantity = U256::from_str(raw).map_err(|_| {
        invalid_input(
            field,
            "must be a canonical unsigned decimal or 0x-prefixed hexadecimal quantity",
        )
    })?;
    let canonical = if raw.starts_with("0x") {
        format!("{quantity:#x}")
    } else {
        quantity.to_string()
    };
    if canonical != raw {
        return Err(invalid_input(
            field,
            "must be a canonical unsigned decimal or 0x-prefixed hexadecimal quantity",
        ));
    }
    Ok(quantity)
}

fn parse_data(raw: &str) -> Result<Bytes, PublicError> {
    let Some(encoded) = raw.strip_prefix("0x") else {
        return Err(invalid_input(
            "data",
            "must be canonical 0x-prefixed hexadecimal bytes",
        ));
    };
    let bytes = alloy_primitives::hex::decode(encoded)
        .map_err(|_| invalid_input("data", "must be canonical 0x-prefixed hexadecimal bytes"))?;
    if format!("0x{}", alloy_primitives::hex::encode(&bytes)) != raw {
        return Err(invalid_input(
            "data",
            "must be canonical 0x-prefixed hexadecimal bytes",
        ));
    }
    Ok(Bytes::from(bytes))
}

fn invalid_input(field: &'static str, requirement: &'static str) -> PublicError {
    PublicError::bad_request(
        format!("invalid_{}", field.replace('-', "_")),
        format!("{field} {requirement}"),
    )
}

pub(crate) fn assemble_keystore_signer(
    runtime_config_path: PathBuf,
    signer_ref: SignerRef,
) -> Result<KeystoreSignerProvider, PublicError> {
    let binding =
        RuntimeConfig::load_signer_binding(runtime_config_path, &signer_ref).map_err(|error| {
            match error.kind() {
                RuntimeConfigErrorKind::MissingSigner => PublicError::bad_request(
                    "SignerNotConfigured",
                    "Requested signer is not configured",
                ),
                _ => PublicError::backend(
                    ErrorClass::ServiceUnavailable,
                    "RuntimeConfigInvalid",
                    "EVM signer runtime configuration is invalid",
                ),
            }
        })?;
    let signer = binding.signer();
    let keystore = binding.keystore();
    KeystoreSignerProvider::new(
        signer_ref,
        signer.entry_id(),
        keystore.keystore_path().expose_path(),
        keystore.unlock_file().expose_path(),
    )
    .map_err(|_| {
        PublicError::backend(
            ErrorClass::ServiceUnavailable,
            "SignerRuntimePathInvalid",
            "EVM signer runtime configuration is invalid",
        )
    })
}

fn public_signing_error(error: EvmSigningError) -> PublicError {
    match error {
        EvmSigningError::Signing(SigningError::Provider { .. }) => PublicError::backend(
            ErrorClass::ServiceUnavailable,
            "SignerUnavailable",
            "Configured signer could not sign the EVM transaction",
        ),
        EvmSigningError::Signing(SigningError::PublicIdentityMismatch { .. })
        | EvmSigningError::RecoveredAddressMismatch => PublicError::bad_request(
            "SignerIdentityMismatch",
            "Configured signer does not match the expected sender",
        ),
        EvmSigningError::QuantityOutOfRange { .. }
        | EvmSigningError::ZeroChainId
        | EvmSigningError::PriorityFeeExceedsMaxFee => PublicError::bad_request(
            "InvalidEip1559Transaction",
            "EIP-1559 transaction inputs are invalid",
        ),
        EvmSigningError::Signing(_)
        | EvmSigningError::DeterministicProfileMismatch
        | EvmSigningError::SigningResultMismatch { .. }
        | EvmSigningError::InvalidSignature { .. }
        | EvmSigningError::SignedHashMismatch => PublicError::backend(
            ErrorClass::Internal,
            "SignerContractViolation",
            "EVM signer violated the canonical signing contract",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn assembly_errors_do_not_expose_runtime_config_paths() {
        let secret_path = PathBuf::from("/tmp/private/runtime-signers.toml");
        let envelope = UnsignedEip1559Envelope::new(
            alloy_primitives::U256::from(1),
            alloy_primitives::U256::ZERO,
            alloy_primitives::U256::from(1),
            alloy_primitives::U256::from(2),
            alloy_primitives::U256::from(21_000),
            alloy_primitives::TxKind::Create,
            alloy_primitives::U256::ZERO,
            Default::default(),
            alloy_primitives::Bytes::new(),
        )
        .expect("envelope");

        let error = sign_checked_eip1559_transaction(
            Some(&secret_path),
            SignerRef::new("deployer").expect("signer"),
            Address::ZERO,
            &envelope,
        )
        .await
        .expect_err("missing config");

        assert_eq!(error.code, "RuntimeConfigInvalid");
        assert!(!format!("{error:?} {error}").contains("runtime-signers.toml"));
    }

    #[test]
    fn command_parsers_accept_only_canonical_forms() {
        assert_eq!(
            parse_quantity("42", "nonce").expect("decimal"),
            U256::from(42)
        );
        assert_eq!(
            parse_quantity("0x2a", "nonce").expect("hex"),
            U256::from(42)
        );
        for invalid in ["", "00", "0x", "0x02a", "0X2a", "0x2A", "2_a", "-1", " 1"] {
            assert!(parse_quantity(invalid, "nonce").is_err(), "{invalid}");
        }

        assert!(parse_address("0x1111111111111111111111111111111111111111", "to").is_ok());
        assert!(parse_address("1111111111111111111111111111111111111111", "to").is_err());
        assert_eq!(parse_data("0x").expect("empty data"), Bytes::new());
        assert!(parse_data("0x0").is_err());
        assert!(parse_data("0xAA").is_err());
        assert!(parse_data("00").is_err());
    }
}
