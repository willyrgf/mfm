use web3::types::U256;

use crate::config::{asset::Asset, rebalancer::Rebalancer};

struct Token {
    asset: Asset,
}

impl Token {}

pub struct State {
    config: Rebalancer,
}
