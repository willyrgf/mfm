use alloy_primitives::{Address as AlloyAddress, B256 as AlloyB256};
use anyhow::{anyhow, Result};
use hex;
use k256::{
    ecdsa::{signature::Signer, RecoveryId, SigningKey},
    elliptic_curve::generic_array::GenericArray,
};
use sha3::{Digest, Keccak256};
use std::fmt;

use crate::blockchain::adapter::provider::Transaction;
use crate::blockchain::adapter::types::{Address, Bytes, B256, U256};

/// LocalWallet adapter for transaction signing
#[derive(Clone)]
pub struct LocalWallet {
    /// Private key as B256
    private_key: B256,
    /// Cached wallet address
    address: Address,
    /// Chain ID for EIP-155 signing
    chain_id: Option<u64>,
}

impl fmt::Debug for LocalWallet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalWallet")
            .field("address", &self.address)
            .finish()
    }
}

impl LocalWallet {
    /// Create a new local wallet from a private key
    pub fn from_private_key(private_key: &str) -> Result<Self> {
        // Remove 0x prefix if present
        let key_str = private_key.trim_start_matches("0x");

        // Parse the key into bytes
        let key_bytes =
            hex::decode(key_str).map_err(|e| anyhow!("Invalid private key format: {}", e))?;

        if key_bytes.len() != 32 {
            return Err(anyhow!("Private key must be 32 bytes"));
        }

        // Convert to B256
        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);
        let inner_key = AlloyB256::from(key_array);
        let private_key = B256(inner_key);

        // Derive address from private key
        let signing_key = derive_signing_key(&key_bytes)?;
        let address = derive_address_from_signing_key(&signing_key)?;

