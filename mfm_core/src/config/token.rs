use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Kind {
    ERC20,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenNetwork {
    pub name: String,
    pub network_id: String,
    pub address: String,
    pub slippage: f64,
    pub path_asset: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenNetworks(HashMap<String, TokenNetwork>);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Token {
    pub kind: Kind,
    pub networks: TokenNetworks,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Tokens(HashMap<String, Token>);
impl Tokens {
    pub fn hashmap(&self) -> &HashMap<String, Token> {
        &self.0
    }

    pub fn get(&self, key: &str) -> Option<&Token> {
        self.0.get(key)
    }
}
