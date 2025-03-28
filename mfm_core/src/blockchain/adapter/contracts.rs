//! contract adapter for alloy
use crate::blockchain::adapter::provider::Provider;
use crate::blockchain::adapter::signer::LocalWallet;
use crate::blockchain::adapter::types::{Address, Bytes, U256};
use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// Generic contract adapter
#[derive(Debug, Clone)]
pub struct Contract {
    /// Contract address
    pub address: Address,
    /// Provider for blockchain interactions
    pub provider: Provider,
    /// Function selectors map (name -> selector)
    function_selectors: HashMap<String, Vec<u8>>,
}

impl Contract {
    /// Create a new contract
    pub fn new(address: Address, provider: Provider) -> Self {
        Self {
            address,
            provider,
            function_selectors: HashMap::new(),
        }
    }

    /// Add a function selector
    pub fn add_function(&mut self, name: &str, selector: &[u8]) -> &mut Self {
        self.function_selectors
            .insert(name.to_string(), selector.to_vec());
        self
    }

    /// Get the contract address
    pub fn address(&self) -> Address {
        self.address
    }

    /// Call a contract function (read-only)
    pub async fn call(&self, function_name: &str, args: Vec<u8>) -> Result<Bytes> {
        // Get call data by combining selector and arguments
        let call_data = self.build_call_data(function_name, &args)?;

        // Call the contract
        self.provider.call(self.address, Bytes(call_data)).await
    }

    /// Call a contract function with parameters (read-only)
    pub async fn call_with_params(&self, function_name: &str, params: &[Bytes]) -> Result<Bytes> {
        // Get call data by combining selector and parameters
        let call_data = self.build_params_call_data(function_name, params)?;

        // Call the contract
        self.provider.call(self.address, Bytes(call_data)).await
    }

    /// Send a transaction to the contract (state-changing)
    pub async fn send(
        &self,
        function_name: &str,
        args: Vec<u8>,
        wallet: &LocalWallet,
        value: Option<U256>,
    ) -> Result<String> {
        // Get call data by combining selector and arguments
        let call_data = self.build_call_data(function_name, &args)?;

        // Create and send the transaction
        wallet
            .send_transaction(&self.provider, Some(self.address), value, Bytes(call_data))
            .await
    }

    /// Send a transaction with parameters (state-changing)
    pub async fn send_with_params(
        &self,
        function_name: &str,
        params: &[Bytes],
        wallet: &LocalWallet,
        value: Option<U256>,
    ) -> Result<String> {
        // Get call data by combining selector and parameters
        let call_data = self.build_params_call_data(function_name, params)?;

        // Create and send the transaction
        wallet
            .send_transaction(&self.provider, Some(self.address), value, Bytes(call_data))
            .await
    }

    /// Encode a function call
    pub fn encode_function_data(&self, function_name: &str, args: Vec<u8>) -> Result<Bytes> {
        // Get call data by combining selector and arguments
        let call_data = self.build_call_data(function_name, &args)?;
        Ok(Bytes(call_data))
    }

    /// Encode a function call with parameters
    pub fn encode_with_params(&self, function_name: &str, params: &[Bytes]) -> Result<Bytes> {
        // Get call data by combining selector and parameters
        let call_data = self.build_params_call_data(function_name, params)?;
        Ok(Bytes(call_data))
    }

    /// Build call data by combining selector and arguments
    fn build_call_data(&self, function_name: &str, args: &[u8]) -> Result<Vec<u8>> {
        // Get the function selector
        let selector = self
            .function_selectors
            .get(function_name)
            .ok_or_else(|| anyhow!("function selector not found for {}", function_name))?;

        // Create call data
        let mut call_data = selector.clone();
        call_data.extend_from_slice(args);

        Ok(call_data)
    }

    /// Build call data by combining selector and parameters
    fn build_params_call_data(&self, function_name: &str, params: &[Bytes]) -> Result<Vec<u8>> {
        // Get the function selector
        let selector = self
            .function_selectors
            .get(function_name)
            .ok_or_else(|| anyhow!("function selector not found for {}", function_name))?;

        // Create call data
        let mut call_data = selector.clone();

        // Add each parameter
        for param in params {
            call_data.extend_from_slice(&param.0);
        }

        Ok(call_data)
    }

    /// Get the provider
    pub fn provider(&self) -> &Provider {
        &self.provider
    }
}
