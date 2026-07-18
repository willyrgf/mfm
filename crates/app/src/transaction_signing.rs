use std::env;
use std::path::{Path, PathBuf};

use alloy_primitives::Address;
use mfm_evm_signing::{EvmSigningError, TransientSignedEip1559Envelope, UnsignedEip1559Envelope};
use mfm_runtime_config::{RuntimeConfig, RuntimeConfigErrorKind};
use mfm_signers_keystore::KeystoreSignerProvider;
use mfm_signing::{SignerRef, SigningError};

use crate::{ErrorClass, PublicError, MFM_RUNTIME_CONFIG_FILE};

/// Signs one checked EIP-1559 envelope through the exact runtime signer binding.
///
/// The explicit path takes precedence over [`MFM_RUNTIME_CONFIG_FILE`]. Runtime
/// configuration and keystore work stay below this application boundary; the
/// returned signed envelope remains transient bearer material.
pub async fn sign_eip1559_transaction(
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

    mfm_evm_signing::sign_eip1559(envelope, signer_ref, expected_sender, &provider)
        .await
        .map_err(public_signing_error)
}

fn assemble_keystore_signer(
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
    Ok(KeystoreSignerProvider::new(
        signer_ref,
        signer.entry_id(),
        keystore.keystore_path().expose_path(),
        keystore.unlock_file().expose_path(),
    ))
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

        let error = sign_eip1559_transaction(
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
}
