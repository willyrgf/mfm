use clap::Subcommand;

use super::CommandContext;

/// EVM contract lifecycle command implementation.
mod contracts;

/// Subcommands under `mfm evm`.
#[derive(Subcommand)]
pub(crate) enum EvmCommand {
    /// EVM contract lifecycle workflows
    Contracts {
        /// Nested contract lifecycle command to execute.
        #[command(subcommand)]
        command: contracts::ContractsCommand,
    },
}

impl EvmCommand {
    /// Dispatches the selected EVM subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            EvmCommand::Contracts { command } => command.execute(ctx).await,
        }
    }
}
