extern crate prettytable;
use mfm::{
    cmd,
    telemetry::{get_subscriber, init_subscriber},
    APP_NAME, DEFAULT_LOG_LEVEL,
};

fn main() {
    let subscriber = get_subscriber(APP_NAME.into(), DEFAULT_LOG_LEVEL.into(), std::io::stdout);
    init_subscriber(subscriber);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let command = cmd::new();
    rt.block_on(cmd::run(command));
}
