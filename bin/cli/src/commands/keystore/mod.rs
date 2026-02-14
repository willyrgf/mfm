use clap::Subcommand;

use super::CommandContext;

pub mod delete;
pub mod import;
pub mod list;
mod run_op;
pub mod tx_send_raw;
pub mod tx_sign;

#[derive(Subcommand)]
pub enum KeystoreCommand {
    /// Import a private key or mnemonic
    Import {
        #[command(flatten)]
        args: import::ImportArgs,
    },
    /// Delete a key from the keystore
    Delete {
        #[command(flatten)]
        args: delete::DeleteArgs,
    },
    /// List keys in the keystore
    List {
        #[command(flatten)]
        args: list::ListArgs,
    },
    /// Sign an EIP-1559 transaction payload using a keystore-managed key
    TxSign {
        #[command(flatten)]
        args: tx_sign::TxSignArgs,
    },
    /// Submit a signed raw transaction via JSON-RPC
    TxSendRaw {
        #[command(flatten)]
        args: tx_send_raw::TxSendRawArgs,
    },
}

impl KeystoreCommand {
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
