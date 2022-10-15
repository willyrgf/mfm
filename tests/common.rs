use clap::{ArgMatches, Command};
use once_cell::sync::Lazy;

use mfm::{
    cmd,
    config::Config,
    telemetry::{get_subscriber, init_subscriber},
};

const APP_NAME: &str = "integration_test";
const DEFAULT_LOG_LEVEL: &str = "debug";
const DEFAULT_CONFIG_FILE: &str = "test_config.yaml";

static TRACING: Lazy<()> = Lazy::new(|| {
    let subscriber = get_subscriber(APP_NAME.into(), DEFAULT_LOG_LEVEL.into(), std::io::stdout);
    init_subscriber(subscriber);
});

pub struct App {
    command: Command,
    config: Config,
}

impl App {
    // TODO: impl all configs as builders here
    pub fn new() -> Self {
        Self::default()
    }

    pub fn command(&self) -> Command {
        self.command.clone()
    }

    pub fn config(&self) -> Config {
        self.config.clone()
    }

    pub fn get_arg_matches(&self, argv: &'static str) -> ArgMatches {
        get_arg_matches(self.command(), argv)
    }
}

impl Default for App {
    fn default() -> Self {
        Lazy::force(&TRACING);

        App {
            command: cmd::new(),
            config: Config::from_file(DEFAULT_CONFIG_FILE).unwrap(),
        }
    }
}

pub fn get_arg_matches(cmd: Command, argv: &'static str) -> ArgMatches {
    cmd.try_get_matches_from(argv.split(' ').collect::<Vec<_>>())
        .unwrap()
}
