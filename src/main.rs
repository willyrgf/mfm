use clap::Parser;
use mfm::{config::Config, signing};
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use std::str::FromStr;
use web3::{
    contract::{Contract, Options},
    types::{Address, U256},
};

// multiverse finance machine cli
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    // filename with the mfm configurations
    #[clap(short, long, default_value = "config.yaml")]
    config_filename: String,
}

#[tokio::main]
async fn main() -> web3::contract::Result<()> {
    let args = Args::parse();
    let config = Config::from_file(&args.config_filename);

    let wallet = config.wallets.get("test-wallet");

    let secret = SecretKey::from_str(&wallet.private_key()).unwrap();

    let bsc_network = config.networks.get("bsc");
    let anonq_asset = config.assets.get("anonq");

    // TODO: move it to exchange
    let abi_path = |name: &str| format!("./res/exchanges/{}/abi.json", name);
    // let abi_path = |name: &str| -> &str { format!("../exchanges/{}/abi.json", name).as_str() };

    let pk_exchange = config.exchanges.get(anonq_asset.exchange_id());

    let secp = Secp256k1::new();
    let public = PublicKey::from_secret_key(&secp, &secret);

    let http = web3::transports::Http::new(bsc_network.rpc_url()).unwrap();
    let client = web3::Web3::new(http);
    let address = Address::from_str(anonq_asset.address()).unwrap();

    let abi_json = |path: &str| -> String {
        let reader = std::fs::File::open(path).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    };

    let path = abi_path(pk_exchange.name());
    let json = abi_json(path.as_str());

    let contract = Contract::from_json(client.eth(), address, json.as_bytes()).unwrap();
    let account_address = signing::public_key_address(&public);

    let result = contract.query(
        "balanceOf",
        (account_address,),
        None,
        Options::default(),
        None,
    );
    let balance_of: U256 = result.await?;
    println!("balance_of: {:?}", balance_of);

    Ok(())
}

// pub struct Crypto {
//     name: String,
// }

// pub struct Defi {
//     contract_abi: String,
//     // steps: v
// }

// pub enum Handle {
//     Crypto(Crypto),
//     Defi(Defi),
// }

// impl Handle {
//     pub fn next_step() {}
// }
