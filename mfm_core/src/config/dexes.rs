use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    UniswapV2,
    CowSwap,
    UniswapV3,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Dex {
    pub name: String,
    pub kind: Kind,
    pub router_address: Option<String>,
    pub factory_address: Option<String>,
    pub network_id: String,
    pub settlement_contract: Option<String>,
    pub api_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Dexes(HashMap<String, Dex>);
impl Dexes {
    pub fn hashmap(&self) -> &HashMap<String, Dex> {
        &self.0
    }
    pub fn get(&self, key: &str) -> Option<&Dex> {
        self.0.get(key)
    }
}
