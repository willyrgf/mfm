use core::panic;

use mfm::{cmd, config::Config};

//TODO: handle with all unwraps
#[tokio::main]
async fn main() -> web3::contract::Result<()> {
    let cmd = cmd::new();

    let cmd_matches = cmd.get_matches();
    println!("matches: {:?}", cmd_matches);
    let config = match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    cmd::handle_sub_commands(&cmd_matches, &config).await;

    Ok(())
}
