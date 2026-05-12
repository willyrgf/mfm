# Keystore Module Documentation

## Overview

The MFM keystore is a minimal, secure implementation for storing Ethereum private keys. It accepts
BIP39 mnemonics only as one-time import inputs for deriving a selected private key; mnemonic phrases
and BIP39 passphrases are not stored or recoverable from MFM.

## Design Philosophy

### Core Principles
- **Simplicity over feature completeness**: Only essential functionality is implemented
- **Security by default**: Secure configurations are the default, not optional
- **Minimal attack surface**: Fewer features mean fewer potential vulnerabilities
- **Clear separation of concerns**: Each component has a single, well-defined responsibility

### Security Assumptions
- **Full disk encryption** (including swap) is enabled on the system
- **Single-threaded usage** - not designed for concurrent access
- **Local-only operation** - no network features or remote storage
- **Trusted application environment** - assumes the application itself is not compromised

## Architecture

### Key Components

```
keystore/
├── mod.rs          # Main keystore implementation
├── error.rs        # Error types and handling
├── tests.rs        # Comprehensive unit tests
└── KEYSTORE.md     # This documentation
```

### Core Types

- **`Keystore`**: Main struct managing encrypted key storage
- **`KeystoreConfig`**: Configuration with security parameters
- **`KeyInfo`**: Metadata about stored keys (no sensitive data)
- **`SecureKey`**: Zeroizing wrapper for private key material
- **`KeystoreError`**: Comprehensive error handling

## Security Model

### Encryption
- **Key Derivation**: Argon2id with configurable parameters
  - Default: 1GB memory, 8 iterations, 1 parallelism
  - Produces 256-bit master key from password
- **Symmetric Encryption**: AES-256-GCM for key material
  - Unique nonce per entry
  - Authenticated encryption prevents tampering

### Memory Safety
- **Zeroization**: All sensitive data is automatically cleared from memory
- **Secure Types**: `Zeroizing<T>` and `ZeroizeOnDrop` used throughout
- **Limited Scope**: Cryptographic objects have minimal lifetime

### File Format Security
- **JSON Structure**: Human-readable but with encrypted payloads
- **Integrity**: Each entry includes authentication data
- **DoS Protection**: Limits on file size, entry count, and individual entry size

## API Reference

### Basic Usage

```rust
use mfm_core::keystore::{Keystore, KeystoreConfig, KeystoreError};

// Create or open a keystore
let mut keystore = Keystore::new("./wallet.keystore", KeystoreConfig::default())?;

// Unlock with password (required for all operations)
keystore.unlock("secure_password")?;

// Import a private key
let key_id = keystore.import_private_key(
    Some("my-trading-wallet".to_string()),
    "0x1234567890abcdef..."
)?;

// Import a mnemonic-derived key with derivation path. Keep your seed backup outside MFM.
let mnemonic_id = keystore.import_mnemonic(
    Some("hardware-wallet-backup".to_string()),
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    "m/44'/60'/0'/0/0",
    None,
)?;

// List all stored keys
let keys = keystore.list_keys()?;
for key in keys {
    println!("ID: {}, Label: {:?}, Type: {:?}", key.id, key.label, key.key_type);
}

// Retrieve a private key for signing
let private_key = keystore.get_private_key(key_id)?;
let signature = private_key.sign_hash(&hash)?;

// Delete a key
keystore.delete_key(key_id)?;
```

### Configuration

```rust
use mfm_core::keystore::KeystoreConfig;

// Development configuration (faster, less secure)
let dev_config = KeystoreConfig::development();

// Production configuration (slower, more secure)
let prod_config = KeystoreConfig::production();

// Custom configuration
let custom_config = KeystoreConfig {
    argon2_memory_kb: 2097152,  // 2GB
    argon2_iterations: 10,
    argon2_parallelism: 1,
    allow_secret_exports: false,
};
```

### Error Handling

