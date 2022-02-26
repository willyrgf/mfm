use clap::Parser;
use ethsign::SecretKey;
use mfm::config::Config;
use std::str::FromStr;
use web3::{contract::Contract, types::Address};

// multiverse finance machine cli
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    // filename with the mfm configurations
    #[clap(short, long, default_value = "config.yaml")]
    config_filename: String,
}

fn main() {
    let args = Args::parse();
    let config = Config::from_file(&args.config_filename);
    // println!("config: {:?}", config);

    let wallet = config.wallets.get("test-wallet");

    let secret = match SecretKey::from_raw(&wallet.to_raw()) {
        Ok(s) => s,
        Err(e) => panic!("invalid secret, err: {}", e),
    };

    let bsc_network = config.networks.get("bsc");
    let anonq_asset = config.assets.get("anonq");

    let abi_path = |name: &str| format!("./config_files/exchanges/{}/abi.json", name);
    // let abi_path = |name: &str| -> &str { format!("../exchanges/{}/abi.json", name).as_str() };

    let pk_exchange = config.exchanges.get(anonq_asset.exchange_id());

    let public = secret.public();
    let http = web3::transports::Http::new(bsc_network.rpc_url()).unwrap();
    let client = web3::Web3::new(http);
    let address = Address::from_str(anonq_asset.pair_address()).unwrap();

    let abi_json = |path: &str| -> String {
        let reader = std::fs::File::open(path).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    };

    let path = abi_path(pk_exchange.name());
    println!("path: {}", path);

    let json = abi_json(path.as_str());

    let contract = Contract::from_json(client.eth(), address, json.as_bytes());
    println!("contract: {:?}", contract);
    // let name = exchange.name();

    // println!("name: {}", abi_path(pk_exchange.name().to_string()));
    // println!(
    //     "include_bytes: {}",
    //     include_bytes!(abi_path(pk_exchange.name()))
    // );

    // let address = Address::from_str(public.address().);
    // let eth = web3::et

    println!(
        "secret: {:?}; public: {:?}; address: {:?}",
        secret,
        public,
        public.address()
    );
    println!("address: {:?}", address);
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
