// erc-2335 keystore primitives
// reference: https://eips.ethereum.org/eips/eip-2335
use bls12_381::{G1Projective, Scalar};
use hex::{FromHex, ToHex};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zeroize::Zeroize;

// constants for scrypt
const SCRYPT_LOG_N: u8 = 18; // n = 2^18
const SCRYPT_R: u32 = 8; // r = 8
const SCRYPT_P: u32 = 1; // p = 1
const SCRYPT_DKLEN: usize = 32; // derived key length
const SCRYPT_SALT_LEN: usize = 32; // salt length

// constants for aes-128-ctr
const AES_IV_LEN: usize = 16; // iv length for aes-128-ctr

/// Represents a BLS12-381 public key.
/// Serialized as a 0x-prefixed hex string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlsPublicKey(
    #[serde(
        serialize_with = "serialize_g1_projective_hex",
        deserialize_with = "deserialize_g1_projective_hex"
    )]
    pub G1Projective,
);

fn serialize_g1_projective_hex<S>(val: &G1Projective, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let bytes = val.to_affine().to_compressed();
    serializer.serialize_str(&format!("0x{}", hex::encode(bytes)))
}

fn deserialize_g1_projective_hex<'de, D>(deserializer: D) -> Result<G1Projective, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let s_no_prefix = s.strip_prefix("0x").unwrap_or(&s);
    let bytes = Vec::<u8>::from_hex(s_no_prefix)
        .map_err(|e| serde::de::Error::custom(format!("invalid hex for public key: {}", e)))?;
    let point = G1Projective::from_compressed(&bytes.try_into().map_err(|_| {
        serde::de::Error::custom(format!(
            "public key bytes have incorrect length: expected 48, got {}",
            bytes.len()
        ))
    })?);
    if point.is_none().into() {
        return Err(serde::de::Error::custom(
            "invalid bls12-381 public key bytes",
        ));
    }
    Ok(point.unwrap())
}

/// ERC-2335 Keystore File structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeystoreFile {
    pub crypto: Crypto,
    pub pubkey: BlsPublicKey,
    pub path: String, // derivation path, e.g., "m/12381/3600/0/0"
    pub uuid: String, // uuid v4 string
    pub version: u32, // keystore version, should be 4 for erc-2335
}

/// Crypto object within the Keystore file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crypto {
    pub kdf: String, // "scrypt" or "pbkdf2"
    pub kdfparams: KdfParams,
    pub cipher: String, // "aes-128-ctr"
    pub cipherparams: CipherParams,
    pub ciphertext: String, // hex encoded ciphertext
    pub mac: String,        // hex encoded mac
}

/// KDF parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)] // to handle both scrypt and pbkdf2 params if needed, though we focus on scrypt
pub enum KdfParams {
    Scrypt {
        dklen: usize,
        n: u32,
        r: u32,
        p: u32,
        salt: String, // hex encoded
    },
    // pbkdf2 can be added here if support is extended
}

/// Cipher parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipherParams {
    pub iv: String, // hex encoded
}

#[derive(Debug, Clone)]
pub struct KeystoreManager {
    keystore_dir: PathBuf,
}

impl KeystoreManager {
    pub fn new(path_str: &str) -> Result<Self, String> {
        let keystore_dir = PathBuf::from(path_str);
        if !keystore_dir.exists() {
            fs::create_dir_all(&keystore_dir).map_err(|e| {
                format!("failed to create keystore directory at {}: {}", path_str, e)
            })?;
        }
        if !keystore_dir.is_dir() {
            return Err(format!("keystore path {} is not a directory", path_str));
        }
        Ok(Self { keystore_dir })
    }

