use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Erc20,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenNetwork {
    pub name: String,
    pub kind: Kind,
    pub network_id: String,
    pub address: String,
    pub slippage: f64,
    pub path_token: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenNetworks(HashMap<String, TokenNetwork>);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Token {
    // TODO: rethink tokens to be any kind of token, but each
    // chain/network will may have an different token kind.
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
