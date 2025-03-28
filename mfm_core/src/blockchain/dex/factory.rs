//! Factory for creating DEX providers from configuration
use crate::blockchain::adapter::{LocalWallet, Provider};
use crate::blockchain::config::{Config, DexType};
use crate::blockchain::cow_swap::CowSwapProvider;
use crate::blockchain::dex::{DexError, DexProvider};
use std::sync::Arc;

/// Factory for creating DEX providers
pub struct DexFactory;

impl DexFactory {
    /// Create a DEX provider from configuration
    pub fn create_dex_provider(
        config: &Config,
        provider: Arc<Provider>,
        wallet: Option<LocalWallet>,
    ) -> Result<Box<dyn DexProvider>, DexError> {
        match config.dex.dex_type {
            DexType::CowSwap => {
                let cow_swap = CowSwapProvider::from_config(provider, wallet, config)?;
                Ok(Box::new(cow_swap))
            }
            // Add other DEX types here as they are implemented
            DexType::UniswapV2 => Err(DexError::Other("UniswapV2 not implemented yet".into())),
            DexType::UniswapV3 => Err(DexError::Other("UniswapV3 not implemented yet".into())),
            DexType::SushiSwap => Err(DexError::Other("SushiSwap not implemented yet".into())),
        }
    }
}
