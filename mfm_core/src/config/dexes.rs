use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Kind {
    UniswapV2,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Dex {
    pub name: String,
    pub kind: Kind,
    pub router_address: String,
    pub factory_address: String,
    pub network_id: String,
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
