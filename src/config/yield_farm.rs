use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldFarm {
    wallet_id: String,
    address: String,
    operation: String,
}

impl YieldFarm {
    pub fn address(&self) -> String {
        self.address.clone()
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldFarms(HashMap<String, YieldFarm>);

impl YieldFarms {
    pub fn get(&self, key: &str) -> &YieldFarm {
        self.0.get(key).unwrap()
    }
}
