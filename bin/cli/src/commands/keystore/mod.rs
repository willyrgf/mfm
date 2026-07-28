use clap::Subcommand;

use super::CommandContext;

/// Delete-key command implementation.
mod delete;
/// Import-key command implementation.
mod import;
/// List-keys command implementation.
mod list;

/// Subcommands under `mfm keystore`.
#[derive(Subcommand)]
pub(crate) enum KeystoreCommand {
    /// Import a private key or mnemonic
    Import {
        /// Parsed arguments for the import command.
        #[command(flatten)]
        args: import::ImportArgs,
    },
    /// Delete a key from the keystore
    Delete {
        /// Parsed arguments for the delete command.
        #[command(flatten)]
        args: delete::DeleteArgs,
    },
    /// List keys in the keystore
    List {
        /// Parsed arguments for the list command.
        #[command(flatten)]
        args: list::ListArgs,
    },
}

impl KeystoreCommand {
    /// Dispatches the selected keystore subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            KeystoreCommand::Import { args } => {
                import::execute(ctx, args).await;
            }
            KeystoreCommand::Delete { args } => {
                delete::execute(ctx, args).await;
            }
            KeystoreCommand::List { args } => {
                list::execute(ctx, args).await;
            }
        }
    }
}