```rust
use mfm_core::keystore::KeystoreError;

match keystore.unlock("wrong_password") {
    Err(KeystoreError::InvalidPassword) => {
        eprintln!("Password is incorrect");
    },
    Err(KeystoreError::Locked) => {
        eprintln!("Keystore is locked, call unlock() first");
    },
    Err(KeystoreError::Io(e)) => {
        eprintln!("File system error: {}", e);
    },
    Ok(_) => println!("Successfully unlocked"),
}
```

## Key Types

### Private Keys
- **Format**: Hexadecimal string (with or without "0x" prefix)
- **Validation**: Must be valid secp256k1 private key
- **Storage**: Encrypted directly as provided

### Mnemonic-Derived Keys
- **Format**: BIP39 mnemonic phrases (12-24 words)
- **Derivation**: BIP32/BIP44 compatible derivation paths
- **Storage**: The selected derived private key is encrypted; the mnemonic phrase and BIP39
  passphrase are discarded before import returns
- **Default Path**: `m/44'/60'/0'/0/0` (Ethereum standard)
- **Backup**: Users must keep their own seed backup outside MFM; MFM cannot export or recover the
  original mnemonic

## File Format

The keystore file is strict JSON format version 3. Unknown fields are rejected at every level.
The file MAC authenticates the canonical file payload, including KDF params, entries, and audit
records, with only the MAC slot zeroed for the MAC preimage.
Every unlocked mutation re-reads the current bounded file body under the mutation lock, validates
its strict shape, recomputes the MAC over that body, and fails closed if the stored MAC, recomputed
MAC, and in-memory MAC do not all match.
The audit log is a bounded recent log. When it reaches the retention limit, the oldest records are
compacted into an explicit `audit_log_compacted` summary entry before the newest event is appended.
Externally oversized audit logs remain invalid and are rejected during authenticated load.
Byte arrays below are shortened for readability.

```json
{
  "version": 3,
  "kdf_params": {
    "salt": [0],
    "memory_kb": 1048576,
    "iterations": 8,
    "parallelism": 1
  },
  "master_key_verification": [0],
  "audit_log": [],
  "entries": [
    {
      "id": "uuid-v4",
      "alias": "optional-human-readable-alias",
      "address": "ethereum-address",
      "key_type": {
        "PrivateKey": null
      },
      "encrypted_data": [0],
      "nonce": [0],
      "created_at": "2024-01-15T10:30:00Z"
    }
  ],
  "file_integrity_mac": [0]
}
```

## Security Considerations

### Threats Mitigated
- **Password attacks**: Argon2id makes brute force expensive
- **Data tampering**: AES-GCM provides authentication
- **Memory dumps**: Zeroization clears sensitive data
- **File system attacks**: Only encrypted data persists

### Threats NOT Mitigated
- **Physical access**: If disk encryption is disabled
- **Application compromise**: Malicious code with same privileges
- **Side-channel attacks**: Not hardened against timing attacks
- **Concurrent access**: No file locking or atomic operations

### Best Practices
1. **Use full disk encryption** (FileVault, BitLocker, LUKS)
2. **Strong passwords**: Passphrase with high entropy
3. **Secure backup**: Store keystore files securely
4. **Limited exposure**: Don't keep keystore unlocked longer than necessary
5. **Regular rotation**: Consider rotating important keys periodically
6. **Diagnostics**: `Debug` output is intentionally redacted and never includes live keys,
   verification MACs, encrypted entry payloads, mnemonics, passphrases, or aliases.

## Performance Characteristics

### Unlock Operation
- **Time**: ~2-8 seconds (depending on Argon2 parameters)
- **Memory**: 1GB+ RAM usage during key derivation
- **CPU**: Significant CPU usage during derivation

### Key Operations (after unlock)
- **Import**: Fast (milliseconds)
- **List**: Fast (no decryption needed)
- **Retrieve**: Fast (single AES-GCM decryption)
- **Delete**: Fast (metadata operation + file write)

### File Size
- **Empty keystore**: ~500 bytes
- **Per private key**: ~300-400 bytes
- **Per mnemonic-derived key**: ~300-400 bytes

## Common Usage Patterns

