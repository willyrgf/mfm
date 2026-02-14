use clap::Parser;

mod commands;
mod presentation;
mod support;

use commands::Cli;
use mfm_app::observability::{
    init_observability, observability_from_env, ENV_MFM_LOG, ENV_RUST_LOG,
};

#[tokio::main]
async fn main() -> ! {
    let mut observability = observability_from_env("mfm_cli");
    if std::env::var(ENV_MFM_LOG).is_err() && std::env::var(ENV_RUST_LOG).is_err() {
        // Keep CLI stderr JSON contracts stable by default while preserving CLI-originated warnings
        // (for example insecure password-env usage).
        observability.filter =
            "warn,mfm_machine=error,mfm_app=error,mfm_op_keystore_tx=error,tower_http=error"
                .to_string();
    }

    if let Err(err) = init_observability(observability) {
        eprintln!("failed to initialize observability: {}", err.message);
        std::process::exit(1);
    }

    let cli = Cli::parse();
    cli.execute().await;
}
