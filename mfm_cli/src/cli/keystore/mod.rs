use clap::Subcommand;

use super::OutputFormat;

pub mod delete;
pub mod import;
pub mod list;

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
}

impl KeystoreCommand {
    pub async fn execute(
        &self,
        output_format: &OutputFormat,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            KeystoreCommand::Import { args } => {
                import::execute(args, output_format).await?;
            }
            KeystoreCommand::Delete { args } => {
                delete::execute(args, output_format).await?;
            }
            KeystoreCommand::List { args } => {
                list::execute(args, output_format).await?;
            }
        }
        Ok(())
    }
}
