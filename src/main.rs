use env_logger::Env;
use mfm::{cmd, config::Config};

//TODO: handle with all unwraps
#[tokio::main]
async fn main() -> web3::contract::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();

    let cmd = cmd::new();

    let cmd_matches = cmd.get_matches();
    log::debug!("matches: {:?}", cmd_matches);
    let config = match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    cmd::handle_sub_commands(&cmd_matches, &config).await;

    Ok(())
}
