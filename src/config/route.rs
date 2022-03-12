use crate::config::asset::{Asset, Assets};

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use web3::ethabi::Token;
use web3::types::H160;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    base: String,
    quote: String,
    path: Vec<String>,
    // path_address: Vec<H160>,
}

impl Route {
    pub fn path(&self) -> Vec<String> {
        self.path.clone()
    }
    pub fn build_path(&self, assets: &Assets) -> Vec<H160> {
        let mut v = vec![];

        let base = assets.get(self.base.as_str());
        let quote = assets.get(self.quote.as_str());

        v.push(base.as_address().unwrap());
        self.path.iter().for_each(|p| {
            let a = assets.get(p.as_str());
            v.push(a.as_address().unwrap());
        });
        v.push(quote.as_address().unwrap());

        v
    }
    pub fn build_path_using_tokens(&self, assets: &Assets) -> Token {
        let mut v = vec![];

        let base = assets.get(self.base.as_str());
        let quote = assets.get(self.quote.as_str());

        v.push(Token::Address(base.as_address().unwrap()));
        self.path.iter().for_each(|p| {
            let a = assets.get(p.as_str());
            v.push(Token::Address(a.as_address().unwrap()));
        });
        v.push(Token::Address(quote.as_address().unwrap()));

        Token::Array(v)
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Routes(HashMap<String, Route>);
impl Routes {
    pub fn get(&self, key: &str) -> &Route {
        match self.0.get(key) {
            Some(r) => r,
            None => {
                log::error!("routes.get(): key: {} not found", key);
                panic!()
            }
        }
    }
    pub fn search(&self, base: &Asset, quote: &Asset) -> &Route {
        let key = format!("{}-{}", base.name(), quote.name());
        self.get(key.as_str())
    }
}
