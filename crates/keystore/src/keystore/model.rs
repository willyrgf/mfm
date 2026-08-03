use alloy_primitives::Address;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Simplified configuration with secure defaults.
#[derive(Debug, Clone)]
pub struct KeystoreConfig {
    /// Argon2 memory cost in KB (default: 1GB = 1048576).
    pub argon2_memory_kb: u32,
    /// Argon2 time cost in iterations (default: 8).
    pub argon2_iterations: u32,
    /// Argon2 parallelism (default: 1).
    pub argon2_parallelism: u32,
}

impl Default for KeystoreConfig {
    fn default() -> Self {
        Self {
            argon2_memory_kb: 1_048_576, // 1GB - production secure
            argon2_iterations: 8,        // 8 iterations - secure default
            argon2_parallelism: 1,       // Single threaded
        }
    }
}

impl KeystoreConfig {
    /// Production-grade secure configuration.
    pub fn production() -> Self {
        Self::default()
    }

    /// Development configuration (faster but less secure).
    #[cfg(test)]
    pub fn development() -> Self {
        Self {
            argon2_memory_kb: 8192, // 8MB for faster tests
            argon2_iterations: 2,   // 2 iterations
            argon2_parallelism: 1,
        }
    }

    /// Integration test configuration (very fast but insecure - DO NOT USE IN PRODUCTION).
    pub fn insecure_integration_test() -> Self {
        Self {
            argon2_memory_kb: 64, // 64KB - minimal for fast tests
            argon2_iterations: 1, // 1 iteration - minimal
            argon2_parallelism: 1,
        }
    }
}

/// Key type for different storage formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum KeyType {
    /// Raw secp256k1 private key material.
    PrivateKey,
    /// Private key derived once from BIP-39 input during import.
    HdDerived {
        /// Derivation path used for the one-time import derivation.
        derivation_path: String,
    },
}

impl KeyType {
    fn sanitized_for_output(&self) -> Self {
        self.clone()
    }
}

/// Key entry stored in keystore.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyEntry {
    /// Stable identifier for the stored key entry.
    pub id: Uuid,
    /// Optional human-readable alias.
    pub alias: Option<String>,
    /// Derived Ethereum address for the key material.
    pub address: Address,
    /// Stored key format.
    pub key_type: KeyType,
    /// Encrypted key payload bytes.
    pub encrypted_data: Vec<u8>,
    /// AES-GCM nonce used to encrypt `encrypted_data`.
    pub nonce: [u8; 12],
    /// Creation timestamp in UTC.
    pub created_at: DateTime<Utc>,
}

/// Key metadata for listing operations.
#[derive(Debug, Clone)]
pub struct KeyInfo {
    /// Stable identifier for the stored key entry.
    pub id: Uuid,
    /// Optional human-readable alias.
    pub alias: Option<String>,
    /// Derived Ethereum address for the key material.
    pub address: Address,
    /// Stored key format.
    pub key_type: KeyType,
    /// Creation timestamp in UTC.
    pub created_at: DateTime<Utc>,
}

impl From<&KeyEntry> for KeyInfo {
    fn from(entry: &KeyEntry) -> Self {
        Self {
            id: entry.id,
            alias: entry.alias.clone(),
            address: entry.address,
            key_type: entry.key_type.sanitized_for_output(),
            created_at: entry.created_at,
        }
    }
}

/// Audit events appended to the in-memory and persisted keystore audit log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum AuditEvent {
    /// An unlock attempt occurred.
    Unlock,
    /// The keystore was locked.
    Lock,
    /// A private key import was attempted.
    ImportPrivateKey {
        /// Identifier assigned to the imported entry.
        id: Uuid,
    },
    /// A mnemonic import was attempted.
    ImportMnemonic {
        /// Identifier assigned to the imported entry.
        id: Uuid,
    },
    // Retained only so authenticated v1 audit histories remain decodable and MAC-covered. The
    // current implementation has no producer for this historical record.
    #[allow(dead_code)]
    /// A historical v1 private-key retrieval was attempted for signing.
    GetPrivateKey {
        /// Identifier of the requested entry.
        id: Uuid,
    },
    /// A key deletion was attempted.
    DeleteKey {
        /// Identifier of the deleted entry.
        id: Uuid,
    },
    /// Old audit records were compacted to keep the persisted log bounded.
    AuditLogCompacted {
        /// Number of oldest audit records represented by this summary.
        dropped_entries: u64,
    },
}

/// One persisted audit-log record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditLogEntry {
    /// UTC timestamp when the event was recorded.
    pub timestamp: DateTime<Utc>,
    /// Operation that was attempted.
    pub event: AuditEvent,
    /// Whether the attempted operation succeeded.
    pub success: bool,
}

/// On-disk keystore file format.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeystoreFile {
    pub(super) version: u8,
    pub(super) kdf_params: ArgonParams,
    pub(super) master_key_verification: [u8; 32], // HMAC for password verification
    #[serde(default)]
    pub(super) audit_log: Vec<AuditLogEntry>,
    pub(super) entries: Vec<KeyEntry>,
    pub(super) file_integrity_mac: [u8; 32], // HMAC over the entire file contents for integrity
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArgonParams {
    pub(super) salt: [u8; 32],
    pub(super) memory_kb: u32,
    pub(super) iterations: u32,
    pub(super) parallelism: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeystoreHeader {
    pub(super) version: u8,
    pub(super) kdf_params: ArgonParams,
    pub(super) master_key_verification: [u8; 32],
    pub(super) file_integrity_mac: [u8; 32],
}