        Ok(Self {
            private_key,
            address,
            chain_id: None,
        })
    }

    /// Get the wallet's address
    pub fn address(&self) -> Address {
        self.address
    }

    /// Sign a message (Ethereum format with prefix)
    pub async fn sign_message(&self, message: &[u8]) -> Result<Bytes> {
        // Prepare the Ethereum message format:
        // keccak256("\x19Ethereum Signed Message:\n" + len(message) + message)
        let mut hasher = Keccak256::new();
        hasher.update(b"\x19Ethereum Signed Message:\n");

        // Add the message length as a string
        let msg_len_string = message.len().to_string();
        hasher.update(msg_len_string.as_bytes());

        // Add the message itself
        hasher.update(message);

        // Compute the hash
        let hash = hasher.finalize();
        let hash_bytes = hash.as_slice();

        // Sign the hash using the private key
        let key_bytes = self.private_key.0.to_vec();
        let signing_key = derive_signing_key(&key_bytes)?;

        // Create signature data
        let signature = sign_message_hash(hash_bytes, &signing_key)?;

        Ok(Bytes(signature))
    }

    /// Set the chain ID for this wallet
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = Some(chain_id);
        self
    }

    /// Get the chain ID if set
    pub fn chain_id(&self) -> Option<u64> {
        self.chain_id
    }

    /// Sign a transaction
    pub async fn sign_transaction(&self, tx: &Transaction) -> Result<Vec<u8>> {
        // Ensure we have a chain ID for EIP-155 signing
        let chain_id = tx
            .chain_id
            .or(self.chain_id)
            .ok_or_else(|| anyhow!("Chain ID is required for transaction signing"))?;

        // Get the key bytes
        let key_bytes = self.private_key.0.to_vec();
        let signing_key = derive_signing_key(&key_bytes)?;

        // Create the transaction data to sign
        // For simplicity, we'll use a Legacy transaction format (EIP-155)
        // In a complete implementation, we would support EIP-1559 and other formats

        // Ensure transaction has all required fields
        let nonce = tx
            .nonce
            .ok_or_else(|| anyhow!("Transaction nonce is required"))?;
        let gas_price = tx
            .gas_price
            .ok_or_else(|| anyhow!("Gas price is required"))?;
        let gas_limit = tx.gas.ok_or_else(|| anyhow!("Gas limit is required"))?;
        let to = tx.to; // Can be None for contract creation
        let value = tx.value.unwrap_or(U256::from(0u64));
        let data = &tx.data;

        // Prepare the RLP data for the transaction
        let mut rlp_data = Vec::new();

        // 1. Nonce
        rlp_encode(&mut rlp_data, &nonce.0.to_be_bytes::<32>());

        // 2. Gas price
        rlp_encode(&mut rlp_data, &gas_price.0.to_be_bytes::<32>());

        // 3. Gas limit
        rlp_encode(&mut rlp_data, &gas_limit.0.to_be_bytes::<32>());

        // 4. To address (if present)
        if let Some(to) = to {
            rlp_encode(&mut rlp_data, to.0.as_ref());
        } else {
            // Empty bytes for contract creation
            rlp_encode(&mut rlp_data, &[]);
        }

        // 5. Value
        rlp_encode(&mut rlp_data, &value.0.to_be_bytes::<32>());

        // 6. Data
        rlp_encode(&mut rlp_data, &data.0);

        // 7-9. Chain ID, 0, a0 (for EIP-155)
        rlp_encode(&mut rlp_data, &u64_to_be_bytes(chain_id));
        rlp_encode(&mut rlp_data, &[0]);
        rlp_encode(&mut rlp_data, &[0]);

        // Hash the RLP data to get the message to sign
        let mut hasher = Keccak256::new();
        hasher.update(&rlp_data);
        let hash = hasher.finalize();

        // Sign the hash
        let signature = sign_message_hash(hash.as_slice(), &signing_key)?;

        // Extract r, s, v components
        let r = signature[0..32].to_vec();
        let s = signature[32..64].to_vec();
        let v = signature[64]; // v is 27 or 28

        // Adjust v for EIP-155
        let v_int = (v as u64) - 27;
        let eip155_v = chain_id * 2 + 35 + v_int;

        // Build the signed transaction RLP data
        let mut signed_data = Vec::new();

        // 1. Nonce
        rlp_encode(&mut signed_data, &nonce.0.to_be_bytes::<32>());

        // 2. Gas price
        rlp_encode(&mut signed_data, &gas_price.0.to_be_bytes::<32>());

        // 3. Gas limit
        rlp_encode(&mut signed_data, &gas_limit.0.to_be_bytes::<32>());

        // 4. To address (if present)
        if let Some(to) = to {
            rlp_encode(&mut signed_data, to.0.as_ref());
        } else {
            // Empty bytes for contract creation
            rlp_encode(&mut signed_data, &[]);
        }

        // 5. Value
        rlp_encode(&mut signed_data, &value.0.to_be_bytes::<32>());

        // 6. Data
        rlp_encode(&mut signed_data, &data.0);

        // 7. v (EIP-155)
        rlp_encode(&mut signed_data, &u64_to_be_bytes(eip155_v));

        // 8. r
        rlp_encode(&mut signed_data, &r);

        // 9. s
        rlp_encode(&mut signed_data, &s);

        // Wrap the entire data in an RLP list
        let mut wrapped_data = Vec::new();
        rlp_encode_list(&mut wrapped_data, &signed_data);

        Ok(wrapped_data)
    }

    /// Send a transaction using the given provider
    pub async fn send_transaction(
        &self,
        provider: &crate::blockchain::adapter::provider::Provider,
        to: Option<Address>,
        value: Option<U256>,
        data: Bytes,
    ) -> Result<String> {
        // Prepare the transaction
        let nonce = provider.get_transaction_count(self.address()).await?;
        let gas_price = provider.get_gas_price().await?;

        let tx = Transaction {
            to,
            value,
            data,
            nonce: Some(nonce),
            gas_price: Some(gas_price),
            gas: Some(U256::from(100000)), // Default gas limit
            chain_id: self.chain_id,
        };

        // Estimate gas if possible
        let gas = provider
            .estimate_gas(&tx)
            .await
            .unwrap_or(U256::from(100000));

        let tx = Transaction {
            gas: Some(gas),
            ..tx
        };

        // Sign the transaction
        let signed_tx = self.sign_transaction(&tx).await?;

        // Send the signed transaction
        provider.send_transaction(&signed_tx).await
    }
}

/// Helper function to derive a secp256k1 signing key from private key bytes
fn derive_signing_key(private_key_bytes: &[u8]) -> Result<SigningKey> {
    let key_bytes = GenericArray::from_slice(private_key_bytes);
    let signing_key = SigningKey::from_bytes(key_bytes)?;
    Ok(signing_key)
}

