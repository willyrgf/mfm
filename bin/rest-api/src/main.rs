use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use mfm_app::{Application, Deployment};

mod server;

const DEFAULT_MAX_IN_FLIGHT_RUNS: usize = 8;
const MAX_MAX_IN_FLIGHT_RUNS: usize = 256;
const DEFAULT_RUN_TIMEOUT_SECONDS: u64 = 120;
const MAX_RUN_TIMEOUT_SECONDS: u64 = 3_600;

#[derive(Parser)]
#[command(name = "mfm_rest_api")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long)]
        deployment: Option<PathBuf>,
        #[arg(long)]
        unix_socket: PathBuf,
        #[arg(
            long,
            default_value_t = DEFAULT_MAX_IN_FLIGHT_RUNS,
            value_parser = parse_max_in_flight_runs
        )]
        max_in_flight_runs: usize,
        #[arg(
            long,
            default_value_t = DEFAULT_RUN_TIMEOUT_SECONDS,
            value_parser = parse_run_timeout_seconds
        )]
        run_timeout: u64,
    },
}

fn parse_max_in_flight_runs(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| (1..=MAX_MAX_IN_FLIGHT_RUNS).contains(value))
        .ok_or_else(|| "max in-flight runs is invalid".to_owned())
}

fn parse_run_timeout_seconds(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| (1..=MAX_RUN_TIMEOUT_SECONDS).contains(value))
        .ok_or_else(|| "run timeout is invalid".to_owned())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<(), MainError> {
    match cli.command {
        Command::Serve {
            deployment,
            unix_socket,
            max_in_flight_runs,
            run_timeout,
        } => {
            let deployment = Deployment::load(deployment.as_deref())
                .await
                .map_err(MainError::Composition)?;
            let application = Arc::new(
                Application::open(&deployment)
                    .await
                    .map_err(MainError::Composition)?,
            );
            let listener =
                tokio::net::UnixListener::bind(&unix_socket).map_err(|_| MainError::Socket)?;
            let router = server::router(
                application,
                max_in_flight_runs,
                Duration::from_secs(run_timeout),
            );
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal())
                .await;
            let cleanup = tokio::fs::remove_file(&unix_socket).await;
            result.map_err(|_| MainError::Serve)?;
            cleanup.map_err(|_| MainError::Socket)
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let terminate = async {
            let mut signal =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("installing the terminate signal handler must succeed");
            signal.recv().await;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error("{0}")]
    Composition(mfm_app::ComposeError),
    #[error("rest socket is invalid or unavailable")]
    Socket,
    #[error("rest server failed")]
    Serve,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_control_bounds_are_checked() {
        assert_eq!(parse_max_in_flight_runs("1"), Ok(1));
        assert_eq!(parse_max_in_flight_runs("256"), Ok(256));
        assert!(parse_max_in_flight_runs("0").is_err());
        assert!(parse_max_in_flight_runs("257").is_err());
        assert_eq!(parse_run_timeout_seconds("1"), Ok(1));
        assert_eq!(parse_run_timeout_seconds("3600"), Ok(3_600));
        assert!(parse_run_timeout_seconds("0").is_err());
        assert!(parse_run_timeout_seconds("3601").is_err());
    }

    #[test]
    fn serve_grammar_requires_a_socket_and_preserves_deployment_presence() {
        let without_override = Cli::try_parse_from([
            "mfm_rest_api",
            "serve",
            "--unix-socket",
            "/private/mfm.sock",
        ])
        .expect("serve grammar");
        let Command::Serve { deployment, .. } = without_override.command;
        assert!(deployment.is_none());

        let with_override = Cli::try_parse_from([
            "mfm_rest_api",
            "serve",
            "--deployment",
            "/operator/deployment.toml",
            "--unix-socket",
            "/private/mfm.sock",
        ])
        .expect("serve grammar");
        let Command::Serve { deployment, .. } = with_override.command;
        assert_eq!(deployment, Some(PathBuf::from("/operator/deployment.toml")));
        assert!(Cli::try_parse_from(["mfm_rest_api", "serve"]).is_err());
    }
}
