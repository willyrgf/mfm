use clap::Parser;

/// Binary entrypoint for the Phase-1 `publish-docs` tool.
#[tokio::main]
async fn main() -> std::process::ExitCode {
    mfm_publish_docs::app::run(mfm_publish_docs::cli::Cli::parse()).await
}
