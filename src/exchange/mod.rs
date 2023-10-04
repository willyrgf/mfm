use crate::config::network::Network;
use crate::config::Config;

pub mod config;

pub mod swap_eth_for_tokens;
pub mod swap_tokens_for_tokens;
use anyhow::Context;

use self::config::ExchangeConfig;

pub struct Exchange {
    config: ExchangeConfig,
    network: Network,
}

impl Exchange {
    pub fn new(config: &ExchangeConfig) -> Self {
        let network = config.get_network();
        Self {
            config: config.clone(),
            network: network.clone(),
        }
    }
}

impl TryFrom<(&Config, &String)> for Exchange {
    type Error = anyhow::Error;
    fn try_from(t: (&Config, &String)) -> Result<Self, Self::Error> {
        let config =
            t.0.exchanges
                .get(t.1)
                .context(format!("exchange '{}' not found", t.1))?;
        Ok(Exchange::new(config))
    }
}
