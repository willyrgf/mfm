use clap::Subcommand;

use super::CommandContext;

/// Run list/watch observation command implementation.
mod list;
/// Manual-resolution recording command implementation.
mod manual_resolution;
/// Typed public-output rendering command implementation.
mod public_output;
/// Typed replay command implementation.
mod replay;
/// Run-resume command implementation.
mod resume;
/// Run-start command implementation.
mod start;
/// Run-status command implementation.
mod status;
/// Run-stream query command implementation.
mod stream;

/// Subcommands under `mfm run`.
#[derive(Subcommand)]
pub(crate) enum RunCommand {
    /// Start a certified typed run
    Start {
        /// Parsed arguments for the start command.
        #[command(flatten)]
        args: start::StartArgs,
    },
    /// List or watch observed runs
    List {
        /// Parsed arguments for the list command.
        #[command(flatten)]
        args: list::ListArgs,
    },
    /// Resume a certified typed run by id
    Resume {
        /// Parsed arguments for the resume command.
        #[command(flatten)]
        args: resume::ResumeArgs,
    },
    /// Show certified typed run status without executing states
    Status {
        /// Parsed arguments for the status command.
        #[command(flatten)]
        args: status::StatusArgs,
    },
    /// Print certified typed run stream records
    Stream {
        /// Parsed arguments for the stream command.
        #[command(flatten)]
        args: stream::StreamArgs,
    },
    /// Render a typed public output by schema id
    PublicOutput {
        /// Parsed arguments for the public-output command.
        #[command(flatten)]
        args: public_output::PublicOutputArgs,
    },
    /// Record a signed manual resolution for a blocked typed run
    ManualResolution {
        /// Parsed arguments for the manual-resolution command.
        #[command(flatten)]
        args: manual_resolution::ManualResolutionArgs,
    },
    /// Verify typed replay authority for a run
    Replay {
        /// Parsed arguments for the replay command.
        #[command(flatten)]
        args: replay::ReplayArgs,
    },
}

impl RunCommand {
    /// Dispatches the selected run subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            RunCommand::Start { args } => start::execute(ctx, args).await,
            RunCommand::List { args } => list::execute(ctx, args).await,
            RunCommand::Resume { args } => resume::execute(ctx, args).await,
            RunCommand::Status { args } => status::execute(ctx, args).await,
            RunCommand::Stream { args } => stream::execute(ctx, args).await,
            RunCommand::PublicOutput { args } => public_output::execute(ctx, args).await,
            RunCommand::ManualResolution { args } => manual_resolution::execute(ctx, args).await,
            RunCommand::Replay { args } => replay::execute(ctx, args).await,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_only_run_commands_use_evidence_only_services() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let run_root = manifest_dir.join("src/commands/run");
        let read_only_commands = [
            "list.rs",
            "status.rs",
            "stream.rs",
            "public_output.rs",
            "replay.rs",
        ];

        for file in read_only_commands {
            let path = run_root.join(file);
            let source = std::fs::read_to_string(&path).expect("read command source");
            assert!(
                source.contains("connect_run_read_services"),
                "{} must construct evidence-only run services",
                path.display()
            );
            assert!(
                !source.contains("connect_run_services"),
                "{} must not construct live run services",
                path.display()
            );
        }
    }
}
