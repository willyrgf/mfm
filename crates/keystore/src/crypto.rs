//! Crate-private Ethereum key operations over protected borrowed material.

use alloy_primitives::{Address, PrimitiveSignature, B256};
use k256::ecdsa::SigningKey;
use k256::SecretKey;
use tiny_keccak::{Hasher, Keccak};

/// Error returned by Ethereum private-key helpers.
pub(crate) enum EthereumKeyError {
    /// The supplied bytes did not form a valid secp256k1 private key.
    InvalidPrivateKey,
    /// The signing primitive failed.
    SigningFailed,
}

impl std::fmt::Debug for EthereumKeyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("EthereumKeyError")
            .field(&match self {
                Self::InvalidPrivateKey => "invalid_private_key",
                Self::SigningFailed => "signing_failed",
            })
            .finish()
    }
}

impl std::fmt::Display for EthereumKeyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPrivateKey => "signing key did not form a valid secp256k1 key",
            Self::SigningFailed => "failed to sign prehashed payload",
        })
    }
}

impl std::error::Error for EthereumKeyError {}

fn secret_key(key_bytes: &[u8]) -> Result<SecretKey, EthereumKeyError> {
    SecretKey::from_slice(key_bytes).map_err(|_| EthereumKeyError::InvalidPrivateKey)
}

/// Derives the Ethereum address from protected key bytes without taking
/// ownership or copying them into an application-owned plaintext array.
pub(crate) fn address(key_bytes: &[u8]) -> Result<Address, EthereumKeyError> {
    let secret_key = secret_key(key_bytes)?;
    let public_key = secret_key.public_key();

    use k256::elliptic_curve::sec1::ToEncodedPoint;
    let uncompressed_pk = public_key.to_encoded_point(false);
    let mut keccak = Keccak::v256();
    keccak.update(&uncompressed_pk.as_bytes()[1..]);
    let mut hash = [0u8; 32];
    keccak.finalize(&mut hash);

    Ok(Address::from_slice(&hash[12..]))
}

/// Signs a 32-byte prehash with protected key bytes.
pub(crate) fn sign_hash_recoverable(
    key_bytes: &[u8],
    hash: &[u8; 32],
) -> Result<PrimitiveSignature, EthereumKeyError> {
    let secret_key = secret_key(key_bytes)?;
    let signing_key = SigningKey::from(&secret_key);
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(hash)
        .map_err(|_| EthereumKeyError::SigningFailed)?;
    let signature_bytes = signature.to_bytes();
    let mut r = [0u8; 32];
    r.copy_from_slice(&signature_bytes[..32]);
    let mut s = [0u8; 32];
    s.copy_from_slice(&signature_bytes[32..]);

    Ok(PrimitiveSignature::from_scalars_and_parity(
        B256::from(r),
        B256::from(s),
        recovery_id.is_y_odd(),
    )
    .normalized_s())
}
