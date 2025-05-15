// modules
pub mod error;
#[cfg(test)]
mod tests;

// keystore impl
use error::KeystoreError;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use alloy_primitives::{Address, B256 as H256}; // Removed U256
use alloy_signer::Signature;
use argon2::{self, Argon2}; // Import argon2 module for Params
use atomicwrites::{AtomicFile, OverwriteBehavior};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _}; // For base64 encoding
use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic; // Ensure Seed is not imported from bip39
use chrono::{DateTime, Utc};
use dirs_next;
use hex; // For encoding salt
use k256::{ecdsa::SigningKey, SecretKey}; // Removed PublicKey
use rand_core::{CryptoRng, OsRng, RngCore}; // Added OsRng
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write; // Added for f.write_all
use std::path::PathBuf;
use std::str::FromStr; // For DerivationPath::from_str
use std::time::Instant;
use tiny_keccak::{Hasher, Keccak}; // For Keccak-256
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing}; // Added Zeroizing struct

// --- Structs for Keystore Data ---

#[derive(Serialize, Deserialize, Debug, Clone, Zeroize, ZeroizeOnDrop)] // Zeroize for salt is fine
pub struct MasterKdfParams {
    pub salt: String, // hex_encoded_salt
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub output_len: usize,
}

// Uuid and DateTime<Utc> do not implement Zeroize/ZeroizeOnDrop by default.
// String fields (encrypted_pk, nonce) do. Alias is Option<String>.
// Address is a fixed-size array, which should be fine.
// Removing ZeroizeOnDrop from EncryptedKeyEntry derive.
// String fields within will handle their own zeroization as String implements ZeroizeOnDrop.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedKeyEntry {
    pub id: Uuid,                  // Not secret
    pub alias: Option<String>,     // String part will be zeroized on drop if Some
    pub address: Address,          // Not secret
    pub encrypted_pk: String, // base64 encoded encrypted private key - String implements ZeroizeOnDrop
    pub nonce: String,        // base64 encoded nonce for AES-GCM - String implements ZeroizeOnDrop
    pub created_at: DateTime<Utc>, // Not secret
    pub updated_at: DateTime<Utc>, // Not secret
                              // Potentially other metadata like derivation path if applicable, key type, etc.
}

// KeystoreFile's ZeroizeOnDrop will apply to its String fields.
// MasterKdfParams also derives ZeroizeOnDrop for its salt.
// Vec<EncryptedKeyEntry> elements (Strings) will be zeroized when they are dropped.
#[derive(Serialize, Deserialize, Debug, ZeroizeOnDrop)]
struct KeystoreFile {
    version: String,    // String implements ZeroizeOnDrop
    master_kdf: String, // String implements ZeroizeOnDrop
    master_kdf_params: MasterKdfParams,
    #[zeroize(skip)] // Skip entries vec, as EncryptedKeyEntry doesn't derive ZeroizeOnDrop itself.
    // Sensitive String fields within EncryptedKeyEntry will self-zeroize.
    entries: Vec<EncryptedKeyEntry>,
}

