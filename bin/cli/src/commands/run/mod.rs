use clap::Subcommand;

use super::CommandContext;

pub mod artifacts;
pub mod events;
pub mod pipeline;
pub mod resume;
pub mod start;
pub mod status;

#[derive(Subcommand)]
pub enum RunCommand {
    /// Start a run (currently supports built-in ops like `proof`)
    Start {
        #[command(flatten)]
        args: start::StartArgs,
    },
    /// Start a multi-step pipeline run
    Pipeline {
        #[command(subcommand)]
        command: pipeline::PipelineCommand,
    },
    /// Resume an existing run by id
    Resume {
        #[command(flatten)]
        args: resume::ResumeArgs,
    },
    /// Show run status (does not execute states)
    Status {
        #[command(flatten)]
        args: status::StatusArgs,
    },
    /// Print run events
    Events {
        #[command(flatten)]
        args: events::EventsArgs,
    },
    /// Artifact operations
    Artifacts {
        #[command(subcommand)]
        command: artifacts::ArtifactsCommand,
    },
}

impl RunCommand {
    pub async fn execute(&self, ctx: &CommandContext) -> ! {
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
