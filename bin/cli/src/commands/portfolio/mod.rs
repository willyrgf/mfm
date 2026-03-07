use clap::Subcommand;

use super::CommandContext;

mod snapshot;

/// Subcommands under `mfm portfolio`.
#[derive(Subcommand)]
pub enum PortfolioCommand {
    /// Snapshot a wallet portfolio (Ethereum default)
    Snapshot {
        /// Parsed arguments for the snapshot command.
        #[command(flatten)]
        args: snapshot::SnapshotArgs,
    },
}

impl PortfolioCommand {
    /// Dispatches the selected portfolio subcommand and terminates the process.
    pub async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            PortfolioCommand::Snapshot { args } => snapshot::execute(ctx, args).await,
        }
    }
}
