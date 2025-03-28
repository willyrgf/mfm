//! Adapter module providing blockchain abstractions for the application
//!
//! This module contains adapters for various blockchain components:
//! - Types (Address, U256, Bytes, B256)
//! - Provider for blockchain interactions
//! - Wallet/Signer for transaction signing
//! - Contract and ERC20 interfaces

pub mod contracts;
pub mod erc20;
pub mod provider;
pub mod signer;
pub mod types;

// Re-export common types for easier use
pub use provider::Provider;
pub use signer::LocalWallet;
pub use types::{Address, Bytes, B256, U256};

#[cfg(test)]
mod tests {
    use crate::blockchain::adapter::contracts::Contract;
    use crate::blockchain::adapter::erc20::ERC20;
    use crate::blockchain::adapter::provider::Provider;
    use crate::blockchain::adapter::signer::LocalWallet;
    use crate::blockchain::adapter::types::{Address, U256};
    use std::str::FromStr;

    async fn setup_test_wallet() -> anyhow::Result<(Provider, LocalWallet)> {
        // Use environment variables for these in a real application
        let rpc_url = "https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY";
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001"; // Never use this in production!

        // Connect to provider
        let provider = Provider::connect(rpc_url).await?;

        // Create wallet
        let wallet = LocalWallet::from_private_key(private_key)?;
        let chain_id = provider.get_chainid().await?;
        let wallet = wallet.with_chain_id(chain_id);

        Ok((provider, wallet))
    }

    #[tokio::test]
    #[ignore]
    async fn test_erc20_basics() -> anyhow::Result<()> {
        let (provider, _wallet) = setup_test_wallet().await?;

        // USDC on Ethereum mainnet
        let usdc_address = Address::from_str("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")?;
        let usdc = ERC20::new(usdc_address, provider);

        // Basic token info
        let name = usdc.name().await?;
        let symbol = usdc.symbol().await?;
        let decimals = usdc.decimals().await?;

        println!("Token: {} ({})", name, symbol);
        println!("Decimals: {}", decimals);

        // Check Vitalik's USDC balance
        let vitalik = Address::from_str("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045")?;
        let balance = usdc.balance_of(vitalik).await?;
        let formatted = usdc.formatted_balance_of(vitalik).await?;

        println!("Balance: {} ({})", balance.0, formatted);

        Ok(())
    }

    // This test is disabled by default as it would actually submit a transaction
    // Uncomment and provide valid credentials to test transaction signing
    #[tokio::test]
    #[ignore]
    async fn test_token_transfer() -> anyhow::Result<()> {
        let (provider, wallet) = setup_test_wallet().await?;

        // Use a testnet token instead of mainnet for real tests
        let token_address = Address::from_str("YOUR_TOKEN_ADDRESS")?;
        let token = ERC20::new(token_address, provider);

        // Transfer a small amount
        let recipient = Address::from_str("RECIPIENT_ADDRESS")?;
        let amount = U256::from(1000000); // Adjust based on token decimals

        let tx_hash = token.transfer(recipient, amount, &wallet).await?;
        println!("Transfer tx hash: {}", tx_hash);

        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_generic_contract() -> anyhow::Result<()> {
        let (provider, _wallet) = setup_test_wallet().await?;

        // USDC on Ethereum mainnet
        let usdc_address = Address::from_str("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")?;

        // Create a contract instance
        let mut usdc = Contract::new(usdc_address, provider);

        // Add function selectors for ERC20 methods
        usdc.add_function("balanceOf", &[0x70, 0xa0, 0x82, 0x31])
            .add_function("name", &[0x06, 0xfd, 0xde, 0x03])
            .add_function("symbol", &[0x95, 0xd8, 0x9b, 0x41])
            .add_function("decimals", &[0x31, 0x3c, 0xe5, 0x67]);

        // Call name() function
        let name_result = usdc.call("name", vec![]).await?;

        // Decode name (ABI encoded string)
        if name_result.0.len() < 64 {
            return Err(anyhow::anyhow!("Invalid response length for name()"));
        }

        // Parse the length from bytes 32-63
        let length_data = &name_result.0[32..64];
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

        // Extract the actual string bytes
        let string_data = &name_result.0[64..64 + length];
        let name = String::from_utf8_lossy(string_data).to_string();

        println!("Token name: {}", name);
        assert_eq!(name, "USD Coin");

        // Check Vitalik's USDC balance
        let vitalik = Address::from_str("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045")?;

        // Prepare balanceOf parameters (address padded to 32 bytes)
        let mut address_bytes = vec![0u8; 12]; // 12 zeros for padding
        address_bytes.extend_from_slice(vitalik.0.as_ref());

        let balance_result = usdc.call("balanceOf", address_bytes).await?;

        // Decode the balance (uint256)
        if balance_result.0.len() < 32 {
            return Err(anyhow::anyhow!("Invalid response length for balanceOf()"));
        }

        // Parse the uint256 from the 32 bytes
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&balance_result.0[0..32]);
        let balance = alloy_primitives::U256::from_be_bytes(bytes);

        println!("Balance: {}", balance);

        Ok(())
    }
}
