use clap::Subcommand;

use super::CommandContext;

/// Delete-key command implementation.
pub mod delete;
/// Import-key command implementation.
pub mod import;
/// List-keys command implementation.
pub mod list;
/// Raw-transaction submission command implementation.
pub mod tx_send_raw;
/// Transaction signing command implementation.
pub mod tx_sign;

/// Subcommands under `mfm keystore`.
#[derive(Subcommand)]
pub enum KeystoreCommand {
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
    /// Sign an EIP-1559 transaction payload using a keystore-managed key
    TxSign {
        /// Parsed arguments for the signing command.
        #[command(flatten)]
        args: tx_sign::TxSignArgs,
    },
    /// Submit a signed raw transaction via JSON-RPC
    TxSendRaw {
        /// Parsed arguments for the raw-transaction submission command.
        #[command(flatten)]
        args: tx_send_raw::TxSendRawArgs,
    },
}

impl KeystoreCommand {
    /// Dispatches the selected keystore subcommand and terminates the process.
    pub async fn execute(&self, ctx: &CommandContext) -> ! {
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
            KeystoreCommand::TxSign { args } => {
                tx_sign::execute(ctx, args).await;
            }
            KeystoreCommand::TxSendRaw { args } => {
                tx_send_raw::execute(ctx, args).await;
            }
        }
    }
}
