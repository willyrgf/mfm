use clap::Subcommand;

use super::CommandContext;

mod snapshot;

#[derive(Subcommand)]
pub enum PortfolioCommand {
    /// Snapshot a wallet portfolio (Ethereum default)
    Snapshot {
        #[command(flatten)]
        args: snapshot::SnapshotArgs,
    },
}

impl PortfolioCommand {
    pub async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            PortfolioCommand::Snapshot { args } => snapshot::execute(ctx, args).await,
        }
    }
}
