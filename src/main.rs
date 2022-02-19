use clap::Parser;
use mfm::config::Config;

// multiverse finance machine cli
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    // filename with the mfm configurations
    #[clap(short, long)]
    config_filename: String,
}

fn main() {
    let args = Args::parse();
    let config = Config::from_file(&args.config_filename);

    println!("config: {:?}", config);
}