### Single Key Wallet
```rust
let mut keystore = Keystore::new("./personal.keystore", KeystoreConfig::default())?;
keystore.unlock(&password)?;
let key_id = keystore.import_private_key(Some("main".to_string()), &private_key)?;
```

Configuration files must not point at plaintext private-key files. Import the key into the
keystore first, then pass only the keystore path plus entry id or label through runtime
configuration.

### Mnemonic-Derived Accounts
```rust
let mut keystore = Keystore::new("./hd.keystore", KeystoreConfig::default())?;
keystore.unlock(&password)?;

// Import one selected account from the mnemonic. Keep the seed backup outside MFM.
let mnemonic_id = keystore.import_mnemonic(
    Some("seed".to_string()),
    &mnemonic,
    "m/44'/60'/0'/0/0",
    None,
)?;

// Import additional accounts by providing the mnemonic again for each derivation path.
for account in 0..5 {
    let path = format!("m/44'/60'/0'/0/{}", account);
    let account_id = keystore.import_mnemonic(
        Some(format!("account-{}", account)),
        &mnemonic,
        &path,
        None,
    )?;
}
```

### Key Management Service
```rust
struct KeyManager {
    keystore: Keystore,
}

impl KeyManager {
    pub fn new(path: &str, password: &str) -> Result<Self, KeystoreError> {
        let mut keystore = Keystore::new(path, KeystoreConfig::production())?;
        keystore.unlock(password)?;
        Ok(Self { keystore })
    }
    
    pub fn sign_transaction(&mut self, key_id: Uuid, tx_hash: &[u8]) -> Result<[u8; 65], KeystoreError> {
        let key = self.keystore.get_private_key(key_id)?;
        key.sign_hash(tx_hash)
    }
}
```

## Troubleshooting

### Common Issues

**"InvalidPassword" error**
- Verify password is correct
- Check if keystore file was created with different password
- Ensure file hasn't been corrupted

**"Locked" error**
- Call `keystore.unlock()` before any key operations
- Password may have failed silently - check return value

**"InvalidPrivateKey" error**
- Private key must be valid secp256k1 key
- Check hex encoding (64 characters, valid hex digits)
- Value must be less than secp256k1 curve order

**"InvalidMnemonic" error**
- Must be valid BIP39 mnemonic (12, 15, 18, 21, or 24 words)
- All words must be in BIP39 word list
- Checksum must be valid

**File I/O errors**
- Check file permissions
- Ensure directory exists
- Verify disk space available

### Debugging

Enable debug logging to see internal operations:
```rust
// This will show Argon2 parameters, file operations, etc.
env_logger::init();
```

### Performance Issues

If unlock is too slow:
```rust
let fast_config = KeystoreConfig::development(); // Reduced security for development
```

If memory usage is too high:
```rust
let low_memory_config = KeystoreConfig {
    argon2_memory_kb: 65536,  // 64MB instead of 1GB
    argon2_iterations: 8,
    argon2_parallelism: 1,
    allow_secret_exports: false,
};
```

## Migration and Compatibility

### Version 1 (Current)
- Initial implementation
- AES-256-GCM encryption
- Argon2id key derivation
- JSON file format

### Future Versions
- Will maintain backward compatibility for reading
- New features may require version upgrades
- Migration tools will be provided if breaking changes are needed

## Contributing

When modifying the keystore:

1. **Security Review**: All changes must be security-reviewed
2. **Test Coverage**: Maintain comprehensive test coverage
3. **Memory Safety**: Verify zeroization of sensitive data
4. **Documentation**: Update this document for API changes
5. **Backward Compatibility**: Avoid breaking existing keystores

## References

- [BIP39 - Mnemonic code for generating deterministic keys](https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki)
- [BIP32 - Hierarchical Deterministic Wallets](https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki)
- [BIP44 - Multi-Account Hierarchy for Deterministic Wallets](https://github.com/bitcoin/bips/blob/master/bip-0044.mediawiki)
- [RFC 9106 - Argon2 Memory-Hard Function](https://tools.ietf.org/rfc/rfc9106.txt)
- [AES-GCM - Galois/Counter Mode](https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38d.pdf)
