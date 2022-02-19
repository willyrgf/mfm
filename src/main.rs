use clap::Parser;
use ethsign::SecretKey;
use mfm::config::Config;

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
    println!("config: {:?}", config);

    let wallet = config.wallets.get("test-wallet");

    let secret = match SecretKey::from_raw(&wallet.to_raw()) {
        Ok(s) => s,
        Err(e) => panic!("invalid secret, err: {}", e),
    };

    let public = secret.public();

    println!(
        "secret: {:?}; public: {:?}; address: {:?}",
        secret,
        public,
        public.address()
    );
}