    // todo: implement key import AI:
    pub async fn import_key(
        &self,
        private_key_hex: &str,
        password: &str,
        // todo: AI: add derivation path as an argument, e.g. "m/12381/3600/0/0"
    ) -> Result<BlsPublicKey, String> {
        let pk_hex_no_prefix = private_key_hex
            .strip_prefix("0x")
            .unwrap_or(private_key_hex);
        let mut private_key_bytes = Vec::<u8>::from_hex(pk_hex_no_prefix)
            .map_err(|e| format!("invalid private key hex: {}", e))?;
        if private_key_bytes.len() != 32 {
            return Err(format!(
                "private key must be 32 bytes, got {}",
                private_key_bytes.len()
            ));
        }

        let secret_key = Scalar::from_bytes_wide(
            &private_key_bytes
                .try_into()
                .map_err(|_| "failed to convert private key bytes to array".to_string())?,
        );
        private_key_bytes.zeroize(); // zeroize sensitive bytes

        let public_key = BlsPublicKey(G1Projective::generator() * secret_key);

        // --- encryption ---
        let mut salt = [0u8; SCRYPT_SALT_LEN];
        OsRng
            .fill_bytes(&mut salt)
            .map_err(|e| format!("failed to generate salt: {}", e))?;

        let scrypt_params = scrypt::Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P, SCRYPT_DKLEN)
            .map_err(|e| format!("failed to create scrypt params: {}", e))?;

        let mut derived_key = [0u8; SCRYPT_DKLEN];
        scrypt::scrypt(password.as_bytes(), &salt, &scrypt_params, &mut derived_key)
            .map_err(|e| format!("scrypt key derivation failed: {}", e))?;

        let enc_key = &derived_key[..16]; // use first 16 bytes for aes-128 key
        let mut iv = [0u8; AES_IV_LEN];
        OsRng
            .fill_bytes(&mut iv)
            .map_err(|e| format!("failed to generate iv: {}", e))?;

        // encrypt the private key (original bytes before converting to Scalar)
        // re-decode for encryption as scalar conversion might alter it for bls12-381 specifics
        let mut pk_bytes_for_encryption = Vec::<u8>::from_hex(pk_hex_no_prefix)
            .map_err(|e| format!("failed to re-decode private key hex for encryption: {}", e))?;

        use aes::cipher::{KeyIvInit, StreamCipher};
        let mut cipher = aes::ctr::Ctr128BE::<aes::Aes128>::new(enc_key.into(), &iv.into());
        cipher.apply_keystream(&mut pk_bytes_for_encryption);
        let ciphertext_hex = hex::encode(pk_bytes_for_encryption);

        // calculate mac
        // mac = keccak256(derived_key[16..32] ++ ciphertext)
        let mut mac_data = Vec::new();
        mac_data.extend_from_slice(&derived_key[16..32]);
        mac_data.extend_from_slice(&hex::decode(&ciphertext_hex).unwrap()); // unwrap is safe here

        use sha3::{Digest, Keccak256};
        let mut hasher = Keccak256::new();
        hasher.update(&mac_data);
        let mac_bytes = hasher.finalize();
        let mac_hex = hex::encode(mac_bytes);

        let keystore_uuid = Uuid::new_v4().to_string();
        let keystore_file = KeystoreFile {
            crypto: Crypto {
                kdf: "scrypt".to_string(),
                kdfparams: KdfParams::Scrypt {
                    dklen: SCRYPT_DKLEN,
                    n: 2u32.pow(SCRYPT_LOG_N as u32),
                    r: SCRYPT_R,
                    p: SCRYPT_P,
                    salt: hex::encode(salt),
                },
                cipher: "aes-128-ctr".to_string(),
                cipherparams: CipherParams {
                    iv: hex::encode(iv),
                },
                ciphertext: ciphertext_hex,
                mac: mac_hex,
            },
            // todo: AI: use the derivation path argument here
            path: "m/12381/3600/0/0".to_string(), // placeholder path
            pubkey: public_key.clone(),
            uuid: keystore_uuid,
            version: 4,
        };

        let pubkey_hex = public_key.0.to_affine().to_compressed();
        let filename = format!(
            "UTC--{}--{}",
            chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                .replace(":", "-"),
            hex::encode(pubkey_hex)
        );
        let filepath = self.keystore_dir.join(filename);

        let mut file = File::create(&filepath).map_err(|e| {
            format!(
                "failed to create keystore file {}: {}",
                filepath.display(),
                e
            )
        })?;
        let json_content = serde_json::to_string_pretty(&keystore_file)
            .map_err(|e| format!("failed to serialize keystore data: {}", e))?;
        file.write_all(json_content.as_bytes()).map_err(|e| {
            format!(
                "failed to write to keystore file {}: {}",
                filepath.display(),
                e
            )
        })?;

