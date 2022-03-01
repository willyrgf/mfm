use clap::Parser;
use mfm::{config::Config, signing};
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use std::str::FromStr;

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

    let secp = Secp256k1::new();
    let public = PublicKey::from_secret_key(&secp, &secret);

    let http = web3::transports::Http::new(bsc_network.rpc_url()).unwrap();
    let client = web3::Web3::new(http);
    let account_address = signing::public_key_address(&public);
    // TODO: move it to a async func and let main without async
    for (_, asset) in config.assets.hashmap().iter() {
        let balance_of = asset.balance_of(client.clone(), account_address).await;
        let decimals = asset.decimals(client.clone()).await;
        println!(
            "asset: {}, balance_of: {:?}, decimals: {}",
            asset.name(),
            balance_of,
            decimals
        );

        if asset.name() == "anonq" {
            let decimals = asset.decimals(client.clone()).await;
            let exchange = config.exchanges.get(asset.exchange_id());
            let wbnb = config.assets.get("wbnb");
            let busd = config.assets.get("busd");
            let path = vec![
                asset.as_address().unwrap(),
                wbnb.as_address().unwrap(),
                busd.as_address().unwrap(),
            ];
            let route = config.routes.search(asset, busd);
            println!(
                "route: {:?}; real_path: {:?}",
                route,
                route.build_path(&config.assets)
            );
            println!("path: {:?}", path);

            let result_amounts_out = exchange
                .get_amounts_out(client.clone(), decimals, path)
                .await;

            println!("getAmountsOut: {:?}", result_amounts_out);
        }
    }

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