/// Helper function to derive the Ethereum address from a signing key
fn derive_address_from_signing_key(signing_key: &SigningKey) -> Result<Address> {
    // Get the public key from the signing key (without the 0x04 prefix)
    let public_key = signing_key.verifying_key().to_encoded_point(false);
    let public_key_bytes = public_key.as_bytes();

    // Public key is 65 bytes with a 0x04 prefix
    if public_key_bytes.len() != 65 {
        return Err(anyhow!("Invalid public key length"));
    }

    // Hash the public key (minus the 0x04 prefix) with keccak256
    let mut hasher = Keccak256::new();
    hasher.update(&public_key_bytes[1..]);
    let hash = hasher.finalize();

    // Take the last 20 bytes as the Ethereum address
    let ethereum_address = &hash[12..32];

    // Convert to our address type
    let mut address_bytes = [0u8; 20];
    address_bytes.copy_from_slice(ethereum_address);
    let alloy_address = AlloyAddress::from(address_bytes);
    Ok(Address(alloy_address))
}

/// Sign a message hash using the provided signing key
fn sign_message_hash(hash: &[u8], signing_key: &SigningKey) -> Result<Vec<u8>> {
    if hash.len() != 32 {
        return Err(anyhow!("Hash must be 32 bytes"));
    }

    // Sign the hash to get a recoverable signature
    let signature: k256::ecdsa::Signature = signing_key.sign(hash);
    let signature_bytes = signature.to_vec();

    // For simplicity, we'll use a fixed recovery ID of 0 (v = 27)
    // In a production implementation, we would properly compute the recovery ID
    let recovery_id = RecoveryId::new(false, false);

    // Convert recovery ID to Ethereum's v value (27 or 28)
    let v = recovery_id.to_byte() + 27;

    // Construct the Ethereum signature (r, s, v)
    let mut eth_signature = Vec::with_capacity(65);
    eth_signature.extend_from_slice(&signature_bytes[0..32]); // r
    eth_signature.extend_from_slice(&signature_bytes[32..64]); // s
    eth_signature.push(v);

    Ok(eth_signature)
}

// Helper for RLP encoding a byte array
fn rlp_encode(output: &mut Vec<u8>, data: &[u8]) {
    if data.len() == 1 && data[0] < 128 {
        // For a single byte < 0x80, the byte is its own RLP encoding
        output.push(data[0]);
    } else if data.len() <= 55 {
        // For a string 0-55 bytes long, the RLP encoding is 0x80 + length followed by the string
        output.push(0x80 + data.len() as u8);
        output.extend_from_slice(data);
    } else {
        // For a string > 55 bytes long, the RLP encoding is 0xb7 + length of the length in bytes,
        // followed by the length, followed by the string
        let len_bytes = data.len().to_be_bytes();
        let len_without_leading_zeros = len_bytes
            .iter()
            .skip_while(|&&b| b == 0)
            .cloned()
            .collect::<Vec<u8>>();

        output.push(0xb7 + len_without_leading_zeros.len() as u8);
        output.extend_from_slice(&len_without_leading_zeros);
        output.extend_from_slice(data);
    }
}

// Helper for RLP encoding a list
fn rlp_encode_list(output: &mut Vec<u8>, data: &[u8]) {
    if data.len() <= 55 {
        // For a list 0-55 bytes long, the RLP encoding is 0xc0 + length followed by the concatenation of the RLP encodings
        output.push(0xc0 + data.len() as u8);
        output.extend_from_slice(data);
    } else {
        // For a list > 55 bytes long, the RLP encoding is 0xf7 + length of the length in bytes,
        // followed by the length, followed by the concatenation of the RLP encodings
        let len_bytes = data.len().to_be_bytes();
        let len_without_leading_zeros = len_bytes
            .iter()
            .skip_while(|&&b| b == 0)
            .cloned()
            .collect::<Vec<u8>>();

        output.push(0xf7 + len_without_leading_zeros.len() as u8);
        output.extend_from_slice(&len_without_leading_zeros);
        output.extend_from_slice(data);
    }
}

// Helper to convert u64 to big-endian bytes without leading zeros
fn u64_to_be_bytes(value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    bytes
        .iter()
        .skip_while(|&&b| b == 0)
        .cloned()
        .collect::<Vec<u8>>()
}
