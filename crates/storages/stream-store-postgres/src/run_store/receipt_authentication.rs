use super::*;
use ed25519_dalek::{Signer, SigningKey};
use std::fmt;
use zeroize::Zeroize;

/// In-memory signer for authenticated Postgres fact-query receipts.
#[derive(Clone)]
pub struct PostgresFactReceiptSigner {
    store_identity: mfm_facts::StoreIdentity,
    key_id: mfm_facts::StoreKeyId,
    signing_key: SigningKey,
}

impl PostgresFactReceiptSigner {
    /// Creates a fact receipt signer from secret Ed25519 signing key bytes.
    pub fn from_ed25519_signing_key_bytes(
        store_identity: mfm_facts::StoreIdentity,
        key_id: mfm_facts::StoreKeyId,
        mut signing_key: [u8; 32],
    ) -> Self {
        let signer = Self {
            store_identity,
            key_id,
            signing_key: SigningKey::from_bytes(&signing_key),
        };
        signing_key.zeroize();
        signer
    }

    /// Returns the non-secret store identity this signer asserts.
    pub const fn store_identity(&self) -> &mfm_facts::StoreIdentity {
        &self.store_identity
    }

    /// Returns the non-secret key id this signer asserts.
    pub const fn key_id(&self) -> &mfm_facts::StoreKeyId {
        &self.key_id
    }

    /// Returns the public Ed25519 verifying key bytes for this signer.
    pub fn verifying_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Returns the public trust root matching this signer.
    pub fn trust_root(&self) -> Result<mfm_store::v1::FactQueryReceiptTrustRoot> {
        mfm_store::v1::FactQueryReceiptTrustRoot::new(
            self.store_identity.clone(),
            mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            self.key_id.clone(),
            self.verifying_key(),
        )
        .map_err(PostgresStoreError::Store)
    }

    pub(super) fn sign_receipt_hash(
        &self,
        store_receipt_hash: &ContentDigest,
    ) -> Result<mfm_facts::StoreReceiptAuthentication> {
        let message = mfm_store::v1::fact_query_receipt_authentication_message(
            &self.store_identity,
            mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            &self.key_id,
            store_receipt_hash,
        )
        .map_err(PostgresStoreError::Store)?;
        let signature = self.signing_key.sign(message.as_bytes());
        mfm_facts::StoreReceiptAuthentication::new(
            self.store_identity.clone(),
            mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(self.key_id.clone()),
            signature.to_bytes().to_vec(),
        )
        .map_err(fact_error)
    }
}

impl fmt::Debug for PostgresFactReceiptSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresFactReceiptSigner")
            .field("store_identity", &self.store_identity)
            .field("key_id", &self.key_id)
            .field("verifying_key", &self.verifying_key())
            .finish_non_exhaustive()
    }
}

pub(super) fn require_fact_receipt_signer_matches_authority(
    authority: &PostgresStoreAuthority,
    signer: &PostgresFactReceiptSigner,
) -> Result<()> {
    let expected = authority
        .fact_receipt_trust_root()
        .ok_or_else(|| receipt_authentication_store_error("missing fact receipt trust root"))?;
    let actual = signer.trust_root()?;
    if &actual != expected {
        return Err(receipt_authentication_store_error(
            "fact receipt signer does not match store trust root",
        ));
    }
    Ok(())
}

pub(super) fn require_fact_receipt_signer(
    signer: Option<&PostgresFactReceiptSigner>,
) -> Result<&PostgresFactReceiptSigner> {
    signer.ok_or_else(|| receipt_authentication_store_error("missing fact receipt signer"))
}

pub(super) fn receipt_authentication_store_error(message: impl Into<String>) -> PostgresStoreError {
    StoreError::ReceiptAuthentication {
        message: message.into(),
    }
    .into()
}
