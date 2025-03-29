//! erc20 token adapter
use crate::blockchain::adapter::provider::Provider;
use crate::blockchain::adapter::signer::LocalWallet;
use crate::blockchain::adapter::types::{Address, Bytes, U256};
use alloy_primitives;
use anyhow::{anyhow, Result};
use lazy_static;
use tiny_keccak::{Hasher, Keccak};

/// Compute a function selector from its signature
///
/// Takes a function signature string and computes the first 4 bytes
/// of its keccak256 hash, which is the standard function selector in Ethereum
fn compute_selector(signature: &str) -> [u8; 4] {
    let mut selector = [0u8; 4];
    let mut hasher = Keccak::v256();
    hasher.update(signature.as_bytes());
    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);
    selector.copy_from_slice(&hash[0..4]);
    selector
}

// Define function selectors for ERC20 methods using the actual function signatures
lazy_static::lazy_static! {
    static ref BALANCE_OF_SELECTOR: [u8; 4] = compute_selector("balanceOf(address)");
    static ref TRANSFER_SELECTOR: [u8; 4] = compute_selector("transfer(address,uint256)");
    static ref APPROVE_SELECTOR: [u8; 4] = compute_selector("approve(address,uint256)");
    static ref ALLOWANCE_SELECTOR: [u8; 4] = compute_selector("allowance(address,address)");
    static ref NAME_SELECTOR: [u8; 4] = compute_selector("name()");
    static ref SYMBOL_SELECTOR: [u8; 4] = compute_selector("symbol()");
    static ref DECIMALS_SELECTOR: [u8; 4] = compute_selector("decimals()");
}

/// ERC20 token contract adapter
#[derive(Debug, Clone)]
pub struct ERC20 {
    /// Contract address
    pub address: Address,
    /// Provider to use for queries
    provider: Provider,
}

impl ERC20 {
    /// Create a new ERC20 contract instance
    pub fn new(address: Address, provider: Provider) -> Self {
        Self { address, provider }
    }

    /// Get the name of the token
    pub async fn name(&self) -> Result<String> {
        // Call the contract
        let result = self
            .provider
            .call(self.address, Bytes((*NAME_SELECTOR).to_vec()))
            .await?;

        // decode the result using helper function
        decode_string(&result.0)
    }

    /// Get the symbol of the token
    pub async fn symbol(&self) -> Result<String> {
        // Call the contract
        let result = self
            .provider
            .call(self.address, Bytes((*SYMBOL_SELECTOR).to_vec()))
            .await?;

        // decode the result using helper function
        decode_string(&result.0)
    }

    /// Get the number of decimals the token uses
    pub async fn decimals(&self) -> Result<u8> {
        // Call the contract
        let result = self
            .provider
            .call(self.address, Bytes((*DECIMALS_SELECTOR).to_vec()))
            .await?;

        // Make sure we have enough data
        if result.0.len() < 32 {
            return Err(anyhow!("invalid response length for decimals()"));
        }

        // The uint8 value is in the last byte of the 32-byte word
        Ok(result.0[31])
    }

    /// Get the balance of an account
    pub async fn balance_of(&self, account: Address) -> Result<U256> {
        // Prepare the call data
        let call_data = create_balance_of_call_data(&account);

        // Call the contract
        let result = self.provider.call(self.address, Bytes(call_data)).await?;

        // Decode the result - ERC20 balanceOf() returns a uint256
        decode_uint256(&result.0)
    }

    /// Get a formatted balance (with decimals)
    pub async fn formatted_balance_of(&self, account: Address) -> Result<String> {
        let balance = self.balance_of(account).await?;
        let decimals = self.decimals().await?;

        // Format the balance with proper decimal places
        format_token_amount(balance, decimals)
    }

    /// Transfer tokens to another address
    pub async fn transfer(
        &self,
        to: Address,
        amount: U256,
        wallet: &LocalWallet,
    ) -> Result<String> {
        // Prepare the transfer function call data
        let call_data = create_transfer_call_data(&to, &amount);

        // Send the transaction using the wallet
        wallet
            .send_transaction(
                &self.provider,
                Some(self.address),
                None, // no ETH value
                Bytes(call_data),
            )
            .await
    }

    /// Approve another address to spend tokens
    pub async fn approve(
        &self,
        spender: Address,
        amount: U256,
        wallet: &LocalWallet,
    ) -> Result<String> {
        // Prepare the approve function call data
        let call_data = create_approve_call_data(&spender, &amount);

        // Send the transaction using the wallet
        wallet
            .send_transaction(
                &self.provider,
                Some(self.address),
                None, // no ETH value
                Bytes(call_data),
            )
            .await
    }

    /// Get the allowance for a spender
    pub async fn allowance(&self, owner: Address, spender: Address) -> Result<U256> {
        // Prepare the call data
        let call_data = create_allowance_call_data(&owner, &spender);

        // Call the contract
        let result = self.provider.call(self.address, Bytes(call_data)).await?;

        // Decode the result - ERC20 allowance() returns a uint256
        decode_uint256(&result.0)
    }

    /// Encode a transfer call without executing it
    pub fn encode_transfer(&self, to: Address, amount: U256) -> Result<Bytes> {
        // Create the call data
        let call_data = create_transfer_call_data(&to, &amount);

        // Return as Bytes
        Ok(Bytes(call_data))
    }

