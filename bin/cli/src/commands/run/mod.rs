use clap::Subcommand;

use super::CommandContext;

/// Artifact inspection subcommands.
mod artifacts;
/// Run-event query command implementation.
mod events;
/// Pipeline-start command implementations.
mod pipeline;
/// Run-resume command implementation.
mod resume;
/// Run-start command implementation.
mod start;
/// Run-status command implementation.
mod status;

/// Subcommands under `mfm run`.
#[derive(Subcommand)]
pub(crate) enum RunCommand {
    /// Start a run (currently supports built-in ops like `proof`)
    Start {
        /// Parsed arguments for the start command.
        #[command(flatten)]
        args: start::StartArgs,
    },
    /// Start a multi-step pipeline run
    Pipeline {
        /// Nested pipeline command to execute.
        #[command(subcommand)]
        command: pipeline::PipelineCommand,
    },
    /// Resume an existing run by id
    Resume {
        /// Parsed arguments for the resume command.
        #[command(flatten)]
        args: resume::ResumeArgs,
    },
    /// Show run status (does not execute states)
    Status {
        /// Parsed arguments for the status command.
        #[command(flatten)]
        args: status::StatusArgs,
    },
    /// Print run events
    Events {
        /// Parsed arguments for the events command.
        #[command(flatten)]
        args: events::EventsArgs,
    },
    /// Artifact operations
    Artifacts {
        /// Nested artifact command to execute.
        #[command(subcommand)]
        command: artifacts::ArtifactsCommand,
    },
}

impl RunCommand {
    /// Dispatches the selected run subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            RunCommand::Start { args } => start::execute(ctx, args).await,
            RunCommand::Pipeline { command } => command.execute(ctx).await,
            RunCommand::Resume { args } => resume::execute(ctx, args).await,
            RunCommand::Status { args } => status::execute(ctx, args).await,
            RunCommand::Events { args } => events::execute(ctx, args).await,
            RunCommand::Artifacts { command } => command.execute(ctx).await,
        }
    }
}
