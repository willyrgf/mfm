use std::str::FromStr;
use zeroize::Zeroizing;

use super::secure_key::ethereum_address_from_key_bytes;
use super::*;
use crate::signer::ReadAttestationKeyAccess;

impl Keystore {
    /// Import private key (hex format).
    pub fn import_private_key(
        &mut self,
        alias: Option<String>,
        private_key_hex: &str,
    ) -> Result<Uuid, KeystoreError> {
        let id = Uuid::new_v4();
        self.ensure_master_key_available()?;

        let private_key_hex = private_key_hex.trim_start_matches("0x");
        if private_key_hex.len() != 64 {
            return Err(KeystoreError::InvalidPrivateKey);
        }

        let mut key_array = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(private_key_hex, key_array.as_mut())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;
        self.import_key_material(
            id,
            alias,
            key_array,
            KeyType::PrivateKey,
            AuditEvent::ImportPrivateKey { id },
        )
    }

    /// Import mnemonic with derivation path.
    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        mnemonic: &str,
        derivation_path: &str,
        passphrase: Option<&str>,
    ) -> Result<Uuid, KeystoreError> {
        let id = Uuid::new_v4();
        self.ensure_master_key_available()?;

        let mnemonic = Mnemonic::from_str(mnemonic)?;
        let derivation_path_obj = DerivationPath::from_str(derivation_path)?;

        // The mnemonic and passphrase are import inputs, not persisted wallet material.
        let seed = Zeroizing::new(mnemonic.to_seed(passphrase.unwrap_or("")));
        let derived_key: Zeroizing<[u8; 32]> = {
            let derived_xprv = XPrv::derive_from_path(*seed, &derivation_path_obj)?;
            Zeroizing::new(derived_xprv.private_key().to_bytes().into())
        };

        self.import_key_material(
            id,
            alias,
            derived_key,
            KeyType::HdDerived {
                derivation_path: derivation_path.to_string(),
            },
            AuditEvent::ImportMnemonic { id },
        )
    }

    fn import_key_material(
        &mut self,
        id: Uuid,
        alias: Option<String>,
        key_bytes: Zeroizing<[u8; 32]>,
        key_type: KeyType,
        audit_event: AuditEvent,
    ) -> Result<Uuid, KeystoreError> {
        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;
        let address = ethereum_address_from_key_bytes(key_bytes.as_ref())?;

        let mut nonce = [0u8; 12];
        OsRng.try_fill_bytes(&mut nonce).map_err(|_| {
            KeystoreError::CryptoError("Failed to generate random nonce".to_string())
        })?;

        let encrypted_data =
            self.encrypt_data(master_key, &nonce, key_bytes.as_ref(), id.as_bytes())?;
        let entry = KeyEntry {
            id,
            alias,
            address,
            key_type,
            encrypted_data,
            nonce,
            created_at: Utc::now(),
        };

        let previous_entry_len = self.entries.len();
        self.entries.push(entry);
        if let Err(err) = self.record_successful_audit(audit_event) {
            self.entries.truncate(previous_entry_len);
            return Err(err);
        }

        Ok(id)
    }

    /// Decrypts one key for Read attestation without refreshing or persisting keystore state.
    ///
    /// Decrypted material moves as an owned zeroizing allocation into
    /// [`SecureKey`]; there is no plaintext `[u8; 32]` intermediate.
    pub(crate) fn private_key_for_read_attestation(
        &self,
        id: Uuid,
        _access: &ReadAttestationKeyAccess,
    ) -> Result<SecureKey, KeystoreError> {
        self.ensure_unlocked_for_read()?;
        Ok(self.decrypt_entry_key(id)?.into_secure_key())
    }

    #[cfg(test)]
    pub(super) fn private_key_for_test(&self, id: Uuid) -> Result<SecureKey, KeystoreError> {
        self.ensure_unlocked_for_read()?;
        Ok(self.decrypt_entry_key(id)?.into_secure_key())
    }

    /// Test-only decrypt path that witnesses protected-allocation ownership
    /// transfer into [`SecureKey`] and cleanup on drop.
    #[cfg(test)]
    pub(super) fn private_key_for_test_with_ownership_witness(
        &self,
        id: Uuid,
        witness: super::secure_key::KeyMaterialWitness,
    ) -> Result<SecureKey, KeystoreError> {
        self.ensure_unlocked_for_read()?;
        let material = self.decrypt_entry_key_with_witness(id, witness.clone())?;
        witness.record_transfer();
        Ok(material.into_secure_key())
    }

    /// List stored keys (metadata only). Requires an unlocked session.
    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError> {
        self.ensure_unlocked_for_read()?;
        Ok(self.entries.iter().map(KeyInfo::from).collect())
    }

    /// Remove key from keystore.
    pub fn delete_key(&mut self, id: Uuid) -> Result<(), KeystoreError> {
        let result: Result<(), KeystoreError> = (|| {
            self.ensure_master_key_available()?;

            let previous_entries = self.entries.clone();
            let initial_len = previous_entries.len();
            self.entries.retain(|entry| entry.id != id);

            if self.entries.len() == initial_len {
                return Err(KeystoreError::KeyNotFound(id));
            }

            if let Err(err) = self.record_successful_audit(AuditEvent::DeleteKey { id }) {
                self.entries = previous_entries;
                return Err(err);
            }
            Ok(())
        })();
        result
    }

    /// Decrypts one entry into protected key material.
    ///
    /// On every error path (locked, missing entry, AEAD failure, wrong length)
    /// any intermediate plaintext buffer is owned by a zeroizing container and
    /// is cleaned up when that container drops. Success transfers the same
    /// protected allocation into [`SecureKey`] without an ordinary array copy.
    fn decrypt_entry_key(
        &self,
        id: Uuid,
    ) -> Result<super::secure_key::ProtectedKeyMaterial, KeystoreError> {
        let decrypted = self.decrypt_entry_bytes(id)?;
        super::secure_key::ProtectedKeyMaterial::from_decrypted_exact(decrypted)
    }

    #[cfg(test)]
    fn decrypt_entry_key_with_witness(
        &self,
        id: Uuid,
        witness: super::secure_key::KeyMaterialWitness,
    ) -> Result<super::secure_key::ProtectedKeyMaterial, KeystoreError> {
        let decrypted = self.decrypt_entry_bytes(id)?;
        super::secure_key::ProtectedKeyMaterial::from_decrypted_exact_with_witness(
            decrypted, witness,
        )
    }

    fn decrypt_entry_bytes(&self, id: Uuid) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(KeystoreError::KeyNotFound(id))?;
        // AEAD failure returns before any key material is allocated; successful
        // plaintext lands only in the returned Zeroizing buffer.
        self.decrypt_data(
            master_key,
            &entry.nonce,
            &entry.encrypted_data,
            entry.id.as_bytes(),
        )
    }

    fn record_successful_audit(&mut self, event: AuditEvent) -> Result<(), KeystoreError> {
        let previous_audit_log = self.audit_log.clone();
        self.append_audit_event(event, true);
        if let Err(err) = self.save_to_disk() {
            self.audit_log = previous_audit_log;
            return Err(err);
        }
        Ok(())
    }
}
