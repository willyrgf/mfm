use clap::Subcommand;

use super::CommandContext;

mod dcv;

/// Subcommands under `mfm evm`.
#[derive(Subcommand)]
pub(crate) enum EvmCommand {
    /// EVM deploy/configure/validate workflows
    Dcv {
        /// Nested EVM DCV command to execute.
        #[command(subcommand)]
        command: dcv::DcvCommand,
    },
}

impl EvmCommand {
    /// Dispatches the selected EVM subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            EvmCommand::Dcv { command } => command.execute(ctx).await,
        }
    }
}
