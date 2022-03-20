#[macro_use]
extern crate prettytable;
use env_logger::Env;
use mfm::cmd;

//TODO: handle with all unwraps
fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let command = cmd::new();
    rt.block_on(cmd::run(command));
}