// Information about a key, returned by list_keys
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyInfo {
    pub id: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// --- Keystore Struct ---

#[derive(Debug)] // Removed ZeroizeOnDrop from Keystore struct itself
pub struct Keystore {
    file_path: PathBuf, // PathBuf does not need to be zeroized
    // master_key is Option<Zeroizing<Vec<u8>>> which handles its own zeroization. No attribute needed.
    master_key: Option<zeroize::Zeroizing<Vec<u8>>>,
    entries: Vec<EncryptedKeyEntry>,
    master_kdf_params: Option<MasterKdfParams>,
    is_unlocked: bool,
    // last_activity_at does not need zeroization. No attribute needed.
    last_activity_at: Option<Instant>,
}

// --- Keystore Implementation ---

impl Keystore {
    const DEFAULT_KEYSTORE_FILENAME: &'static str = "keystore_v1.json";
    const APP_DIR_NAME: &'static str = "mfm";

    pub fn new(custom_path: Option<PathBuf>) -> Result<Self, KeystoreError> {
        let file_path = match custom_path {
            Some(path) => path,
            None => {
                let data_dir = dirs_next::data_dir().ok_or_else(|| {
                    KeystoreError::FsError("Could not determine system data directory".to_string())
                })?;
                let app_data_dir = data_dir.join(Self::APP_DIR_NAME);
                if !app_data_dir.exists() {
                    fs::create_dir_all(&app_data_dir).map_err(KeystoreError::Io)?;
                }
                app_data_dir.join(Self::DEFAULT_KEYSTORE_FILENAME)
            }
        };

        Ok(Self {
            file_path,
            master_key: None,
            entries: Vec::new(),
            master_kdf_params: None,
            is_unlocked: false,
            last_activity_at: None,
        })
    }

    pub fn initialize_or_load(&mut self, password: Option<&str>) -> Result<(), KeystoreError> {
        self.load_from_disk()?;

        if self.master_kdf_params.is_none() {
            // Keystore file doesn't exist or is uninitialized
            if let Some(p) = password {
                if p.is_empty() {
                    return Err(KeystoreError::InvalidPassword); // Or a specific error for empty password on init
                }
                // New keystore, and password provided: initialize KDF params
                let kdf_params = Self::generate_kdf_params(&mut OsRng)?;
                self.master_kdf_params = Some(kdf_params);
                // Save the new keystore structure with KDF params (but no entries yet)
                self.save_to_disk()?;
            }
            // If no password provided for a new keystore, it remains uninitialized.
            // It cannot be used until a password is set and KDF params are created.
        }
        // If master_kdf_params is Some, it means an existing keystore was loaded.
        // It remains locked. Unlocking is a separate step.
        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        if self.is_unlocked {
            // Optionally, update last_activity_at or just return Ok
            self.last_activity_at = Some(Instant::now());
            return Ok(());
        }

        if password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        let kdf_params = self
            .master_kdf_params
            .as_ref()
            .ok_or(KeystoreError::FsError(
                "Keystore is not initialized with KDF parameters.".to_string(),
            ))?;

        let derived_key = self.derive_master_key(password, kdf_params)?;

        self.master_key = Some(zeroize::Zeroizing::new(derived_key));
        self.is_unlocked = true;
        self.last_activity_at = Some(Instant::now());

        Ok(())
    }

    pub fn lock(&mut self) {
        self.master_key = None; // This will zeroize the key due to Zeroizing wrapper
        self.is_unlocked = false;
        self.last_activity_at = Some(Instant::now()); // Record lock time as last activity
    }

    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        phrase: &str,
        passphrase: Option<&str>,
        path_str: &str, // e.g., "m/44'/60'/0'/0/0"
    ) -> Result<(Uuid, Address), KeystoreError> {
        if !self.is_unlocked || self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }
        if phrase.is_empty() {
            // Or a more specific error like InvalidMnemonic
            return Err(KeystoreError::InvalidPrivateKey);
        }

        let mnemonic = Mnemonic::parse(phrase)?;
        // Get seed bytes directly from mnemonic using to_seed()
        let seed_bytes_array = mnemonic.to_seed(passphrase.unwrap_or("")); // Returns [u8; 64]

        let derivation_path = DerivationPath::from_str(path_str).map_err(|e| {
            KeystoreError::InvalidPath(format!("Failed to parse derivation path: {}", e))
        })?;

        // Derive the private key using BIP32
        // XPrv::derive_from_path takes AsRef<[u8]> for seed
        let xprv = XPrv::derive_from_path(seed_bytes_array.as_slice(), &derivation_path)
            .map_err(KeystoreError::Bip32)?;

        // xprv.private_key() returns a SigningKey from bip32's re-exported k256.
        // We need to get its bytes and reconstruct with our project's k256::SecretKey.
        let bip32_signing_key = xprv.private_key();
        let secret_bytes_from_bip32 = bip32_signing_key.to_bytes(); // This is GenericArray from bip32's k256

        let secret_key = SecretKey::from_slice(&secret_bytes_from_bip32)
            .map_err(|_| KeystoreError::DerivationFailed)?; // Convert to our k256::SecretKey

        let pk_bytes_for_encryption = secret_key.to_bytes(); // Get bytes from our k256::SecretKey

        // From here, similar to import_private_key_hex
        let signing_key = SigningKey::from(&secret_key); // Use our project's k256::SecretKey
        let public_key = signing_key.verifying_key();

        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]);
        let mut hashed_pk = [0u8; 32];
        keccak.finalize(&mut hashed_pk);
        let address_bytes: [u8; 20] = hashed_pk[12..]
            .try_into()
            .map_err(|_| KeystoreError::DerivationFailed)?;
        let address = Address::from(address_bytes);

        if let Some(ref new_alias) = alias {
            if self
                .entries
                .iter()
                .any(|e| e.alias.as_ref() == Some(new_alias))
            {
                return Err(KeystoreError::AliasExists(new_alias.clone()));
            }
        }

        let id = Uuid::new_v4();
        let mut nonce_bytes = [0u8; 12];
        OsRng
            .try_fill_bytes(&mut nonce_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate nonce: {}", e)))?;

        // pk_bytes_for_encryption is GenericArray, convert to slice for encrypt_pk
        let encrypted_pk_vec = self.encrypt_pk(pk_bytes_for_encryption.as_slice(), &nonce_bytes)?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            nonce: BASE64_STANDARD.encode(nonce_bytes),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.entries.push(entry);
        self.save_to_disk()?;
        self.last_activity_at = Some(Instant::now());

        Ok((id, address))
    }

    pub fn import_private_key_hex(
        &mut self,
        alias: Option<String>,
        pk_hex: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        if !self.is_unlocked || self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }
        if pk_hex.is_empty() {
            return Err(KeystoreError::InvalidPrivateKey);
        }

        let pk_bytes = hex::decode(pk_hex)?;
        if pk_bytes.len() != 32 {
            // Secp256k1 private keys are 32 bytes
            return Err(KeystoreError::InvalidPrivateKey);
        }

        // Validate and derive public key + address
        let secret_key =
            SecretKey::from_slice(&pk_bytes).map_err(|_| KeystoreError::InvalidPrivateKey)?; // k256 error is not std::Error
        let signing_key = SigningKey::from(secret_key); // This is infallible if SecretKey is valid
        let public_key = signing_key.verifying_key();

        // Derive EVM address from public key
        let uncompressed_pk = public_key.to_encoded_point(false); // false for uncompressed
                                                                  // uncompressed_pk[0] is 0x04. We need to hash uncompressed_pk[1..]
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]); // Skip the 0x04 prefix
        let mut hashed_pk = [0u8; 32];
        keccak.finalize(&mut hashed_pk);
        let address_bytes: [u8; 20] = hashed_pk[12..].try_into().map_err(|_| {
            KeystoreError::DerivationFailed // Should not happen if logic is correct
        })?;
        let address = Address::from(address_bytes);

        // Check for alias conflict
        if let Some(ref new_alias) = alias {
            if self
                .entries
                .iter()
                .any(|e| e.alias.as_ref() == Some(new_alias))
            {
                return Err(KeystoreError::AliasExists(new_alias.clone()));
            }
        }

        let id = Uuid::new_v4();
        let mut nonce_bytes = [0u8; 12];
        OsRng
            .try_fill_bytes(&mut nonce_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate nonce: {}", e)))?;

        let encrypted_pk_vec = self.encrypt_pk(&pk_bytes, &nonce_bytes)?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            nonce: BASE64_STANDARD.encode(nonce_bytes),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.entries.push(entry);
        self.save_to_disk()?;
        self.last_activity_at = Some(Instant::now());

        Ok((id, address))
    }

    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError> {
        // Listing keys does not require the keystore to be unlocked as it only exposes metadata.
        let key_infos = self
            .entries
            .iter()
            .map(|entry| KeyInfo {
                id: entry.id,
                alias: entry.alias.clone(),
                address: entry.address,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
            })
            .collect();
        Ok(key_infos)
    }

    // Note: The plan mentions PrivateKeySigner, which comes from alloy-signer-local.
    // We'll need to ensure this is correctly typed and handled.
    // For now, returning a k256::SigningKey which can be wrapped or converted.
    pub fn get_signer(&mut self, uuid: Uuid) -> Result<SigningKey, KeystoreError> {
        if !self.is_unlocked || self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }

        let entry = self
            .entries
            .iter()
            .find(|e| e.id == uuid)
            .ok_or(KeystoreError::KeyNotFound(uuid))?;

        let encrypted_pk_bytes = BASE64_STANDARD
            .decode(&entry.encrypted_pk)
            .map_err(|_e| KeystoreError::InvalidFormat)?; // Underscore e

        let nonce_vec = BASE64_STANDARD
            .decode(&entry.nonce)
            .map_err(|_e| KeystoreError::InvalidFormat)?; // Underscore e

        let nonce_bytes: [u8; 12] = nonce_vec.try_into().map_err(|_| {
            KeystoreError::InvalidFormat // Or specific error for nonce length
        })?;

        let decrypted_pk_zeroizing_vec = self.decrypt_pk(&encrypted_pk_bytes, &nonce_bytes)?;

        let secret_key = SecretKey::from_slice(decrypted_pk_zeroizing_vec.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?; // Should be valid if encryption/decryption worked

        let signing_key = SigningKey::from(secret_key);

        self.last_activity_at = Some(Instant::now());
        Ok(signing_key)
    }

    pub fn delete_key(&mut self, uuid: Uuid) -> Result<(), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        let initial_len = self.entries.len();
        self.entries.retain(|entry| entry.id != uuid);

        if self.entries.len() == initial_len {
            return Err(KeystoreError::KeyNotFound(uuid));
        }

        self.save_to_disk()?;
        self.last_activity_at = Some(Instant::now());
        Ok(())
    }

    pub fn verify_signature(
        &self,
        uuid: Uuid,
        message_hash: H256,   // This is typically [u8; 32]
        signature: Signature, // alloy_signer::Signature
    ) -> Result<bool, KeystoreError> {
        if !self.is_unlocked || self.master_key.is_none() {
            // Verification needs the public key, which means decrypting the private key first
            // to ensure we are verifying against the correct key.
            // Alternatively, if addresses/public keys were stored unencrypted,
            // we could verify without unlocking. But current plan encrypts PK.
            return Err(KeystoreError::Locked);
        }

        let entry = self
            .entries
            .iter()
            .find(|e| e.id == uuid)
            .ok_or(KeystoreError::KeyNotFound(uuid))?;

        let encrypted_pk_bytes = BASE64_STANDARD
            .decode(&entry.encrypted_pk)
            .map_err(|_e| KeystoreError::InvalidFormat)?;

        let nonce_vec = BASE64_STANDARD
            .decode(&entry.nonce)
            .map_err(|_e| KeystoreError::InvalidFormat)?;

        let nonce_bytes: [u8; 12] = nonce_vec
            .try_into()
            .map_err(|_| KeystoreError::InvalidFormat)?;

        let decrypted_pk_zeroizing_vec = self.decrypt_pk(&encrypted_pk_bytes, &nonce_bytes)?;

        let secret_key = SecretKey::from_slice(decrypted_pk_zeroizing_vec.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;

        let signing_key = SigningKey::from(secret_key); // Use owned secret_key
        let verifying_key = signing_key.verifying_key();

        // Convert alloy_signer::Signature to k256::ecdsa::Signature
        // r and s are U256, k256::ecdsa::Signature::from_scalars needs [u8; 32]
        // Use accessor methods r() and s()
        let r_bytes: [u8; 32] = signature.r().to_be_bytes();
        let s_bytes: [u8; 32] = signature.s().to_be_bytes();

        let k256_signature = k256::ecdsa::Signature::from_scalars(r_bytes, s_bytes)
            .map_err(|_e| KeystoreError::SignatureVerificationFailed)?;

        // H256 is B256, which is [u8; 32], suitable for prehash
        use k256::ecdsa::signature::hazmat::PrehashVerifier; // Import the PrehashVerifier trait
        match verifying_key.verify_prehash(&message_hash.0, &k256_signature) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false), // Verification failed, but not an operational error
        }
    }

    // --- Helper methods (private) ---

    fn save_to_disk(&self) -> Result<(), KeystoreError> {
        if self.master_kdf_params.is_none() {
            // This case should ideally be handled before calling save_to_disk,
            // e.g., during initialization if no password is set yet.
            // For now, let's prevent saving an uninitialized keystore structure.
            return Err(KeystoreError::FsError(
                "Cannot save keystore without master KDF parameters.".to_string(),
            ));
        }

        let keystore_file_content = KeystoreFile {
            version: "1.0.0".to_string(),
            master_kdf: "argon2id".to_string(),
            master_kdf_params: self.master_kdf_params.clone().ok_or_else(|| {
                KeystoreError::FsError(
                    "Master KDF parameters are unexpectedly missing.".to_string(),
                )
            })?,
            entries: self.entries.clone(),
        };

        let json_data = serde_json::to_string_pretty(&keystore_file_content)?;

        let af = AtomicFile::new(&self.file_path, OverwriteBehavior::AllowOverwrite);
        af.write(|f| f.write_all(json_data.as_bytes()))
            .map_err(|e| KeystoreError::FsError(format!("Failed to write keystore file: {}", e)))?;

        Ok(())
    }

    fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        if !self.file_path.exists() {
            // If the file doesn't exist, it might be a fresh keystore.
            // The initialize_or_load function will handle creating a new one if needed.
            // For load_from_disk, we assume it should exist if called.
            // Or, we can return Ok and let initialize_or_load decide.
            // For now, let's indicate it's not an error state for load_from_disk itself,
            // but that no data was loaded. The caller can then decide.
            // A better approach might be for initialize_or_load to check existence first.
            // Let's make it return Ok if not found, and initialize_or_load will create new.
            return Ok(());
        }

        let file_content = fs::read_to_string(&self.file_path)?;
        let keystore_data: KeystoreFile = serde_json::from_str(&file_content)?;

        // Validate version and KDF
        if keystore_data.version != "1.0.0" {
            return Err(KeystoreError::InvalidFormat);
        }
        if keystore_data.master_kdf != "argon2id" {
            // Clone master_kdf because KeystoreFile implements Drop
            return Err(KeystoreError::UnsupportedKdf(
                keystore_data.master_kdf.clone(),
            ));
        }

        // Clone fields because KeystoreFile implements Drop
        self.entries = keystore_data.entries.clone();
        self.master_kdf_params = Some(keystore_data.master_kdf_params.clone());
        // Keystore remains locked after loading. Unlock is a separate step.
        self.is_unlocked = false;
        self.master_key = None;

        Ok(())
    }

    // fn encrypt_pk(&self, pk: &[u8], nonce: &[u8]) -> Result<Vec<u8>, KeystoreError> {
    fn encrypt_pk(
        &self,
        pk_bytes: &[u8],
        nonce_bytes: &[u8; 12],
    ) -> Result<Vec<u8>, KeystoreError> {
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_slice();
        let key = Key::<Aes256Gcm>::from_slice(master_key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher
            .encrypt(nonce, pk_bytes)
            .map_err(|e| KeystoreError::AesGcm(format!("Encryption failed: {}", e)))
    }

    fn decrypt_pk(
        &self,
        encrypted_pk_bytes: &[u8],
        nonce_bytes: &[u8; 12],
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_slice();
        let key = Key::<Aes256Gcm>::from_slice(master_key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

        let decrypted_bytes = cipher.decrypt(nonce, encrypted_pk_bytes).map_err(|e| {
            KeystoreError::AesGcm(format!(
                "Decryption failed (possibly invalid password or corrupted data): {}",
                e
            ))
        })?;

        Ok(Zeroizing::new(decrypted_bytes))
    }

    // fn derive_master_key(&self, password: &str, params: &MasterKdfParams) -> Result<Vec<u8>, KeystoreError> {
    fn derive_master_key(
        &self,
        password: &str,
        kdf_params: &MasterKdfParams,
    ) -> Result<Vec<u8>, KeystoreError> {
        let salt = hex::decode(&kdf_params.salt)
            .map_err(|e| KeystoreError::Argon2Error(format!("Failed to decode salt: {}", e)))?; // Re-using Argon2Error for this, or could make a new variant

        let params = argon2::Params::new(
            kdf_params.m_cost,
            kdf_params.t_cost,
            kdf_params.p_cost,
            Some(kdf_params.output_len),
        )
        .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?; // Ensure argon2::Error is converted

        let argon2_context = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13, // Argon2 version 1.3
            params,
        );

        let mut output_key_material = vec![0u8; kdf_params.output_len];
        argon2_context
            .hash_password_into(password.as_bytes(), &salt, &mut output_key_material)
            .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?;

        Ok(output_key_material)
    }

    fn generate_kdf_params(
        rng: &mut (impl CryptoRng + RngCore),
    ) -> Result<MasterKdfParams, KeystoreError> {
        let mut salt_bytes = [0u8; 16]; // 16-byte salt
        rng.try_fill_bytes(&mut salt_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate salt: {}", e)))?;

        Ok(MasterKdfParams {
            salt: hex::encode(salt_bytes),
            m_cost: 65536, // Argon2id default m_cost from argon2 crate is 4096. Plan specifies 65536.
            t_cost: 3,     // Plan specifies 3. Argon2id default t_cost is 3.
            p_cost: 1,     // Plan specifies 1. Argon2id default p_cost is 1.
            output_len: 32, // For AES-256 key
        })
    }
}

// Ensure the module is declared in mfm_core/src/lib.rs or mfm_core/src/keystore/mod.rs