    /// Encode an approve call without executing it
    pub fn encode_approve(&self, spender: Address, amount: U256) -> Result<Bytes> {
        let call_data = create_approve_call_data(&spender, &amount);
        Ok(Bytes(call_data))
    }
}

// pure function to create balanceOf call data
fn create_balance_of_call_data(account: &Address) -> Vec<u8> {
    let mut call_data = (*BALANCE_OF_SELECTOR).to_vec();

    // Pad the address to 32 bytes (EVM ABI encoding)
    let mut address_bytes = vec![0u8; 12]; // 12 zeros for padding
    address_bytes.extend_from_slice(account.0.as_ref());
    call_data.extend_from_slice(&address_bytes);

    call_data
}

// pure function to create transfer call data
fn create_transfer_call_data(to: &Address, amount: &U256) -> Vec<u8> {
    let mut call_data = Vec::with_capacity(68); // 4 + 32 + 32 bytes

    // Function selector - dereference the lazy static ref
    call_data.extend_from_slice(&*TRANSFER_SELECTOR);

    // Pad the address to 32 bytes
    let mut to_bytes = vec![0u8; 12]; // 12 zeros for padding
    to_bytes.extend_from_slice(to.0.as_ref());
    call_data.extend_from_slice(&to_bytes);

    // Amount as uint256
    call_data.extend_from_slice(&amount.0.to_be_bytes::<32>());

    call_data
}

// pure function to create approve call data
fn create_approve_call_data(spender: &Address, amount: &U256) -> Vec<u8> {
    let mut call_data = Vec::with_capacity(68); // 4 + 32 + 32 bytes

    // Function selector - dereference the lazy static ref
    call_data.extend_from_slice(&*APPROVE_SELECTOR);

    // Pad the address to 32 bytes
    let mut spender_bytes = vec![0u8; 12]; // 12 zeros for padding
    spender_bytes.extend_from_slice(spender.0.as_ref());
    call_data.extend_from_slice(&spender_bytes);

    // Amount as uint256
    call_data.extend_from_slice(&amount.0.to_be_bytes::<32>());

    call_data
}

// pure function to create allowance call data
fn create_allowance_call_data(owner: &Address, spender: &Address) -> Vec<u8> {
    let mut call_data = (*ALLOWANCE_SELECTOR).to_vec();

    // Pad both addresses to 32 bytes each (EVM ABI encoding)
    let mut owner_bytes = vec![0u8; 12]; // 12 zeros for padding
    owner_bytes.extend_from_slice(owner.0.as_ref());
    call_data.extend_from_slice(&owner_bytes);

    let mut spender_bytes = vec![0u8; 12]; // 12 zeros for padding
    spender_bytes.extend_from_slice(spender.0.as_ref());
    call_data.extend_from_slice(&spender_bytes);

    call_data
}

// pure function to decode a uint256 from EVM ABI encoding
fn decode_uint256(data: &[u8]) -> Result<U256> {
    // Make sure we have enough data
    if data.len() < 32 {
        return Err(anyhow!("invalid response length for uint256 decoding"));
    }

    // Parse the uint256 from the 32 bytes
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&data[0..32]);
    let value = alloy_primitives::U256::from_be_bytes(bytes);

    // Convert to our adapter type
    Ok(U256(value))
}

// pure function to decode a string from EVM ABI encoding
fn decode_string(data: &[u8]) -> Result<String> {
    // The ABI encoding for strings is:
    // - 32 bytes: offset to data (usually 0x20 = 32)
    // - 32 bytes: length of string
    // - data: the actual string data, padded to 32 bytes

    // Make sure we have enough data
    if data.len() < 64 {
        return Err(anyhow!("invalid response length for string decoding"));
    }

    // Parse the length from bytes 32-63
    let length_data = &data[32..64];
    let length = u64::from_be_bytes([
        length_data[0],
        length_data[1],
        length_data[2],
        length_data[3],
        length_data[4],
        length_data[5],
        length_data[6],
        length_data[7],
    ]) as usize;

    // Make sure we have enough data for the string
    if data.len() < 64 + length {
        return Err(anyhow!("invalid response length for string data"));
    }

    // Extract the actual string bytes
    let string_data = &data[64..64 + length];

    // Convert bytes to string
    let string_value = String::from_utf8_lossy(string_data).to_string();

    Ok(string_value)
}

// pure function to format a token amount with proper decimal places
fn format_token_amount(amount: U256, decimals: u8) -> Result<String> {
    let amount_value = amount.0;
    let divisor = alloy_primitives::U256::from(10).pow(alloy_primitives::U256::from(decimals));

    if divisor.is_zero() {
        return Err(anyhow!("cannot format amount: division by zero"));
    }

    // Calculate the whole and fractional parts
    let whole = amount_value / divisor;
    let fractional = amount_value % divisor;

    // Convert the fractional part to a string with leading zeros
    let fractional_str = fractional.to_string();
    let zeros_needed = decimals as usize - fractional_str.len();
    let mut padded_fractional = "0".repeat(zeros_needed);
    padded_fractional.push_str(&fractional_str);

    // Trim trailing zeros
    let trimmed_fractional = padded_fractional.trim_end_matches('0');

    // Format the final string
    let formatted = if trimmed_fractional.is_empty() {
        whole.to_string()
    } else {
        format!("{}.{}", whole, trimmed_fractional)
    };

    Ok(formatted)
}
