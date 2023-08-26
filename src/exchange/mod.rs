use crate::config::network::Network;

use self::config::ExchangeConfig;

pub mod config;

pub mod swap_eth_for_tokens;
pub mod swap_tokens_for_tokens;

pub struct Exchange {
    config: ExchangeConfig,
    network: Network,
}
