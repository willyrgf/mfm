use crate::config::asset::{Asset, Assets};

use std::collections::HashMap;
use std::str::FromStr;

use rustc_hex::FromHexError;
use serde::{Deserialize, Serialize};
use web3::{
    contract::{Contract, Options},
    transports::Http,
    types::{Address, H160, U256},
};

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchange {
    name: String,
    router_address: String,
}

impl Exchange {
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn router_address(&self) -> &str {
        self.router_address.as_str()
    }

    pub fn as_router_address(&self) -> Result<H160, FromHexError> {
        Address::from_str(self.router_address())
    }

    pub fn router_abi_path(&self) -> String {
        format!("./res/exchanges/{}/abi.json", self.name.as_str())
    }

    pub fn router_abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.router_abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn router_contract(&self, client: web3::Web3<Http>) -> Contract<Http> {
        let contract_address = self.as_router_address().unwrap();
        let json_abi = self.router_abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub async fn wrapper_address(&self, client: web3::Web3<Http>) -> H160 {
        let router_contract = self.router_contract(client);

        let wrapped_addr = router_contract
            .query("WETH", (), None, Options::default(), None)
            .await
            .unwrap();

        wrapped_addr
    }

    pub async fn wrapped_asset<'a>(
        &self,
        assets: &'a Assets,
        client: web3::Web3<Http>,
    ) -> &'a Asset {
        let wrapped_address = self.wrapper_address(client).await.to_string();
        let wrapped_asset = assets.find_by_address(wrapped_address.as_str());
        wrapped_asset
    }

    pub async fn wrap(
      &self,
      client: web3::Web3<Http>,
      wrap_asset: &Asset,
      from_address: H160,
      amount: U256,
      gas_price: U256
    ) {
      let estimate_gas = wrap_asset
          .contract(client.clone())
          .estimate_gas(
              "deposit",
              (),
              from_address,
              web3::contract::Options {
                  value: Some(amount),
                  gas_price: Some(gas_price),
                  // gas: Some(500_000.into()),
                  // gas: Some(gas_price),
                  ..Default::default()
              },
          )
          .await
          .unwrap();
      println!("wrap called estimate_gas: {:?}", estimate_gas);
    }

    pub async fn get_amounts_out(
        &self,
        client: web3::Web3<Http>,
        // decimals: u8,
        amount: U256,
        assets_path: Vec<H160>,
    ) -> Vec<U256> {
        let contract = self.router_contract(client);
        // let quantity = 1;
        // let amount: U256 = (quantity * 10_i32.pow(decimals.into())).into();
        let result = contract.query(
            "getAmountsOut",
            (amount, assets_path),
            None,
            Options::default(),
            None,
        );
        let result_amounts_out: Vec<U256> = result.await.unwrap();
        result_amounts_out
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchanges(HashMap<String, Exchange>);
impl Exchanges {
    pub fn get(&self, key: &str) -> &Exchange {
        self.0.get(key).unwrap()
    }
}