        Ok(public_key)
    }

    // todo: implement key listing AI:
    pub async fn list_keys(&self) -> Result<Vec<BlsPublicKey>, String> {
        let mut pubkeys = Vec::new();
        for entry in fs::read_dir(&self.keystore_dir).map_err(|e| {
            format!(
                "failed to read keystore directory {}: {}",
                self.keystore_dir.display(),
                e
            )
        })? {
            let entry = entry.map_err(|e| format!("failed to read directory entry: {}", e))?;
            let path = entry.path();
            if path.is_file() {
                // basic check for filename pattern, more robust parsing would be better
                if path
                    .file_name()
                    .map_or(false, |name| name.to_string_lossy().starts_with("UTC--"))
                {
                    let mut file_content = String::new();
                    File::open(&path)
                        .map_err(|e| {
                            format!("failed to open keystore file {}: {}", path.display(), e)
                        })?
                        .read_to_string(&mut file_content)
                        .map_err(|e| {
                            format!("failed to read keystore file {}: {}", path.display(), e)
                        })?;

                    let keystore_data: KeystoreFile =
                        serde_json::from_str(&file_content).map_err(|e| {
                            format!("failed to parse keystore file {}: {}", path.display(), e)
                        })?;
                    pubkeys.push(keystore_data.pubkey);
                }
            }
        }
        Ok(pubkeys)
    }

    // todo: implement key deletion AI:
    // todo: AI: key decryption and MAC verification should happen before deletion to confirm password
    pub async fn delete_key(
        &self,
        pubkey_hex_to_delete: &str,
        _password: Option<&str>,
    ) -> Result<(), String> {
        let target_pubkey_bytes = Vec::<u8>::from_hex(
            pubkey_hex_to_delete
                .strip_prefix("0x")
                .unwrap_or(pubkey_hex_to_delete),
        )
        .map_err(|e| format!("invalid target public key hex for deletion: {}", e))?;

        let mut file_to_delete: Option<PathBuf> = None;

        for entry in fs::read_dir(&self.keystore_dir).map_err(|e| {
            format!(
                "failed to read keystore directory {}: {}",
                self.keystore_dir.display(),
                e
            )
        })? {
            let entry = entry.map_err(|e| format!("failed to read directory entry: {}", e))?;
            let path = entry.path();
            if path.is_file() {
                if path
                    .file_name()
                    .map_or(false, |name| name.to_string_lossy().starts_with("UTC--"))
                {
                    let mut file_content = String::new();
                    File::open(&path)
                        .map_err(|e| {
                            format!("failed to open keystore file {}: {}", path.display(), e)
                        })?
                        .read_to_string(&mut file_content)
                        .map_err(|e| {
                            format!("failed to read keystore file {}: {}", path.display(), e)
                        })?;

                    match serde_json::from_str::<KeystoreFile>(&file_content) {
                        Ok(keystore_data) => {
                            let current_pubkey_bytes =
                                keystore_data.pubkey.0.to_affine().to_compressed();
                            if current_pubkey_bytes.as_ref() == target_pubkey_bytes {
                                // todo: AI: if password is provided, attempt to decrypt and verify MAC before deletion
                                // for now, we just mark for deletion if pubkey matches
                                file_to_delete = Some(path.clone());
                                break;
                            }
                        }
                        Err(e) => {
                            // log or print warning about malformed file
                            eprintln!(
                                "warning: could not parse keystore file {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }

        if let Some(filepath) = file_to_delete {
            fs::remove_file(&filepath).map_err(|e| {
                format!(
                    "failed to delete keystore file {}: {}",
                    filepath.display(),
                    e
                )
            })?;
            Ok(())
        } else {
            Err(format!(
                "public key {} not found in keystore",
                pubkey_hex_to_delete
            ))
        }
    }

    // this method was not in the cli context, but it was in the original placeholder
    // it's not strictly needed for basic import/list/delete if we operate directly on files
    // but could be useful for loading a specific key by pubkey or uuid
    pub fn load_keystore_file_by_pubkey(&self, _pubkey_hex: &str) -> Result<KeystoreFile, String> {
        // todo: implement loading a specific keystore file by public key AI:
        Err("load_keystore_file_by_pubkey not yet implemented AI:".to_string())
    }
}

// helper for OsRng.fill_bytes
trait FillBytesRandom {
    fn fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error>;
}

impl<R: rand_core::RngCore + rand_core::CryptoRng> FillBytesRandom for R {
    fn fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.try_fill_bytes(dest)
    }
}
