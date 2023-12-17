use serde::{Deserialize, Serialize};

pub mod authentication;
pub mod dexes;
pub mod network;
pub mod token;

use dexes::Dexes;
use network::Networks;
use token::Tokens;

use self::authentication::Methods;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub networks: Networks,
    pub dexes: Dexes,
    pub tokens: Tokens,
    pub auth_methods: Methods,
}
